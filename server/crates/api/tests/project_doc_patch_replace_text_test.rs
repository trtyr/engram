//! doc_patch mode=replace_text（EN-226）回归——EN-10 静默数据丢失修复（2026-10-01）：
//! content 缺失/空必须显式报错且不落库；唯一命中替换后回读逐字一致；
//! 失配 anchor（零命中/多命中）显式报错，原文不丢。

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use support::{app, create_key, login_token, mcp_initialize, mcp_rpc, rpc};
use tower::util::ServiceExt;

struct Ctx {
    app: axum::Router,
    token: String,
    /// 测试库守卫：随 Ctx 活到测试结束（提前 drop 会中途 FORCE 删库）
    _pg: support::TestPg,
}

impl Ctx {
    async fn new() -> Self {
        let (app, _pg) = app().await;
        let token = login_token(&app).await;
        Self { app, token, _pg }
    }

    async fn req(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {}", self.token))
            .header("content-type", "application/json");
        let req = match body {
            Some(b) => builder.body(Body::from(b.to_string())).unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: Value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, v)
    }

    /// 建项目 + 建文档，返回 (project_id, doc_id)
    async fn setup_doc(&self, proj: &str, title: &str, content: &str) -> (String, String) {
        let (st, p) = self
            .req(
                "POST",
                "/projects",
                Some(json!({ "name": proj, "type": "dev" })),
            )
            .await;
        assert!(st.is_success(), "{p}");
        let pid = p["id"].as_str().unwrap().to_string();
        let (st, d) = self
            .req(
                "POST",
                &format!("/projects/{pid}/docs"),
                Some(json!({ "category": "规划", "title": title, "content": content })),
            )
            .await;
        assert!(st.is_success(), "{d}");
        (pid, d["id"].as_str().unwrap().to_string())
    }

    /// 回读文档：返回 (content, version)
    async fn read_doc(&self, pid: &str, doc_id: &str) -> (String, i64) {
        let (st, cur) = self
            .req("GET", &format!("/projects/{pid}/docs/{doc_id}"), None)
            .await;
        assert_eq!(st, StatusCode::OK, "{cur}");
        (
            cur["content"].as_str().unwrap().to_string(),
            cur["version"].as_i64().unwrap(),
        )
    }
}

fn mcp_call(action: &str, args: Value) -> Value {
    let mut arguments = serde_json::Map::new();
    arguments.insert("action".into(), json!(action));
    if let Value::Object(m) = args {
        for (k, v) in m {
            arguments.insert(k, v);
        }
    }
    rpc(
        2,
        "tools/call",
        json!({ "name": "projects", "arguments": arguments }),
    )
}

/// 解 MCP 成功回执：result.content[0].text 是 ok_json 的 pretty JSON
fn result_body(v: &Value) -> Value {
    let text = v["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("MCP 成功回执应有 content[0].text：{v}"));
    serde_json::from_str(text).expect("content[0].text 应为 JSON")
}

const ORIG: &str = "alpha 行\nbeta 唯一片段行\ngamma 行";

#[tokio::test]
async fn replace_text_happy_path_readback_matches() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["projects"]).await;
    mcp_initialize(&ctx.app, &key).await;
    let (pid, doc_id) = ctx.setup_doc("p-rt-ok", "rt 正常路径", ORIG).await;

    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        mcp_call(
            "doc_patch",
            json!({
                "doc_id": doc_id,
                "mode": "replace_text",
                "anchor": "beta 唯一片段行",
                "content": "beta 已替换行",
                "expected_version": 1
            }),
        ),
    )
    .await;
    assert!(v.get("error").is_none(), "replace_text 不应报错：{v}");
    let body = result_body(&v);
    assert_eq!(body["patched"]["mode"], "replace_text", "{body}");
    assert_eq!(
        body["patched"]["old_chars"],
        json!("beta 唯一片段行".chars().count()),
        "{body}"
    );
    assert_eq!(
        body["patched"]["new_chars"],
        json!("beta 已替换行".chars().count()),
        "{body}"
    );

    // EN-10 教训：不能只看回执——doc_get 回读逐字比对
    let (content, version) = ctx.read_doc(&pid, &doc_id).await;
    assert_eq!(
        content, "alpha 行\nbeta 已替换行\ngamma 行",
        "回读应与预期逐字一致"
    );
    assert_eq!(version, 2, "成功 patch 应 bump version");
}

#[tokio::test]
async fn replace_text_missing_content_rejected_no_write() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["projects"]).await;
    mcp_initialize(&ctx.app, &key).await;
    let (pid, doc_id) = ctx.setup_doc("p-rt-none", "rt 缺 content", ORIG).await;

    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        mcp_call(
            "doc_patch",
            json!({
                "doc_id": doc_id,
                "mode": "replace_text",
                "anchor": "beta 唯一片段行"
                // 故意不传 content——EN-10：旧实现静默删原文且回报成功
            }),
        ),
    )
    .await;
    assert!(v.get("error").is_some(), "缺 content 必须显式报错：{v}");
    let msg = v["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("需要 content"), "错误应指明缺 content：{msg}");

    // 不落库：原文与 version 均不变
    let (content, version) = ctx.read_doc(&pid, &doc_id).await;
    assert_eq!(content, ORIG, "被拒 patch 不应改动原文");
    assert_eq!(version, 1, "被拒 patch 不应 bump version");
}

#[tokio::test]
async fn replace_text_empty_content_rejected_no_write() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["projects"]).await;
    mcp_initialize(&ctx.app, &key).await;
    let (pid, doc_id) = ctx.setup_doc("p-rt-empty", "rt 空 content", ORIG).await;

    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        mcp_call(
            "doc_patch",
            json!({
                "doc_id": doc_id,
                "mode": "replace_text",
                "anchor": "beta 唯一片段行",
                "content": ""
            }),
        ),
    )
    .await;
    assert!(v.get("error").is_some(), "空 content 必须显式报错：{v}");
    let msg = v["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("需要 content"), "错误应指明缺 content：{msg}");

    let (content, version) = ctx.read_doc(&pid, &doc_id).await;
    assert_eq!(content, ORIG, "被拒 patch 不应改动原文");
    assert_eq!(version, 1, "被拒 patch 不应 bump version");
}

#[tokio::test]
async fn replace_text_anchor_zero_hit_rejected() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["projects"]).await;
    mcp_initialize(&ctx.app, &key).await;
    let (pid, doc_id) = ctx.setup_doc("p-rt-zero", "rt 零命中", ORIG).await;

    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        mcp_call(
            "doc_patch",
            json!({
                "doc_id": doc_id,
                "mode": "replace_text",
                "anchor": "根本不存在的句子",
                "content": "新文"
            }),
        ),
    )
    .await;
    assert!(v.get("error").is_some(), "零命中必须显式报错：{v}");
    let msg = v["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("零命中"), "错误应指明零命中：{msg}");

    let (content, version) = ctx.read_doc(&pid, &doc_id).await;
    assert_eq!(content, ORIG, "被拒 patch 不应改动原文");
    assert_eq!(version, 1, "被拒 patch 不应 bump version");
}

#[tokio::test]
async fn replace_text_anchor_multi_hit_rejected() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["projects"]).await;
    mcp_initialize(&ctx.app, &key).await;
    let multi = "dup 片段在此\n中间行\ndup 片段在此";
    let (pid, doc_id) = ctx.setup_doc("p-rt-multi", "rt 多命中", multi).await;

    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        mcp_call(
            "doc_patch",
            json!({
                "doc_id": doc_id,
                "mode": "replace_text",
                "anchor": "dup 片段在此",
                "content": "新文"
            }),
        ),
    )
    .await;
    assert!(v.get("error").is_some(), "多命中必须显式报错：{v}");
    let msg = v["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("命中 2 处"), "错误应指明命中处数：{msg}");

    let (content, version) = ctx.read_doc(&pid, &doc_id).await;
    assert_eq!(content, multi, "被拒 patch 不应改动原文");
    assert_eq!(version, 1, "被拒 patch 不应 bump version");
}

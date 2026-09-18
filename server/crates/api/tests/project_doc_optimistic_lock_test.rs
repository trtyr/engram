//! 项目文档乐观锁（公网多Agent P001 步骤2，2026-09-17）：
//! 陈旧写入（expected_version 不匹配）被 409 拒，先写内容不丢；
//! 无 expected_version 时行为向后兼容。

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
}

#[tokio::test]
async fn doc_optimistic_lock_conflicts_on_stale_write() {
    let ctx = Ctx::new().await;

    // 建项目 + 建文档
    let (st, proj) = ctx
        .req(
            "POST",
            "/projects",
            Some(json!({"name": "p-lock", "type": "dev"})),
        )
        .await;
    assert!(st.is_success(), "{proj}");
    let pid = proj["id"].as_str().unwrap();
    let (st, doc) = ctx
        .req(
            "POST",
            &format!("/projects/{pid}/docs"),
            Some(json!({"category": "规划", "title": "锁测试", "content": "第1版"})),
        )
        .await;
    assert!(st.is_success(), "{doc}");
    let doc_id = doc["id"].as_str().unwrap();
    assert_eq!(doc["version"], 1, "新文档 version 应为 1：{doc}");

    // 写手 A：基于 version 1 更新 → 成功，version=2
    let (st, u1) = ctx
        .req(
            "PUT",
            &format!("/projects/{pid}/docs/{doc_id}"),
            Some(json!({
                "category": "规划", "folder": "", "title": "锁测试",
                "content": "第2版（写手A）", "expected_version": 1
            })),
        )
        .await;
    assert!(st.is_success(), "{u1}");
    assert_eq!(u1["version"], 2, "{u1}");

    // 写手 B（陈旧）：仍基于 version 1 → 409，内容不丢
    let (st, u2) = ctx
        .req(
            "PUT",
            &format!("/projects/{pid}/docs/{doc_id}"),
            Some(json!({
                "category": "规划", "folder": "", "title": "锁测试",
                "content": "第2版（写手B 陈旧）", "expected_version": 1
            })),
        )
        .await;
    assert_eq!(st, StatusCode::CONFLICT, "{u2}");

    let (st, cur) = ctx
        .req("GET", &format!("/projects/{pid}/docs/{doc_id}"), None)
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        cur["content"], "第2版（写手A）",
        "陈旧写入被拒后内容不应丢：{cur}"
    );
    assert_eq!(cur["version"], 2, "{cur}");

    // 无 expected_version → 向后兼容，成功
    let (st, u3) = ctx
        .req(
            "PUT",
            &format!("/projects/{pid}/docs/{doc_id}"),
            Some(json!({
                "category": "规划", "folder": "", "title": "锁测试",
                "content": "第3版（无锁模式）"
            })),
        )
        .await;
    assert!(st.is_success(), "{u3}");
    assert_eq!(u3["version"], 3, "{u3}");
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
        json!({"name": "projects", "arguments": arguments}),
    )
}

/// MCP doc_patch 陈旧写入：expected_version 不匹配 → 版本冲突报错，先写内容不丢
#[tokio::test]
async fn mcp_doc_patch_stale_conflicts() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["projects"]).await;
    mcp_initialize(&ctx.app, &key).await;

    let (st, proj) = ctx
        .req(
            "POST",
            "/projects",
            Some(json!({"name": "p-lock-mcp", "type": "dev"})),
        )
        .await;
    assert!(st.is_success(), "{proj}");
    let pid = proj["id"].as_str().unwrap();
    let (st, doc) = ctx
        .req(
            "POST",
            &format!("/projects/{pid}/docs"),
            Some(json!({"category": "规划", "title": "锁测试", "content": "第1行\n第2行"})),
        )
        .await;
    assert!(st.is_success(), "{doc}");
    let doc_id = doc["id"].as_str().unwrap();

    // 写手 A：doc_update expected_version=1 → 成功
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        mcp_call(
            "doc_update",
            json!({"doc_id": doc_id, "content": "第1行改\n第2行", "expected_version": 1}),
        ),
    )
    .await;
    assert!(v.get("error").is_none(), "doc_update 不应报错：{v}");

    // 写手 B（陈旧）：doc_patch 仍基于 version 1 → 版本冲突
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        mcp_call(
            "doc_patch",
            json!({
                "doc_id": doc_id, "start_line": 1, "end_line": 1,
                "mode": "replace", "content": "陈旧补丁", "expected_version": 1
            }),
        ),
    )
    .await;
    assert!(v.get("error").is_some(), "陈旧 patch 应报错：{v}");
    let msg = v["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("版本冲突"), "错误应含版本冲突：{msg}");

    // 内容不丢：仍是写手 A 的内容
    let (st, cur) = ctx
        .req("GET", &format!("/projects/{pid}/docs/{doc_id}"), None)
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        cur["content"], "第1行改\n第2行",
        "陈旧 patch 被拒后内容不应丢：{cur}"
    );
}

//! doc_get 长文档无损回归（EN-12，2026-10-01）：
//! EN-12 报告「长文档（>5000 字）doc_get 返回 content_omitted=true 纯元数据」——
//! 本用例按工单复现规格（6303 字级）造长文档，验证 MCP doc_get 与 HTTP GET
//! 双通道均全文无损返回、无 omitted 标记。若本用例绿，则工单描述行为在当前
//! 代码不存在（疑似与 project_get 索引模式混淆），凭此关单。

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

fn result_body(v: &Value) -> Value {
    let text = v["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("MCP 成功回执应有 content[0].text：{v}"));
    serde_json::from_str(text).expect("content[0].text 应为 JSON")
}

/// EN-12 回归：长文档（>5000 字，工单复现规格 6303 字级）doc_get 全文无损
#[tokio::test]
async fn doc_get_long_content_untruncated_both_channels() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["projects"]).await;
    mcp_initialize(&ctx.app, &key).await;

    // 造 7000 字符长文档（每行 100 字符 × 70 行，含可定位的首尾哨兵）
    let mut long = String::from("SENTINEL_HEAD 首哨兵\n");
    for i in 0..69 {
        long.push_str(&format!("第{i:03}行：{}", "长文档内容样本".repeat(14)));
        long.push('\n');
    }
    long.push_str("SENTINEL_TAIL 尾哨兵");
    let long_chars = long.chars().count();
    assert!(long_chars > 5000, "测试前提：应超工单报告的 5000 字阈值");

    let (st, p) = ctx
        .req(
            "POST",
            "/projects",
            Some(json!({ "name": "p-longdoc", "type": "dev" })),
        )
        .await;
    assert!(st.is_success(), "{p}");
    let pid = p["id"].as_str().unwrap().to_string();
    let (st, d) = ctx
        .req(
            "POST",
            &format!("/projects/{pid}/docs"),
            Some(json!({ "category": "规划", "title": "长文档", "content": long })),
        )
        .await;
    assert!(st.is_success(), "{d}");
    let doc_id = d["id"].as_str().unwrap().to_string();

    // ① MCP doc_get（无区间=全文路径）：必须含全文、首尾哨兵俱在、无 content_omitted
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        mcp_call("doc_get", json!({ "doc_id": doc_id })),
    )
    .await;
    assert!(v.get("error").is_none(), "doc_get 不应报错：{v}");
    let body = result_body(&v);
    assert_ne!(
        body.get("content_omitted"),
        Some(&json!(true)),
        "doc_get 不得带 content_omitted：{}",
        serde_json::to_string(&body).unwrap()
    );
    let mcp_content = body["content"].as_str().expect("doc_get 应有 content 字段");
    assert_eq!(
        mcp_content.chars().count(),
        long_chars,
        "MCP doc_get 全文字符数应无损：{} vs {long_chars}",
        mcp_content.chars().count()
    );
    assert!(mcp_content.starts_with("SENTINEL_HEAD"), "首哨兵缺失");
    assert!(mcp_content.ends_with("SENTINEL_TAIL 尾哨兵"), "尾哨兵缺失");

    // ② HTTP GET 同文档：同样全文无损
    let (st, cur) = ctx
        .req("GET", &format!("/projects/{pid}/docs/{doc_id}"), None)
        .await;
    assert_eq!(st, StatusCode::OK, "{cur}");
    assert_ne!(
        cur.get("content_omitted"),
        Some(&json!(true)),
        "HTTP GET 不得带 content_omitted：{cur}"
    );
    let http_content = cur["content"].as_str().expect("HTTP GET 应有 content 字段");
    assert_eq!(
        http_content.chars().count(),
        long_chars,
        "HTTP GET 全文字符数应无损"
    );
    assert!(http_content.ends_with("SENTINEL_TAIL 尾哨兵"), "尾哨兵缺失");
}

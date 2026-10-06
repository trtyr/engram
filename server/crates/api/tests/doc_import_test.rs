//! doc_import（P017 后补充方式二）：批量导入现成文档——HTTP 端点 + MCP action 双面回归。

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

/// HTTP 批量导入：3 篇进（1 篇标题撞车）→ imported=2 skipped=1。
#[tokio::test]
async fn http_doc_import_batch_with_skip() {
    let ctx = Ctx::new().await;
    let (st, proj) = ctx
        .req(
            "POST",
            "/projects",
            Some(json!({"name": "p-import", "type": "dev", "categories": ["架构"]})),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "建项目: {proj}");
    let pid = proj["id"].as_str().unwrap();
    // 分类走 update（CreateProjectRequest 无 categories 字段）
    let (st, _) = ctx
        .req(
            "PUT",
            &format!("/projects/{pid}"),
            Some(json!({"name": "p-import", "status": "active", "categories": ["架构"]})),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "补分类: {st}");

    // 预置一篇，制造标题撞车
    let (st, _) = ctx
        .req(
            "POST",
            &format!("/projects/{pid}/docs"),
            Some(json!({"category": "架构", "title": "总览", "content": "已有版本。"})),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED);

    let (st, v) = ctx
        .req(
            "POST",
            &format!("/projects/{pid}/docs/import"),
            Some(json!({"docs": [
                {"category": "架构", "title": "总览", "content": "撞车版本——应 skip"},
                {"category": "架构", "title": "数据层", "content": "# 数据层\n\nPG。"},
                {"category": "不存在的分类", "title": "迷路", "content": "分类校验应 skip"}
            ]})),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "导入应 200: {v}");
    assert_eq!(v["imported"], 1, "应导入 1 篇: {v}");
    assert_eq!(v["skipped"], 2, "撞车+坏分类应 skip 2: {v}");
    assert_eq!(v["docs"][0]["title"], "数据层");
}

/// MCP doc_import：action 可达 + 结构断言。
#[tokio::test]
async fn mcp_doc_import_reachable() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["project"]).await;
    mcp_initialize(&ctx.app, &key).await;

    let (st, proj) = ctx
        .req(
            "POST",
            "/projects",
            Some(json!({"name": "p-import-mcp", "type": "dev", "categories": ["架构"]})),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED);
    let pid = proj["id"].as_str().unwrap();
    let (st, _) = ctx
        .req(
            "PUT",
            &format!("/projects/{pid}"),
            Some(json!({"name": "p-import-mcp", "status": "active", "categories": ["架构"]})),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "补分类: {st}");

    let call = |docs: Value| {
        rpc(
            2,
            "tools/call",
            json!({
                "name": "projects",
                "arguments": {
                    "action": "doc_import",
                    "project_id": pid,
                    "docs": docs,
                }
            }),
        )
    };
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        call(json!([
            {"category": "架构", "title": "决策记录", "content": "# 决策\n\n单库终局。"}
        ])),
    )
    .await;
    assert!(v.get("error").is_none(), "doc_import 应可达: {v}");
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
    let parsed: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    assert_eq!(parsed["imported"], 1, "应导入 1 篇: {text}");

    // 空数组应报参数错
    let (_, v) = mcp_rpc(&ctx.app, &key, call(json!([]))).await;
    assert!(v.get("error").is_some(), "空 docs 应报参数错: {v}");
}

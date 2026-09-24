//! 动作级权限（公网多Agent P001 步骤3）：key scope 的 `:ro` 只读变体只能调域内读类动作。
//! 写类 action 被拒且报错列出可用只读动作（报错即文档）；全量 scope 回归无伤。

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use support::{app, create_key, login_token, mcp_call_json, mcp_initialize, mcp_rpc, rpc};
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

    /// 指定 key 的 MCP 请求（不复用 admin token）
    async fn req_with_key(
        &self,
        key: &str,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {key}"))
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
        json!({"name": "projects", "arguments": arguments}),
    )
}

/// :ro key：写类 action 被拒且报错可行动（列出可用只读动作）；读类放行
#[tokio::test]
async fn readonly_key_write_rejected_with_readable_list() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["project:ro"]).await;
    mcp_initialize(&ctx.app, &key).await;

    // admin 建项目与文档（与 key 权限无关）
    let (st, proj) = ctx
        .req(
            "POST",
            "/projects",
            Some(json!({"name": "p-act", "type": "dev"})),
        )
        .await;
    assert!(st.is_success(), "{proj}");
    let pid = proj["id"].as_str().unwrap();
    let (st, doc) = ctx
        .req(
            "POST",
            &format!("/projects/{pid}/docs"),
            Some(json!({"category": "规划", "title": "只读基线", "content": "第1行"})),
        )
        .await;
    assert!(st.is_success(), "{doc}");
    let doc_id = doc["id"].as_str().unwrap();

    // 写类 doc_add → 拒绝，报错含变体名与可用只读动作清单
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        mcp_call(
            "doc_add",
            json!({"project_name": "p-act", "category": "规划", "title": "越权写", "content": "不应落库"}),
        ),
    )
    .await;
    assert!(v.get("error").is_some(), ":ro key 调 doc_add 应被拒：{v}");
    let msg = v["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("权限不足"), "报错应说明权限不足：{msg}");
    assert!(
        msg.contains(":ro") && msg.contains("project"),
        "报错应含 scope 变体名：{msg}"
    );
    assert!(
        msg.contains("可用只读动作") && msg.contains("doc_search"),
        "报错应列出可用只读动作：{msg}"
    );

    // 越权写确实没落库：项目详情的文档索引里不应出现「越权写」
    let (st, detail) = ctx.req("GET", &format!("/projects/{pid}"), None).await;
    assert!(st.is_success(), "{detail}");
    assert!(
        !detail.to_string().contains("越权写"),
        "越权写入不应落库：{detail}"
    );

    // 读类 doc_get → 放行
    let out = mcp_call_json(
        &ctx.app,
        &key,
        "projects",
        json!({"action": "doc_get", "doc_id": doc_id}),
    )
    .await;
    assert_eq!(out["title"], "只读基线", "{out}");

    // HTTP 侧读放行/写拒绝（RJ-20/A2 统一后：与 MCP 同语义——读端点放行 :ro，写端点仍要求全量）
    let (st, list) = ctx.req_with_key(&key, "GET", "/projects", None).await;
    assert!(st.is_success(), ":ro key 打 HTTP 读端点应放行：{st} {list}");
    let (st, werr) = ctx
        .req_with_key(
            &key,
            "POST",
            "/projects",
            Some(json!({"name": "p-ro-x", "type": "dev"})),
        )
        .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        ":ro key 打 HTTP 写端点应 403：{werr}"
    );
}

/// 全量 scope 回归：写类动作不受影响（含 projects 高频误写归一）
#[tokio::test]
async fn full_scope_write_still_ok() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["projects"]).await; // 归一为 project
    mcp_initialize(&ctx.app, &key).await;

    let (st, proj) = ctx
        .req(
            "POST",
            "/projects",
            Some(json!({"name": "p-act-full", "type": "dev"})),
        )
        .await;
    assert!(st.is_success(), "{proj}");

    // 写类 doc_add → 放行
    let out = mcp_call_json(
        &ctx.app,
        &key,
        "projects",
        json!({"action": "doc_add", "project_name": "p-act-full", "category": "规划", "title": "全量写", "content": "正常写入"}),
    )
    .await;
    let doc_id = out["id"].as_str().expect("doc_add 应返回 id");

    // 读类 doc_get → 放行（写读闭环）
    let out = mcp_call_json(
        &ctx.app,
        &key,
        "projects",
        json!({"action": "doc_get", "doc_id": doc_id}),
    )
    .await;
    assert_eq!(out["title"], "全量写", "{out}");
}

/// RJ-13 实测：缺 scopes 字段 = serde 拒绝 422（旧注释写 400 已修正——行为本就如此）
#[tokio::test]
async fn create_key_missing_scopes_is_422() {
    let (app, _pg) = support::app().await;
    let token = login_token(&app).await;
    let req = Request::builder()
        .method("POST")
        .uri("/settings/api-keys")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"name":"noscope"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "缺 scopes 字段应为 422（serde 拒绝）"
    );
}

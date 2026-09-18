//! todos/tickets 拆域（2026-09-18）：待办域锁 kind=todo、工单域锁 kind=ticket，互不可见。
//! todos.add 显式开工单被拒（报错指引走 tickets 域）；tickets.add/update 工单生命周期可用。

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use support::{app, login_token, mcp_call_json, mcp_rpc, rpc};
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
async fn tickets_domain_split_locks_kinds() {
    let ctx = Ctx::new().await;

    // 1) tickets.add 开单（参数无 kind——域锁定自动 ticket）
    let ticket = mcp_call_json(
        &ctx.app,
        &ctx.token,
        "tickets",
        json!({"action": "add", "title": "拆域验收工单", "severity": "P2",
               "symptom": "todos 域看不到工单", "acceptance": "两边互不可见"}),
    )
    .await;
    assert_eq!(
        ticket["kind"], "ticket",
        "tickets.add 应固定 kind=ticket：{ticket}"
    );
    assert_eq!(ticket["severity"], "P2");
    let ticket_id = ticket["id"].as_str().unwrap().to_string();

    // 2) todos.add 显式 kind=ticket → 被拒且指引走 tickets 域
    let (_, v) = mcp_rpc(
        &ctx.app,
        &ctx.token,
        rpc(
            2,
            "tools/call",
            json!({"name": "todos", "arguments": {"action": "add", "title": "伪装工单", "kind": "ticket"}}),
        ),
    )
    .await;
    let err_msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(
        err_msg.contains("tickets"),
        "todos.add kind=ticket 应被拒并指引 tickets 域：{v}"
    );

    // 3) todos.add 正常待办
    let todo = mcp_call_json(
        &ctx.app,
        &ctx.token,
        "todos",
        json!({"action": "add", "title": "纯待办事项"}),
    )
    .await;
    assert_eq!(todo["kind"], "todo", "todos.add 应固定 kind=todo：{todo}");
    let todo_id = todo["id"].as_str().unwrap().to_string();
    assert_ne!(todo_id, ticket_id);

    // 4) 两域列表互不可见：todos.list 无工单、tickets.list 无待办
    let todos = mcp_call_json(&ctx.app, &ctx.token, "todos", json!({"action": "list"})).await;
    let todo_ids: Vec<&str> = todos["items"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x["id"].as_str()).collect())
        .unwrap_or_default();
    assert!(
        !todo_ids.contains(&ticket_id.as_str()),
        "todos.list 不应包含工单：{todos}"
    );
    let tickets = mcp_call_json(&ctx.app, &ctx.token, "tickets", json!({"action": "list"})).await;
    let ticket_ids: Vec<&str> = tickets["items"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x["id"].as_str()).collect())
        .unwrap_or_default();
    assert!(
        ticket_ids.contains(&ticket_id.as_str()),
        "tickets.list 应包含刚开的工单：{tickets}"
    );
    assert!(
        !ticket_ids.contains(&todo_id.as_str()),
        "tickets.list 不应包含待办：{tickets}"
    );

    // 5) tickets.update 状态流转（confirmed → resolved 带 resolution）
    let upd = mcp_call_json(
        &ctx.app,
        &ctx.token,
        "tickets",
        json!({"action": "update", "id": ticket_id, "status": "resolved", "resolution": "拆域落地，验收通过"}),
    )
    .await;
    assert_eq!(upd["status"], "resolved", "工单状态流转：{upd}");
}

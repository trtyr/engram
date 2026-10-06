//! todos/tickets 拆域 + 项目绑定制（0074）：
//! tickets.add 必须绑定已有项目（解析不到拒绝、绝不自动建）；todos 无 kind 形态；
//! 两域列表互不可见；tickets.update 工单生命周期可用。

mod support;

use serde_json::json;
use support::{app, login_token, mcp_call_json, mcp_rpc, rpc};
use tower::ServiceExt;

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
}

#[tokio::test]
async fn tickets_domain_project_binding() {
    let ctx = Ctx::new().await;

    // 1) tickets.add 无 project → 拒绝（绑定不了就不进工单）
    let (_, v) = mcp_rpc(
        &ctx.app,
        &ctx.token,
        rpc(
            2,
            "tools/call",
            json!({"name": "tickets", "arguments": {"action": "add", "title": "无主工单"}}),
        ),
    )
    .await;
    let err_msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(err_msg.contains("project"), "无 project 应被拒：{v}");

    // 2) tickets.add 幽灵项目名 → 拒绝且绝不自动建
    let (_, v) = mcp_rpc(
        &ctx.app,
        &ctx.token,
        rpc(
            2,
            "tools/call",
            json!({"name": "tickets", "arguments": {"action": "add", "title": "幽灵工单",
                   "project": "不存在的项目XYZ"}}),
        ),
    )
    .await;
    let err_msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(err_msg.contains("不存在"), "幽灵项目名应被拒并明说：{v}");

    // 3) 先建项目（HTTP），再 tickets.add 绑定之（精确项目名）
    let created = axum::body::to_bytes(
        ctx.app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/projects")
                    .header("authorization", format!("Bearer {}", ctx.token))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        json!({"name": "绑定测试项目", "type": "dev", "description": ""})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
            .into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let proj: serde_json::Value = serde_json::from_slice(&created).unwrap();
    let pid = proj["id"].as_str().unwrap().to_string();

    let ticket = mcp_call_json(
        &ctx.app,
        &ctx.token,
        "tickets",
        json!({"action": "add", "title": "绑定验收工单", "project": "绑定测试项目",
               "severity": "P2", "symptom": "项目绑定制验收", "acceptance": "建单成功"}),
    )
    .await;
    assert_eq!(
        ticket["project_id"].as_str().unwrap(),
        pid,
        "应解析项目名并绑定：{ticket}"
    );
    assert_eq!(ticket["severity"], "P2");
    let ticket_id = ticket["id"].as_str().unwrap().to_string();

    // 4) tickets.add 幽灵 project_id（UUID 不在库）→ 拒绝
    let ghost = uuid::Uuid::now_v7();
    let (_, v) = mcp_rpc(
        &ctx.app,
        &ctx.token,
        rpc(
            2,
            "tools/call",
            json!({"name": "tickets", "arguments": {"action": "add", "title": "幽灵 id 工单",
                   "project": ghost.to_string()}}),
        ),
    )
    .await;
    let err_msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(err_msg.contains("不存在"), "幽灵 project id 应被拒：{v}");

    // 5) 两域隔离：todos.add 正常待办；两域列表互不可见
    let todo = mcp_call_json(
        &ctx.app,
        &ctx.token,
        "todos",
        json!({"action": "add", "title": "纯待办事项"}),
    )
    .await;
    let todo_id = todo["id"].as_str().unwrap().to_string();
    assert_ne!(todo_id, ticket_id);

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

    // 6) tickets.update 状态流转（confirmed → resolved 带 resolution）
    let upd = mcp_call_json(
        &ctx.app,
        &ctx.token,
        "tickets",
        json!({"action": "update", "id": ticket_id, "status": "resolved", "resolution": "项目绑定制落地，验收通过"}),
    )
    .await;
    assert_eq!(upd["status"], "resolved", "工单状态流转：{upd}");
}

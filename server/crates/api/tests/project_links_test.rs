//! 项目关联集成测试（project_links，0058）：建/查/解绑 + 自环与重复拒绝 + 两向合并。
//!
//! 语义：`part_of` = from 隶属 to（子 → 母）；`related` = 相关（无向语义，存一行）。
//! 一层隶属足以表达「大项目 → 子工作线」，不做无限深树（《项目与资产模型 · README》§2.3）。

mod support;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use support::{app, create_key, login_token};
use tower::util::ServiceExt;

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    let body = match body {
        Some(b) => {
            builder = builder.header("content-type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let resp = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}

async fn make_project(app: &Router, token: &str, name: &str) -> String {
    let (st, v) = send(
        app,
        "POST",
        "/projects",
        token,
        Some(json!({"name": name, "type": "dev"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    v["id"].as_str().unwrap().to_string()
}

/// 建 part_of / related、自环与重复被拒、两向合并、解绑——全链走 HTTP。
#[tokio::test]
async fn project_links_flow() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;
    let mother = make_project(&app, &admin, "关联-母项目").await;
    let child = make_project(&app, &admin, "关联-子工作线").await;
    let peer = make_project(&app, &admin, "关联-旁支").await;

    // ① 建 part_of（子 → 母）
    let (st, link) = send(
        &app,
        "POST",
        &format!("/projects/{child}/links"),
        &admin,
        Some(json!({"to_project": mother, "kind": "part_of", "note": "POC 属于大项目"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{link}");
    assert_eq!(link["kind"], "part_of");
    assert_eq!(link["from_name"], "关联-子工作线");
    assert_eq!(link["to_name"], "关联-母项目");
    let link_id = link["id"].as_str().unwrap().to_string();

    // ② 同向同类重复 → 409
    let (st, v) = send(
        &app,
        "POST",
        &format!("/projects/{child}/links"),
        &admin,
        Some(json!({"to_project": mother, "kind": "part_of"})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");

    // ③ 自环 → 400
    let (st, v) = send(
        &app,
        "POST",
        &format!("/projects/{child}/links"),
        &admin,
        Some(json!({"to_project": child, "kind": "part_of"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");

    // ④ 未知 kind → 400（文案列值域）
    let (st, v) = send(
        &app,
        "POST",
        &format!("/projects/{child}/links"),
        &admin,
        Some(json!({"to_project": peer, "kind": "bogus"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(v["error"]["message"].as_str().unwrap().contains("part_of"));

    // ⑤ 不存在的项目 → 404
    let (st, _) = send(
        &app,
        "POST",
        &format!("/projects/{child}/links"),
        &admin,
        Some(json!({"to_project": uuid::Uuid::now_v7().to_string(), "kind": "related"})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // ⑥ related（无向语义也存一行）
    let (st, _) = send(
        &app,
        "POST",
        &format!("/projects/{child}/links"),
        &admin,
        Some(json!({"to_project": peer, "kind": "related", "note": "同源代码"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // ⑦ 两向合并：子看得到隶属，母也看得到同一条（前端据此分「隶属 / 下属」）
    let (_, child_links) = send(
        &app,
        "GET",
        &format!("/projects/{child}/links"),
        &admin,
        None,
    )
    .await;
    assert_eq!(child_links.as_array().unwrap().len(), 2, "{child_links}");
    let (_, mother_links) = send(
        &app,
        "GET",
        &format!("/projects/{mother}/links"),
        &admin,
        None,
    )
    .await;
    let m = mother_links.as_array().unwrap();
    assert_eq!(m.len(), 1, "母项目应看到子项目的隶属边：{mother_links}");
    assert_eq!(m[0]["from_name"], "关联-子工作线");
    assert_eq!(m[0]["to_name"], "关联-母项目");

    // ⑧ 项目详情里也带关系（前端「关系」区数据源）
    let (_, detail) = send(&app, "GET", &format!("/projects/{child}"), &admin, None).await;
    assert_eq!(detail["links"].as_array().unwrap().len(), 2, "{detail}");

    // ⑨ 两向都可解绑：从**母项目**侧删这条边（边两端都算它的关系）→ 204
    let (st, _) = send(
        &app,
        "DELETE",
        &format!("/projects/{mother}/links/{link_id}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT, "两向都应可解绑");

    let (_, after) = send(
        &app,
        "GET",
        &format!("/projects/{child}/links"),
        &admin,
        None,
    )
    .await;
    assert_eq!(
        after.as_array().unwrap().len(),
        1,
        "part_of 已解绑，只剩 related"
    );

    // ⑩ 重复解绑 → 404
    let (st, _) = send(
        &app,
        "DELETE",
        &format!("/projects/{child}/links/{link_id}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // ⑪ scope 门控
    let no_scope = create_key(&app, &admin, &["memory"]).await;
    let (st, _) = send(
        &app,
        "GET",
        &format!("/projects/{child}/links"),
        &no_scope,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

/// MCP 通道：link / links / unlink 三动作 + 项目 get 带 links。
#[tokio::test]
async fn project_links_via_mcp() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;
    let key = create_key(&app, &admin, &["project"]).await;

    support::mcp_call_json(
        &app,
        &key,
        "projects",
        json!({"action": "create", "name": "MCP-母", "type": "ops"}),
    )
    .await;
    support::mcp_call_json(
        &app,
        &key,
        "projects",
        json!({"action": "create", "name": "MCP-子", "type": "dev"}),
    )
    .await;

    // 按**名称**建关联（agent 最常用路径）
    let link = support::mcp_call_json(
        &app,
        &key,
        "projects",
        json!({"action": "link", "from_project_name": "MCP-子", "to_project_name": "MCP-母",
               "kind": "part_of", "note": "子工作线"}),
    )
    .await;
    assert_eq!(link["kind"], "part_of");
    let link_id = link["id"].as_str().unwrap().to_string();

    // links 动作（两向）
    let links = support::mcp_call_json(
        &app,
        &key,
        "projects",
        json!({"action": "links", "project_name": "MCP-母"}),
    )
    .await;
    assert_eq!(links["count"], 1, "{links}");
    assert_eq!(links["links"][0]["from_name"], "MCP-子");

    // 项目详情带 links
    let detail = support::mcp_call_json(
        &app,
        &key,
        "projects",
        json!({"action": "get", "project_name": "MCP-子"}),
    )
    .await;
    assert_eq!(detail["links"].as_array().unwrap().len(), 1, "{detail}");

    // unlink
    let del = support::mcp_call_json(
        &app,
        &key,
        "projects",
        json!({"action": "unlink", "link_id": link_id}),
    )
    .await;
    assert_eq!(del["deleted"], link_id);
    let links = support::mcp_call_json(
        &app,
        &key,
        "projects",
        json!({"action": "links", "project_name": "MCP-母"}),
    )
    .await;
    assert_eq!(links["count"], 0);
}

//! 资产台账域集成测试：CRUD、别名命名空间、scope 门控、MCP 域可见与可用。
//!
//! 2026-09-21 第 9 个域（《项目与资产模型 · README》§2）：资产 = 我拥有的、可以被操作的东西，
//! 身份唯一、无「收尾」、被项目**引用**（引用打通与删除保护见后续任务）。

mod support;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use support::{app, create_key, expect_result, login_token, mcp_rpc, rpc};
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

/// 类型模板（6 值）+ CRUD 全链 + 别名检索。
#[tokio::test]
async fn asset_crud_and_alias_lookup() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    // 类型模板：与 core 的 ASSET_KINDS 同源
    let (st, v) = send(&app, "GET", "/assets/types", &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    let kinds = v.as_array().unwrap();
    assert_eq!(kinds.len(), 6, "六类资产：{kinds:?}");
    assert!(
        kinds
            .iter()
            .any(|k| k["kind"] == "host" && k["label"] == "主机")
    );

    // 空台账
    let (st, v) = send(&app, "GET", "/assets", &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 0);

    // 建档（带别名——历史写法收容）
    let (st, v) = send(
        &app,
        "POST",
        "/assets",
        &admin,
        Some(json!({
            "kind": "host",
            "name": "MacBook Air M1",
            "aliases": ["trtyr-mac", "demotestdeMacBook-Air.local"],
            "ip": "100.74.134.42",
            "os": "macOS 26.3",
            "note": "本机"
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(v["kind"], "host");
    assert_eq!(v["aliases"].as_array().unwrap().len(), 2);
    let id = v["id"].as_str().unwrap().to_string();

    // 第二个（不同类）——按类型过滤只回一条
    let (st, _) = send(
        &app,
        "POST",
        "/assets",
        &admin,
        Some(json!({"kind": "device", "name": "U盘 trtyr", "note": "256G exFAT"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let (_, v) = send(&app, "GET", "/assets?kind=device", &admin, None).await;
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["name"], "U盘 trtyr");

    // 按别名检索（q 命中 aliases）
    let (_, v) = send(&app, "GET", "/assets?q=trtyr-mac", &admin, None).await;
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1, "别名应可检索：{v}");
    assert_eq!(arr[0]["name"], "MacBook Air M1");

    // 详情：本体 + used_by（此时无项目引用）
    let (st, v) = send(&app, "GET", &format!("/assets/{id}"), &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["name"], "MacBook Air M1");
    assert!(v["used_by"].as_array().unwrap().is_empty());

    // 补丁式更新：只改 ip，name/kind 不动
    let (st, v) = send(
        &app,
        "PUT",
        &format!("/assets/{id}"),
        &admin,
        Some(json!({"ip": "10.61.77.111"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["ip"], "10.61.77.111");
    assert_eq!(v["name"], "MacBook Air M1");
    assert_eq!(v["kind"], "host");
    assert_eq!(
        v["aliases"].as_array().unwrap().len(),
        2,
        "不传 aliases 就不动"
    );

    // 删除 → 204；再读 404
    let (st, _) = send(&app, "DELETE", &format!("/assets/{id}"), &admin, None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _) = send(&app, "GET", &format!("/assets/{id}"), &admin, None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // 非法类型与坏 id
    let (st, v) = send(
        &app,
        "POST",
        "/assets",
        &admin,
        Some(json!({"kind": "bogus", "name": "x"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("host"),
        "错误文案应列出支持类型：{v}"
    );
    let (st, _) = send(&app, "GET", "/assets/not-a-uuid", &admin, None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "坏 UUID 应 400");
}

/// 名称与别名共享命名空间：撞了就 409（先查后写）。
#[tokio::test]
async fn asset_name_and_alias_share_namespace() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    let (st, _) = send(
        &app,
        "POST",
        "/assets",
        &admin,
        Some(json!({"kind": "host", "name": "机A", "aliases": ["aliasA"]})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // 新条目的名字撞别人的别名 → 409
    let (st, v) = send(
        &app,
        "POST",
        "/assets",
        &admin,
        Some(json!({"kind": "host", "name": "aliasA"})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("aliasA"));

    // 新条目的别名撞别人的名字 → 409
    let (st, _) = send(
        &app,
        "POST",
        "/assets",
        &admin,
        Some(json!({"kind": "host", "name": "机B", "aliases": ["机A"]})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);

    // 同名直接建档 → 409
    let (st, _) = send(
        &app,
        "POST",
        "/assets",
        &admin,
        Some(json!({"kind": "host", "name": "机A"})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);

    // 大小写不敏感：MACA 与 maca 视为同名
    let (st, _) = send(
        &app,
        "POST",
        "/assets",
        &admin,
        Some(json!({"kind": "host", "name": "maca"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, _) = send(
        &app,
        "POST",
        "/assets",
        &admin,
        Some(json!({"kind": "host", "name": "MACA"})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
}

/// scope 门控：没有 assets scope 的 key 读写一律 403。
#[tokio::test]
async fn asset_scope_enforcement() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    let no_scope = create_key(&app, &admin, &["memory"]).await;
    let (st, _) = send(&app, "GET", "/assets", &no_scope, None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _) = send(
        &app,
        "POST",
        "/assets",
        &no_scope,
        Some(json!({"kind": "host", "name": "nope"})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let with_scope = create_key(&app, &admin, &["assets"]).await;
    let (st, _) = send(&app, "GET", "/assets", &with_scope, None).await;
    assert_eq!(st, StatusCode::OK);

    // 只读变体（assets:ro）：HTTP REST 侧**保守拒绝**（既有口径——require_scope 精确匹配，
    // `:ro` 只服务 MCP 主通道，见 action_level_scope_test 同款断言）；MCP 侧读放行 / 写拒绝。
    let ro = create_key(&app, &admin, &["assets:ro"]).await;
    let (st, _) = send(&app, "GET", "/assets", &ro, None).await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        ":ro key 打 HTTP REST 应 403（既有口径）"
    );
    let out = support::mcp_call_json(&app, &ro, "assets", json!({"action": "list"})).await;
    assert_eq!(out["count"], 0, ":ro 应可读：{out}");
    let (_, v) = mcp_rpc(
        &app,
        &ro,
        rpc(
            7,
            "tools/call",
            json!({"name": "assets", "arguments": {"action": "add", "kind": "host", "name": "ro-nope"}}),
        ),
    )
    .await;
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains(":ro") && msg.contains("assets"),
        ":ro key 写应被拒且报错可行动：{msg}"
    );
}

/// MCP：assets 是独立域工具，可按别名定位；无 scope 的 key 看不到它。
#[tokio::test]
async fn asset_mcp_domain_visible_and_usable() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;
    let key = create_key(&app, &admin, &["assets"]).await;

    // 建档
    let out = support::mcp_call_json(
        &app,
        &key,
        "assets",
        json!({"action": "add", "kind": "cloud", "name": "腾讯云 · 北京",
               "aliases": ["tencent-beijing", "VM-0-14-opencloudos"],
               "ip": "82.157.147.224", "os": "OpenCloudOS 9.4"}),
    )
    .await;
    assert_eq!(out["name"], "腾讯云 · 北京");
    let id = out["id"].as_str().unwrap().to_string();

    // 按别名定位（get 的 name 参数认别名）
    let out = support::mcp_call_json(
        &app,
        &key,
        "assets",
        json!({"action": "get", "name": "tencent-beijing"}),
    )
    .await;
    assert_eq!(out["name"], "腾讯云 · 北京");
    assert!(out["used_by"].as_array().unwrap().is_empty());

    // list 的检索命中别名
    let out = support::mcp_call_json(
        &app,
        &key,
        "assets",
        json!({"action": "list", "q": "VM-0-14"}),
    )
    .await;
    assert_eq!(out["count"], 1, "{out}");

    // help 有本域操作目录
    let out = support::mcp_call_json(&app, &key, "assets", json!({"action": "help"})).await;
    assert_eq!(out["domain"], "assets");
    let actions: Vec<String> = out["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["action"].as_str().unwrap().to_string())
        .collect();
    for a in ["kinds", "list", "get", "add", "update", "delete"] {
        assert!(
            actions.contains(&a.to_string()),
            "help 应含 {a}：{actions:?}"
        );
    }

    // 未知操作报错列出合法清单
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            3,
            "tools/call",
            json!({"name": "assets", "arguments": {"action": "nope"}}),
        ),
    )
    .await;
    let msg = v["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(msg.contains("kinds"), "未知操作应列合法清单：{msg}");

    // 清理（各用各的，避免互相影响）
    let out = support::mcp_call_json(
        &app,
        &key,
        "assets",
        json!({"action": "delete", "asset_id": id}),
    )
    .await;
    assert_eq!(out["deleted"], id);

    // 无 assets scope 的 key：tools/list 里看不到 assets 域工具
    let mem_key = create_key(&app, &admin, &["memory"]).await;
    let (_, v) = mcp_rpc(&app, &mem_key, rpc(4, "tools/list", json!({}))).await;
    let names: Vec<String> = expect_result(&v, "tools/list")["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert!(
        !names.contains(&"assets".to_string()),
        "memory-only key 不应看到 assets 域：{names:?}"
    );

    // 有 assets scope 的 key：能看到该域 + 域内操作目录
    let (_, v) = mcp_rpc(&app, &key, rpc(5, "tools/list", json!({}))).await;
    let tools = expect_result(&v, "tools/list")["tools"]
        .as_array()
        .unwrap()
        .clone();
    let assets = tools
        .iter()
        .find(|t| t["name"] == "assets")
        .expect("assets 域工具应在列表里");
    assert!(
        assets["description"]
            .as_str()
            .unwrap_or_default()
            .contains("kinds"),
        "域工具描述应带操作目录：{assets}"
    );
}

/// 引用双向打通（MCP 通道）：位置登记按**别名**引用资产 → 项目 get 列「用到的资产」→
/// 资产 get 反查「被哪些项目用到」；被引用的资产不许删（先显式解绑）。
#[tokio::test]
async fn asset_reference_bidirectional_and_delete_guard() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;
    let key = create_key(&app, &admin, &["assets", "project"]).await;

    // 建档资产
    let a = support::mcp_call_json(
        &app,
        &key,
        "assets",
        json!({"action": "add", "kind": "cloud", "name": "腾讯云 · 北京",
               "aliases": ["tencent-beijing"], "ip": "82.157.147.224", "os": "OpenCloudOS 9.4"}),
    )
    .await;
    let asset_id = a["id"].as_str().unwrap().to_string();

    // 建项目 + 位置登记引用资产（只给别名；ip/host/os 留空自动从台账带出）
    let p = support::mcp_call_json(
        &app,
        &key,
        "projects",
        json!({"action": "create", "name": "引用测试项目", "type": "ops"}),
    )
    .await;
    let pid = p["id"].as_str().unwrap().to_string();
    let loc = support::mcp_call_json(
        &app,
        &key,
        "projects",
        json!({"action": "location_add", "project_id": pid, "asset": "tencent-beijing",
               "path": "/opt/x", "purpose": "部署"}),
    )
    .await;
    assert_eq!(loc["asset_id"], asset_id, "应落真引用：{loc}");
    assert_eq!(loc["host"], "腾讯云 · 北京", "身份字段应从台账带出：{loc}");
    assert_eq!(loc["ip"], "82.157.147.224");
    assert_eq!(loc["os"], "OpenCloudOS 9.4");
    let loc_id = loc["id"].as_str().unwrap().to_string();

    // ① 项目 → 资产
    let detail = support::mcp_call_json(
        &app,
        &key,
        "projects",
        json!({"action": "get", "project_id": pid}),
    )
    .await;
    let used = detail["assets"].as_array().unwrap();
    assert_eq!(used.len(), 1, "项目 get 应列用到的资产：{detail}");
    assert_eq!(used[0]["name"], "腾讯云 · 北京");
    assert_eq!(used[0]["location_id"], loc_id);

    // ② 资产 → 项目
    let a2 = support::mcp_call_json(
        &app,
        &key,
        "assets",
        json!({"action": "get", "asset_id": asset_id}),
    )
    .await;
    let used_by = a2["used_by"].as_array().unwrap();
    assert_eq!(used_by.len(), 1, "资产 get 应反查引用方：{a2}");
    assert_eq!(used_by[0]["project_name"], "引用测试项目");
    assert_eq!(used_by[0]["path"], "/opt/x");
    assert_eq!(used_by[0]["purpose"], "部署");

    // ③ 删除被引用的资产 → 被拒（唯一事实源不许静默断链）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            8,
            "tools/call",
            json!({"name": "assets", "arguments": {"action": "delete", "asset_id": asset_id}}),
        ),
    )
    .await;
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("引用测试项目"), "拒绝文案应列出引用方：{msg}");

    // ④ 显式解绑（asset=""）→ 反查清空 → 可删
    support::mcp_call_json(
        &app,
        &key,
        "projects",
        json!({"action": "location_update", "location_id": loc_id, "asset": ""}),
    )
    .await;
    let after = support::mcp_call_json(
        &app,
        &key,
        "assets",
        json!({"action": "get", "asset_id": asset_id}),
    )
    .await;
    assert!(after["used_by"].as_array().unwrap().is_empty());
    let del = support::mcp_call_json(
        &app,
        &key,
        "assets",
        json!({"action": "delete", "asset_id": asset_id}),
    )
    .await;
    assert_eq!(del["deleted"], asset_id);
}

/// 引用走 HTTP 通道（前端路径）：带 asset_id 建位置 → 项目详情列出；悬空引用被拒。
#[tokio::test]
async fn asset_reference_via_http_and_dangling_guard() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    let (st, a) = send(
        &app,
        "POST",
        "/assets",
        &admin,
        Some(json!({"kind": "host", "name": "机H", "aliases": ["hosth"], "ip": "10.0.0.9"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let asset_id = a["id"].as_str().unwrap().to_string();

    let (st, p) = send(
        &app,
        "POST",
        "/projects",
        &admin,
        Some(json!({"name": "HTTP引用项目", "type": "dev"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let pid = p["id"].as_str().unwrap().to_string();

    // 悬空引用（不存在的 asset_id）→ 400
    let ghost = uuid::Uuid::now_v7().to_string();
    let (st, v) = send(
        &app,
        "POST",
        &format!("/projects/{pid}/locations"),
        &admin,
        Some(json!({"ip": "1.2.3.4", "host": "ghost", "os": "x", "path": "/p", "asset_id": ghost})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "悬空引用应被拒：{v}");

    // 合法引用
    let (st, loc) = send(
        &app,
        "POST",
        &format!("/projects/{pid}/locations"),
        &admin,
        Some(json!({"ip": "10.0.0.9", "host": "机H", "os": "linux", "path": "/srv/a", "asset_id": asset_id})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{loc}");
    assert_eq!(loc["asset_id"], asset_id);

    // 项目详情列出用到的资产
    let (st, detail) = send(&app, "GET", &format!("/projects/{pid}"), &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    let used = detail["assets"].as_array().unwrap();
    assert_eq!(used.len(), 1, "{detail}");
    assert_eq!(used[0]["name"], "机H");
    assert_eq!(used[0]["kind"], "host");

    // 位置的 asset_id 也在 locations 里（前端树用）
    assert_eq!(detail["locations"][0]["asset_id"], asset_id);
}

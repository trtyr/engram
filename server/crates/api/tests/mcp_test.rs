//! MCP 端点集成测试：JSON-RPC 全链路（initialize → tools/list → tools/call）。
//! 全链路：真 PG + 完整 router + Bearer 中间件 + rmcp Streamable HTTP 服务。

mod support;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use engram_api::routes;
use engram_api::state::AppState;
use serde_json::{Value, json};
use tower::util::ServiceExt;

async fn app() -> (Router, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");

    let state = AppState::new(pool)
        .with_admin_password(Some("test-admin-pw".into()))
        .with_master_key(Some("ab".repeat(32)));
    (routes::router(state), container)
}

async fn login_token(app: &Router) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"password":"test-admin-pw"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    v["token"].as_str().unwrap().to_string()
}

async fn create_key(app: &Router, token: &str, scopes: &[&str]) -> String {
    let scopes_json = serde_json::json!(scopes);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/settings/api-keys")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(format!(
                    r#"{{"name":"mcp-test","scopes":{scopes_json}}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "签发 key 应成功");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    v["key"].as_str().unwrap().to_string()
}

/// 向 /mcp 发一条 JSON-RPC 请求，返回 HTTP 状态 + 响应 JSON（无状态模式：纯 JSON 响应）。
async fn mcp_rpc(app: &Router, auth: &str, payload: Value) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("host", "localhost")
                .header("authorization", format!("Bearer {auth}"))
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&body)
        .unwrap_or_else(|e| panic!("响应应为 JSON：{e}\n{}", String::from_utf8_lossy(&body)));
    (status, v)
}

fn rpc(id: i64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

/// 从 JSON-RPC 响应取 result（报 panic 带上下文）。
fn expect_result(v: &Value, what: &str) -> Value {
    assert!(
        v.get("error").is_none(),
        "{what} 不应报错：{}",
        v.get("error").unwrap_or(&Value::Null)
    );
    v["result"].clone()
}

#[tokio::test]
async fn mcp_unauthorized_without_credentials() {
    let (app, _pg) = app().await;
    // 无凭证 → 401（Bearer 中间件在 MCP 层之前拦截）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("host", "localhost")
                .body(Body::from(rpc(1, "initialize", json!({})).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    // 坏凭证 → 401
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("host", "localhost")
                .header("authorization", "Bearer amk_bogus")
                .body(Body::from(rpc(1, "initialize", json!({})).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mcp_initialize_and_list_tools() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["memory"]).await;

    let (status, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            1,
            "initialize",
            json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "0.1.0"}
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let result = expect_result(&v, "initialize");
    assert_eq!(result["serverInfo"]["name"], "engram");
    assert!(
        result["instructions"]
            .as_str()
            .unwrap_or("")
            .contains("memory_context"),
        "instructions 应包含使用时机说明"
    );

    let (_, v) = mcp_rpc(&app, &key, rpc(2, "tools/list", json!({}))).await;
    let tools = expect_result(&v, "tools/list")["tools"]
        .as_array()
        .expect("tools 数组")
        .clone();
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    for expected in [
        "memory_context",
        "memory_search",
        "memory_list_atoms",
        "memory_list_sessions",
        "memory_get_session",
        "memory_write_session",
        "memory_append_session",
        "memory_forget",
        "memory_entities",
    ] {
        assert!(
            names.contains(&expected),
            "tools/list 缺少 {expected}：{names:?}"
        );
    }
    // 只读工具应带 readOnlyHint 注解
    let search = tools.iter().find(|t| t["name"] == "memory_search").unwrap();
    assert_eq!(search["annotations"]["readOnlyHint"], json!(true));
    let forget = tools.iter().find(|t| t["name"] == "memory_forget").unwrap();
    assert_eq!(forget["annotations"]["destructiveHint"], json!(true));
}

#[tokio::test]
async fn mcp_tool_call_write_search_forget_journey() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["memory"]).await;

    // initialize（无状态模式下每条请求独立，但客户端仍按规范先 initialize）
    let (status, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            1,
            "initialize",
            json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "0.1.0"}
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    expect_result(&v, "initialize");

    // 写会话（agent 归因缺省取 key 名 mcp-test）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            2,
            "tools/call",
            json!({
                "name": "memory_write_session",
                "arguments": {
                    "turns": [
                        {"speaker": "user", "text": "我叫特让他也让，我在开发 Engram 记忆系统"},
                        {"speaker": "assistant", "text": "好的，我记住了。"}
                    ]
                }
            }),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call write_session");
    assert!(
        !out["isError"].as_bool().unwrap_or(false),
        "write_session 不应报错：{out}"
    );
    let content_text = out["content"][0]["text"].as_str().expect("文本内容");
    let session: Value = serde_json::from_str(content_text).expect("SessionDto JSON");
    assert_eq!(session["agent"], "mcp-test", "agent 归因应取 key 名");

    // 上下文包（无蒸馏产物 → 各层为空但结构完整）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            4,
            "tools/call",
            json!({"name": "memory_context", "arguments": {}}),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call context");
    let content_text = out["content"][0]["text"].as_str().expect("文本内容");
    let pack: Value = serde_json::from_str(content_text).expect("ContextPack JSON");
    assert!(pack.get("meta").is_some(), "ContextPack 应含 meta");

    // 遗忘（void：蒸馏跳过、原文保留）
    let session_id = session["id"].as_str().unwrap().to_string();
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            5,
            "tools/call",
            json!({
                "name": "memory_forget",
                "arguments": {"session_id": session_id, "mode": "void"}
            }),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call forget void");
    assert!(
        !out["isError"].as_bool().unwrap_or(false),
        "void 遗忘不应报错：{out}"
    );
}

#[tokio::test]
async fn mcp_tool_call_l0_read_chain() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["memory"]).await;

    // initialize
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            1,
            "initialize",
            json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "0.1.0"}
            }),
        ),
    )
    .await;
    expect_result(&v, "initialize");

    // 写会话（distill=off，测试环境无 LLM——L1/L2 蒸馏由带 stub 的蒸馏域测试覆盖）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            2,
            "tools/call",
            json!({
                "name": "memory_write_session",
                "arguments": {
                    "distill": "off",
                    "turns": [
                        {"speaker": "user", "text": "我在开发 Engram 记忆系统"},
                        {"speaker": "assistant", "text": "好的。"}
                    ]
                }
            }),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call write_session");
    let content_text = out["content"][0]["text"].as_str().expect("文本内容");
    let session: Value = serde_json::from_str(content_text).expect("SessionDto JSON");
    assert_eq!(session["agent"], "mcp-test");
    let session_id = session["id"].as_str().unwrap().to_string();

    // list_sessions 应看到它
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            3,
            "tools/call",
            json!({"name": "memory_list_sessions", "arguments": {}}),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call list_sessions");
    let sessions: Value =
        serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(
        sessions
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == session["id"]),
        "list_sessions 应包含刚写的会话：{sessions}"
    );

    // get_session 逐轮原文
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            4,
            "tools/call",
            json!({"name": "memory_get_session", "arguments": {"session_id": session_id}}),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call get_session");
    let got: Value = serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(got["id"], session["id"]);

    // 追加轮次
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            5,
            "tools/call",
            json!({
                "name": "memory_append_session",
                "arguments": {
                    "session_id": session_id,
                    "distill": "off",
                    "turns": [{"speaker": "user", "text": "补充一句"}]
                }
            }),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call append_session");
    assert!(!out["isError"].as_bool().unwrap_or(false));

    // 实体检索工具可执行（空库合法）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            6,
            "tools/call",
            json!({"name": "memory_entities", "arguments": {"q": "Engram"}}),
        ),
    )
    .await;
    expect_result(&v, "tools/call entities");

    // 检索工具可执行（无蒸馏产物 → 空结果合法）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            7,
            "tools/call",
            json!({"name": "memory_search", "arguments": {"query": "Engram"}}),
        ),
    )
    .await;
    expect_result(&v, "tools/call search");
}

#[tokio::test]
async fn mcp_scope_enforcement() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    // 无 memory scope 的 key：能过认证，但工具面拒绝
    let wiki_key = create_key(&app, &admin, &["wiki"]).await;
    let (status, v) = mcp_rpc(&app, &wiki_key, rpc(1, "tools/list", json!({}))).await;
    // tools/list 是协议能力（不含业务数据），放行
    assert_eq!(status, StatusCode::OK);
    expect_result(&v, "tools/list");

    let (status, v) = mcp_rpc(
        &app,
        &wiki_key,
        rpc(
            2,
            "tools/call",
            json!({
                "name": "memory_search",
                "arguments": {"query": "test"}
            }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "业务拒绝在 JSON-RPC 错误层，不在 HTTP 层"
    );
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("memory scope"),
        "应报缺少 memory scope：{v}"
    );

    // erase 工具面分权：无 erase scope 的 key 用 erase 模式被拒
    let mem_key = create_key(&app, &admin, &["memory"]).await;
    // 先造一个会话（走 HTTP API 直写）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/memory/sessions")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {mem_key}"))
                .body(Body::from(
                    r#"{"turns":[{"speaker":"user","text":"erase target"}],"distill":"off"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let session: Value = serde_json::from_slice(&body).unwrap();
    let session_id = session["id"].as_str().unwrap().to_string();

    let (_, v) = mcp_rpc(
        &app,
        &mem_key,
        rpc(
            3,
            "tools/call",
            json!({
                "name": "memory_forget",
                "arguments": {"session_id": session_id, "mode": "erase"}
            }),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("erase"),
        "erase 模式需要 erase scope：{v}"
    );

    // void 模式同 key 可用（memory scope 足够）
    let (_, v) = mcp_rpc(
        &app,
        &mem_key,
        rpc(
            4,
            "tools/call",
            json!({
                "name": "memory_forget",
                "arguments": {"session_id": session_id, "mode": "void"}
            }),
        ),
    )
    .await;
    expect_result(&v, "tools/call forget void");
}

#[tokio::test]
async fn mcp_admin_info_endpoint() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/settings/mcp")
                .header("authorization", format!("Bearer {admin}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let info: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(info["endpoint"], "/mcp");
    assert_eq!(info["server_name"], "engram");
    assert_eq!(info["enabled"], json!(true), "缺省配置应全开");
    assert_eq!(info["disabled_tools"], json!([]));
    let tools = info["tools"].as_array().expect("工具清单");
    assert!(tools.len() >= 9, "工具清单应与 MCP 层同源：{}", tools.len());

    // 非 admin 拒绝
    let key = create_key(&app, &admin, &["memory"]).await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/settings/mcp")
                .header("authorization", format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// PUT /settings/mcp（admin-only）：返回更新后的 McpInfo。
async fn put_mcp_config(app: &Router, token: &str, body: Value) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/settings/mcp")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, v)
}

#[tokio::test]
async fn mcp_service_toggle_gates_endpoint() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;
    let key = create_key(&app, &admin, &["memory"]).await;

    // 关闭服务
    let (status, info) = put_mcp_config(&app, &admin, json!({"enabled": false})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(info["enabled"], json!(false));

    // 已认证 key 的 JSON-RPC 也被 gate 拦下（503，先于 MCP 层）
    let (status, v) = mcp_rpc(&app, &key, rpc(1, "tools/list", json!({}))).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "关闭后应 503：{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("MCP 服务已关闭"),
        "503 应带关闭说明：{v}"
    );

    // 重新开启 → 恢复
    let (status, _) = put_mcp_config(&app, &admin, json!({"enabled": true})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, v) = mcp_rpc(&app, &key, rpc(2, "tools/list", json!({}))).await;
    assert_eq!(status, StatusCode::OK);
    expect_result(&v, "重新开启后 tools/list");

    // 非 admin 不能动开关
    let (status, _) = put_mcp_config(&app, &key, json!({"enabled": false})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn mcp_tool_toggle_hides_and_rejects() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;
    let key = create_key(&app, &admin, &["memory"]).await;

    // 停用 write_session
    let (status, info) = put_mcp_config(
        &app,
        &admin,
        json!({"disabled_tools": ["memory_write_session"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(info["disabled_tools"], json!(["memory_write_session"]));

    // tools/list 对 AI 隐身（9 → 8）
    let (_, v) = mcp_rpc(&app, &key, rpc(1, "tools/list", json!({}))).await;
    let result = expect_result(&v, "tools/list");
    let names: Vec<String> = result["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str().map(String::from))
        .collect();
    assert_eq!(names.len(), 16, "17 个工具停用 1 个应剩 16：{names:?}");
    assert!(!names.contains(&"memory_write_session".to_string()));

    // tools/call 直接拒绝
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            2,
            "tools/call",
            json!({
                "name": "memory_write_session",
                "arguments": {"turns": [{"speaker": "user", "text": "x"}]}
            }),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("已停用"),
        "停用工具调用应报错：{v}"
    );

    // 未知工具名 400（防笔误）
    let (status, _) =
        put_mcp_config(&app, &admin, json!({"disabled_tools": ["memory_bogus"]})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 恢复全开 → 工具回归
    let (status, info) = put_mcp_config(&app, &admin, json!({"disabled_tools": []})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(info["disabled_tools"], json!([]));
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            3,
            "tools/call",
            json!({
                "name": "memory_write_session",
                "arguments": {"distill": "off", "turns": [{"speaker": "user", "text": "回归测试"}]}
            }),
        ),
    )
    .await;
    expect_result(&v, "恢复后 write_session");
}

// ---------- Wiki 域工具 ----------

#[tokio::test]
async fn wiki_mcp_tools_listed() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["wiki"]).await;

    let (_, v) = mcp_rpc(&app, &key, rpc(1, "tools/list", json!({}))).await;
    let result = expect_result(&v, "tools/list");
    let tools = result["tools"].as_array().expect("tools 数组");
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    for expected in [
        "wiki_search",
        "wiki_list_pages",
        "wiki_get_page",
        "wiki_write_page",
        "wiki_ingest",
        "wiki_archive_query",
        "wiki_graph",
        "wiki_lint",
    ] {
        assert!(
            names.contains(&expected),
            "tools/list 缺少 {expected}：{names:?}"
        );
    }
    // 只读 wiki 工具应带 readOnlyHint；写工具不应带
    let search = tools.iter().find(|t| t["name"] == "wiki_search").unwrap();
    assert_eq!(search["annotations"]["readOnlyHint"], json!(true));
    let write = tools
        .iter()
        .find(|t| t["name"] == "wiki_write_page")
        .unwrap();
    assert_eq!(write["annotations"]["readOnlyHint"], json!(false));

    // 管理台信息与 MCP 层同源：wiki 域工具出现、domain 前缀正确
    let admin = login_token(&app).await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/settings/mcp")
                .header("authorization", format!("Bearer {admin}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let info: Value = serde_json::from_slice(&body).unwrap();
    let wiki_tools: Vec<&Value> = info["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["domain"] == "wiki")
        .collect();
    assert_eq!(wiki_tools.len(), 8, "管理台应展示 8 个 wiki 工具");
    // instructions 应覆盖两个域
    assert!(
        info["instructions"]
            .as_str()
            .unwrap()
            .contains("wiki_search")
    );
}

#[tokio::test]
async fn wiki_mcp_journey_write_read_search_archive() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["wiki"]).await;

    // 写页面（AI 通道：via=ai）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            1,
            "tools/call",
            json!({
                "name": "wiki_write_page",
                "arguments": {
                    "slug": "tokio-调度器",
                    "title": "Tokio 调度器",
                    "content": "# Tokio 调度器\n\n工作窃取式调度，参见 [[tokio-runtime]]。另有死链 [[not-exist-page]]。"
                }
            }),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call wiki_write_page");
    assert!(
        !out["isError"].as_bool().unwrap_or(false),
        "写页不应报错：{out}"
    );
    let page: Value = serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(page["slug"], "tokio-调度器");
    assert_eq!(page["frontmatter"]["via"], "ai", "AI 写入应落 via 标记");

    // 第二页（被双链目标）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            2,
            "tools/call",
            json!({
                "name": "wiki_write_page",
                "arguments": {
                    "slug": "tokio-runtime",
                    "title": "Tokio Runtime",
                    "content": "# Tokio Runtime\n\n多线程运行时，与 tokio 调度器协同。"
                }
            }),
        ),
    )
    .await;
    expect_result(&v, "tools/call wiki_write_page #2");

    // 覆盖更新：同 slug 再写 → version +1
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            3,
            "tools/call",
            json!({
                "name": "wiki_write_page",
                "arguments": {
                    "slug": "tokio-runtime",
                    "title": "Tokio Runtime",
                    "content": "# Tokio Runtime\n\n更新后的正文。"
                }
            }),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call wiki_write_page 覆盖更新");
    let updated: Value = serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(updated["version"], 2, "同 slug 覆盖应递增版本");

    // 读页面（slug 宽容匹配：空格→连字符）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            4,
            "tools/call",
            json!({"name": "wiki_get_page", "arguments": {"slug": "tokio runtime"}}),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call wiki_get_page");
    let got: Value = serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(got["slug"], "tokio-runtime");
    assert!(got["content"].as_str().unwrap().contains("更新后的正文"));

    // 列表（瘦身：不带正文）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            5,
            "tools/call",
            json!({"name": "wiki_list_pages", "arguments": {}}),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call wiki_list_pages");
    let pages: Value = serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        pages.as_array().unwrap().len(),
        2,
        "应列出两个页面：{pages}"
    );
    assert_eq!(pages[0]["content_omitted"], json!(true), "列表应省略正文");

    // 检索（FTS 命中 + purpose 字段存在）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            6,
            "tools/call",
            json!({"name": "wiki_search", "arguments": {"query": "tokio"}}),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call wiki_search");
    let result: Value = serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(result.get("purpose").is_some(), "检索应返回 purpose 字段");
    let hits = result["pages"].as_array().expect("pages 数组");
    assert!(
        hits.iter().any(|p| p["slug"] == "tokio-runtime"),
        "检索应命中 tokio-runtime：{hits:?}"
    );

    // 链接图：两个节点
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            7,
            "tools/call",
            json!({"name": "wiki_graph", "arguments": {}}),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call wiki_graph");
    let graph: Value = serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        graph["nodes"].as_array().unwrap().len(),
        2,
        "图应有两个节点"
    );

    // lint：应报出正文里的死链
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            8,
            "tools/call",
            json!({"name": "wiki_lint", "arguments": {}}),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call wiki_lint");
    let report: Value = serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["checked_pages"], 2);
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["rule"] == "dead_link"),
        "lint 应报出 [[not-exist-page]] 死链：{report}"
    );

    // 问答存档：首次落页，同标题幂等跳过
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            9,
            "tools/call",
            json!({
                "name": "wiki_archive_query",
                "arguments": {
                    "title": "tokio 调度原理",
                    "question": "tokio 怎么调度任务？",
                    "answer": "工作窃取式多队列调度。"
                }
            }),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call wiki_archive_query");
    let archived: Value =
        serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        archived["skipped"],
        json!(false),
        "首次存档不应跳过：{archived}"
    );

    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            10,
            "tools/call",
            json!({
                "name": "wiki_archive_query",
                "arguments": {
                    "title": "tokio 调度原理",
                    "question": "tokio 怎么调度任务？",
                    "answer": "重复内容。"
                }
            }),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call wiki_archive_query 重复");
    let again: Value = serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        again["skipped"],
        json!(true),
        "同标题重复存档应幂等跳过：{again}"
    );

    // 织入（入队即返回；测试环境无 worker 不消费）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            11,
            "tools/call",
            json!({
                "name": "wiki_ingest",
                "arguments": {"title": "一份新文档", "text": "# 新文档\n\n正文内容供 LLM 织入。"}
            }),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call wiki_ingest");
    let ingested: Value =
        serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(ingested["skipped"], json!(false), "新内容首次织入不应跳过");

    // 非法 slug → JSON-RPC 层 invalid_params
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            12,
            "tools/call",
            json!({
                "name": "wiki_write_page",
                "arguments": {"slug": "bad slug/路径", "title": "x", "content": "x"}
            }),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("slug"),
        "非法 slug 应报错：{v}"
    );
}

#[tokio::test]
async fn wiki_mcp_scope_enforcement() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    // memory-only key 调 wiki 工具 → JSON-RPC 层拒绝（HTTP 200）
    let mem_key = create_key(&app, &admin, &["memory"]).await;
    let (status, v) = mcp_rpc(
        &app,
        &mem_key,
        rpc(
            1,
            "tools/call",
            json!({"name": "wiki_search", "arguments": {"query": "x"}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "业务拒绝在 JSON-RPC 错误层");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("wiki scope"),
        "应报缺少 wiki scope：{v}"
    );

    // wiki-only key 调 memory 工具 → 拒绝（域间分权双向生效）
    let wiki_key = create_key(&app, &admin, &["wiki"]).await;
    let (_, v) = mcp_rpc(
        &app,
        &wiki_key,
        rpc(
            2,
            "tools/call",
            json!({"name": "memory_search", "arguments": {"query": "x"}}),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("memory scope"),
        "应报缺少 memory scope：{v}"
    );

    // 带 wiki scope 的 key 正常可用
    let (status, v) = mcp_rpc(
        &app,
        &wiki_key,
        rpc(
            3,
            "tools/call",
            json!({"name": "wiki_search", "arguments": {"query": "x"}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    expect_result(&v, "wiki key 调 wiki_search");
}

#[tokio::test]
async fn wiki_mcp_tool_toggle_hides_and_rejects() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;
    let key = create_key(&app, &admin, &["wiki"]).await;

    // 停用 wiki_write_page（管理台校验应认识 wiki 域工具名）
    let (status, info) =
        put_mcp_config(&app, &admin, json!({"disabled_tools": ["wiki_write_page"]})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "wiki 工具名应通过管理台校验：{info}"
    );
    assert_eq!(info["disabled_tools"], json!(["wiki_write_page"]));

    // tools/list 隐身
    let (_, v) = mcp_rpc(&app, &key, rpc(1, "tools/list", json!({}))).await;
    let names: Vec<String> = expect_result(&v, "tools/list")["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str().map(String::from))
        .collect();
    assert!(
        !names.contains(&"wiki_write_page".to_string()),
        "停用工具应隐身：{names:?}"
    );
    assert!(
        names.contains(&"wiki_search".to_string()),
        "其余 wiki 工具不受影响"
    );

    // tools/call 拒绝
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            2,
            "tools/call",
            json!({
                "name": "wiki_write_page",
                "arguments": {"slug": "x", "title": "x", "content": "x"}
            }),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("已停用"),
        "停用工具调用应报错：{v}"
    );

    // 未知 wiki 工具名 → 管理台 400
    let (status, _) = put_mcp_config(&app, &admin, json!({"disabled_tools": ["wiki_bogus"]})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

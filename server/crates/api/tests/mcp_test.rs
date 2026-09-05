//! MCP 端点集成测试：JSON-RPC 全链路（initialize → tools/list → tools/call）。
//! 全链路：真 PG + 完整 router + Bearer 中间件 + rmcp Streamable HTTP 服务。

mod support;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use support::{
    app, create_key, expect_result, login_token, mcp_rpc, rpc,
};
use tower::util::ServiceExt;

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
    assert_eq!(names.len(), 8, "停用工具不应出现在 tools/list：{names:?}");
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

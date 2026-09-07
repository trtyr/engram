//! MCP 端点集成测试：JSON-RPC 全链路（initialize → tools/list → tools/call）。
//! 工具面为渐进式发现：六域各一个入口工具，域内操作经 action 分发
//! （调用形态 {"name":"todos","arguments":{"action":"add",...}}）。
//! 全链路：真 PG + 完整 router + Bearer 中间件 + rmcp Streamable HTTP 服务。

mod support;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use support::{app, create_key, expect_result, login_token, mcp_rpc, rpc};
use tower::util::ServiceExt;

/// 域工具调用：action + 平铺参数（渐进式发现语法）。
fn call(id: i64, tool: &str, action: &str, args: Value) -> Value {
    let mut arguments = serde_json::Map::new();
    arguments.insert("action".into(), json!(action));
    if let Value::Object(map) = args {
        for (k, v) in map {
            arguments.insert(k, v);
        }
    }
    rpc(
        id,
        "tools/call",
        json!({"name": tool, "arguments": arguments}),
    )
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
            .contains("渐进式发现"),
        "instructions 应说明渐进式发现用法：{result}"
    );

    let (_, v) = mcp_rpc(&app, &key, rpc(2, "tools/list", json!({}))).await;
    let tools = expect_result(&v, "tools/list")["tools"]
        .as_array()
        .expect("tools 数组")
        .clone();
    let mut names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    names.sort_unstable();
    // memory-only key 看到 memory 域工具 + 跨域 search_all（按任一 scope 可见）
    assert_eq!(
        names,
        vec!["memory", "search_all"],
        "memory-only key 应见 memory 域工具与 search_all"
    );
    let memory = tools
        .iter()
        .find(|t| t["name"] == "memory")
        .expect("memory 工具");
    // 描述带操作目录（L0 发现层）：action 概览 + help 提示
    let description = memory["description"].as_str().unwrap_or("");
    assert!(
        description.contains("action=\"help\"") || description.contains("help"),
        "域工具描述应提示 help：{description}"
    );
    assert!(
        description.contains("- search："),
        "域工具描述应带操作目录：{description}"
    );
    // inputSchema：action 必填
    assert!(
        memory["inputSchema"]["properties"]["action"].is_object(),
        "inputSchema 应有 action 属性"
    );
    // 域内含破坏性操作（forget）→ destructiveHint
    assert_eq!(memory["annotations"]["destructiveHint"], json!(true));
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
        call(
            2,
            "memory",
            "write_session",
            json!({
                "turns": [
                    {"speaker": "user", "text": "我叫特让他也让，我在开发 Engram 记忆系统"},
                    {"speaker": "assistant", "text": "好的，我记住了。"}
                ]
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
    let (_, v) = mcp_rpc(&app, &key, call(4, "memory", "context", json!({}))).await;
    let out = expect_result(&v, "tools/call context");
    let content_text = out["content"][0]["text"].as_str().expect("文本内容");
    let pack: Value = serde_json::from_str(content_text).expect("ContextPack JSON");
    assert!(pack.get("meta").is_some(), "ContextPack 应含 meta");

    // 遗忘（void：蒸馏跳过、原文保留）
    let session_id = session["id"].as_str().unwrap().to_string();
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            5,
            "memory",
            "forget",
            json!({"session_id": session_id, "mode": "void"}),
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
        call(
            2,
            "memory",
            "write_session",
            json!({
                "distill": "off",
                "turns": [
                    {"speaker": "user", "text": "我在开发 Engram 记忆系统"},
                    {"speaker": "assistant", "text": "好的。"}
                ]
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
    let (_, v) = mcp_rpc(&app, &key, call(3, "memory", "list_sessions", json!({}))).await;
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
        call(
            4,
            "memory",
            "get_session",
            json!({"session_id": session_id}),
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
        call(
            5,
            "memory",
            "append_session",
            json!({
                "session_id": session_id,
                "distill": "off",
                "turns": [{"speaker": "user", "text": "补充一句"}]
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
        call(6, "memory", "entities", json!({"q": "Engram"})),
    )
    .await;
    expect_result(&v, "tools/call entities");

    // 检索工具可执行（无蒸馏产物 → 空结果合法）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(7, "memory", "search", json!({"query": "Engram"})),
    )
    .await;
    expect_result(&v, "tools/call search");
}

#[tokio::test]
async fn mcp_scope_enforcement() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    // 无 memory scope 的 key：能过认证，但域工具拒绝
    let wiki_key = create_key(&app, &admin, &["wiki"]).await;
    let (status, v) = mcp_rpc(&app, &wiki_key, rpc(1, "tools/list", json!({}))).await;
    // tools/list 是协议能力（不含业务数据），放行
    assert_eq!(status, StatusCode::OK);
    expect_result(&v, "tools/list");

    let (status, v) = mcp_rpc(
        &app,
        &wiki_key,
        call(2, "memory", "search", json!({"query": "test"})),
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
        call(
            3,
            "memory",
            "forget",
            json!({"session_id": session_id, "mode": "erase"}),
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
        call(
            4,
            "memory",
            "forget",
            json!({"session_id": session_id, "mode": "void"}),
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
    assert_eq!(
        tools.len(),
        7,
        "应为六个域工具 + search_all：{}",
        tools.len()
    );
    let memory = tools.iter().find(|t| t["name"] == "memory").unwrap();
    assert_eq!(
        memory["actions"].as_array().unwrap().len(),
        10,
        "memory 域应展示 10 个操作：{memory}"
    );

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

    // 停用 memory.write_session（action 级开关）
    let (status, info) = put_mcp_config(
        &app,
        &admin,
        json!({"disabled_tools": ["memory.write_session"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "action 键应通过校验：{info}");
    assert_eq!(info["disabled_tools"], json!(["memory.write_session"]));

    // 域工具本身仍在 tools/list（工具级隐身只对整域停用生效）
    let (_, v) = mcp_rpc(&app, &key, rpc(1, "tools/list", json!({}))).await;
    let result = expect_result(&v, "tools/list");
    let names: Vec<String> = result["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str().map(String::from))
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        vec!["memory", "search_all"],
        "域工具应保留（+跨域 search_all）：{names:?}"
    );
    // 描述目录里 write_session 应隐身
    let domain = result["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "memory")
        .expect("memory 工具");
    let description = domain["description"].as_str().unwrap_or("");
    assert!(
        !description.contains("- write_session："),
        "停用操作应从目录隐身：{description}"
    );
    assert!(
        description.contains("- search："),
        "其余操作不受影响：{description}"
    );

    // tools/call 直接拒绝
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            2,
            "memory",
            "write_session",
            json!({"turns": [{"speaker": "user", "text": "x"}]}),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("已停用"),
        "停用操作调用应报错：{v}"
    );

    // help 手册同步隐身
    let (_, v) = mcp_rpc(&app, &key, call(3, "memory", "help", json!({}))).await;
    let out = expect_result(&v, "help");
    let manual: Value = serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    let actions: Vec<&str> = manual["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["action"].as_str())
        .collect();
    assert_eq!(
        actions.len(),
        9,
        "停用操作应从手册隐身（10 含 remember，停 1 剩 9）：{actions:?}"
    );
    assert!(!actions.contains(&"write_session"));

    // 未知键 400（防笔误）
    let (status, _) =
        put_mcp_config(&app, &admin, json!({"disabled_tools": ["memory_bogus"]})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 恢复全开 → 操作回归
    let (status, info) = put_mcp_config(&app, &admin, json!({"disabled_tools": []})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(info["disabled_tools"], json!([]));
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            4,
            "memory",
            "write_session",
            json!({"distill": "off", "turns": [{"speaker": "user", "text": "回归测试"}]}),
        ),
    )
    .await;
    expect_result(&v, "恢复后 write_session");
}

// ---------- 渐进式发现（help / 未知 action / 坏参数自愈） ----------

#[tokio::test]
async fn mcp_progressive_discovery_help_and_unknown_action() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["todos"]).await;

    // tools/list：todos-only key 恰见一个域工具，描述带操作目录
    let (_, v) = mcp_rpc(&app, &key, rpc(1, "tools/list", json!({}))).await;
    let result = expect_result(&v, "tools/list");
    let names: Vec<&str> = result["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert_eq!(names, vec!["search_all", "todos"]);

    // help：一轮取回全域操作手册（含参数 schema）
    let (_, v) = mcp_rpc(&app, &key, call(2, "todos", "help", json!({}))).await;
    let out = expect_result(&v, "help");
    let manual: Value = serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(manual["domain"], "todos");
    let actions = manual["actions"].as_array().unwrap();
    assert_eq!(actions.len(), 6, "todos 应有 6 个操作：{manual}");
    let add = actions.iter().find(|a| a["action"] == "add").unwrap();
    assert!(
        add["parameters"]["properties"]["title"].is_object(),
        "手册应含 add 的参数 schema：{add}"
    );
    assert_eq!(add["destructive"], json!(false));
    let del = actions.iter().find(|a| a["action"] == "delete").unwrap();
    assert_eq!(del["destructive"], json!(true));

    // 未知 action → 报错即发现（列合法操作 + help 提示）
    let (_, v) = mcp_rpc(&app, &key, call(3, "todos", "nope", json!({}))).await;
    let msg = v["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("nope"), "报错应回显未知操作：{v}");
    assert!(msg.contains("add"), "报错应列合法操作：{v}");
    assert!(msg.contains("help"), "报错应提示 help：{v}");

    // 坏参数 → 报错指向 help（自愈）
    let (_, v) = mcp_rpc(&app, &key, call(4, "todos", "add", json!({}))).await;
    let msg = v["error"]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("todos.add") && msg.contains("help"),
        "坏参数报错应带操作名与 help 提示：{v}"
    );
}

// ---------- 技能域（skills）----------

#[tokio::test]
async fn mcp_skills_tools_listed_with_domain() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["skills"]).await;

    // initialize：instructions 应包含技能域说明
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
    let instructions = expect_result(&v, "initialize")["instructions"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert!(
        instructions.contains("skills 域用法"),
        "instructions 应包含技能域说明：{instructions}"
    );

    // tools/list：skills-only key 只见 skills 域工具，描述带 8 操作目录
    let (_, v) = mcp_rpc(&app, &key, rpc(2, "tools/list", json!({}))).await;
    let result = expect_result(&v, "tools/list");
    let names: Vec<&str> = result["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    let mut sorted: Vec<&str> = names.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        vec!["search_all", "skills"],
        "skills-only key 应见 skills 域工具与 search_all"
    );
    let description = result["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "skills")
        .expect("skills 工具")["description"]
        .as_str()
        .unwrap_or("");
    for action in [
        "- list：",
        "- get：",
        "- file_get：",
        "- file_put：",
        "- create：",
        "- update：",
        "- delete：",
        "- import：",
    ] {
        assert!(
            description.contains(action),
            "目录缺 {action}：{description}"
        );
    }
    // delete 是破坏性操作，目录应标注
    assert!(
        description.contains("【破坏性】"),
        "破坏性操作应标注：{description}"
    );

    // 管理端点：skills 域工具下挂 8 个操作（前端管理台按此自动分组）
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
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let info: Value = serde_json::from_slice(&body).unwrap();
    let skills_tools: Vec<&Value> = info["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["domain"] == "skills")
        .collect();
    assert_eq!(skills_tools.len(), 1, "管理端点应展示 1 个 skills 域工具");
    assert_eq!(
        skills_tools[0]["actions"].as_array().unwrap().len(),
        10,
        "skills 域应展示 10 个操作（含 versions/restore）"
    );
}

#[tokio::test]
async fn mcp_skills_crud_journey() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["skills"]).await;

    // create
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            1,
            "skills",
            "create",
            json!({
                "name": "PR 审查",
                "slug": "review-pr",
                "description": "审查 Rust PR 的固定流程",
                "content": "# 审查步骤\n1. 读 diff\n2. 跑 clippy",
                "tags": ["rust", "review"]
            }),
        ),
    )
    .await;
    let result = expect_result(&v, "skills create");
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("review-pr"),
        "创建结果应回显 slug：{result}"
    );

    // list（q 命中）
    let (_, v) = mcp_rpc(&app, &key, call(2, "skills", "list", json!({"q": "审查"}))).await;
    let result = expect_result(&v, "skills list");
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("review-pr"),
        "列表应包含刚建的技能：{result}"
    );

    // get
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(3, "skills", "get", json!({"slug": "review-pr"})),
    )
    .await;
    assert!(
        expect_result(&v, "skills get")["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("读 diff")
    );

    // update（改正文 + 停用）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            4,
            "skills",
            "update",
            json!({"slug": "review-pr", "content": "# 新流程", "enabled": false}),
        ),
    )
    .await;
    let updated: Value = serde_json::from_str(
        expect_result(&v, "skills update")["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    // P0-1：写操作瘦身——正文不回显，只有 content_chars
    assert!(
        updated["content_chars"].is_i64(),
        "update 应回元数据：{updated}"
    );
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(5, "skills", "get", json!({"slug": "review-pr"})),
    )
    .await;
    let got: Value = serde_json::from_str(
        expect_result(&v, "skills get 回读")["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert!(
        got["content"].as_str().unwrap().contains("新流程"),
        "更新应落库：{got}"
    );

    // 停用后 enabled 过滤查不到
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(5, "skills", "list", json!({"enabled": true})),
    )
    .await;
    assert!(
        !expect_result(&v, "enabled 过滤")["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("review-pr"),
        "停用技能不应出现在 enabled=true 列表"
    );

    // import（SKILL.md 全文 + frontmatter）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            6,
            "skills",
            "import",
            json!({
                "content": "---\nname: Deploy Check\ndescription: 部署前检查清单\nslug: deploy-check\ntags: ops\n---\n# 检查清单\n- 健康检查\n- 回滚预案"
            }),
        ),
    )
    .await;
    let imported = expect_result(&v, "skills import");
    assert!(
        imported["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("deploy-check")
    );

    // delete（先重新启用被停用的技能）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            7,
            "skills",
            "update",
            json!({"slug": "review-pr", "enabled": true}),
        ),
    )
    .await;
    expect_result(&v, "重新启用");
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(8, "skills", "delete", json!({"slug": "review-pr"})),
    )
    .await;
    expect_result(&v, "skills delete");
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(9, "skills", "get", json!({"slug": "review-pr"})),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("不存在"),
        "删除后 get 应报不存在：{v}"
    );
}

#[tokio::test]
async fn mcp_skills_scope_enforcement() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    // 只有 memory scope 的 key：tools/list 放行（协议能力），skills 域调用被拒
    let mem_key = create_key(&app, &admin, &["memory"]).await;
    let (_, v) = mcp_rpc(
        &app,
        &mem_key,
        call(1, "skills", "create", json!({"name": "x", "content": "y"})),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("skills scope"),
        "无 skills scope 应被拒：{v}"
    );

    // skills scope 的 key 调 memory 域同样被拒（域间分权）
    let skills_key = create_key(&app, &admin, &["skills"]).await;
    let (_, v) = mcp_rpc(
        &app,
        &skills_key,
        call(2, "memory", "search", json!({"query": "test"})),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("memory scope"),
        "skills key 调记忆域应被拒：{v}"
    );
}

// ---------- Wiki 域（wiki）----------

#[tokio::test]
async fn wiki_mcp_tools_listed() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["wiki"]).await;

    let (_, v) = mcp_rpc(&app, &key, rpc(1, "tools/list", json!({}))).await;
    let result = expect_result(&v, "tools/list");
    let tools = result["tools"].as_array().expect("tools 数组");
    let mut names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec!["search_all", "wiki"],
        "wiki-only key 应见 wiki 域工具与 search_all"
    );
    // 描述目录：操作齐备
    let description = tools
        .iter()
        .find(|t| t["name"] == "wiki")
        .expect("wiki 工具")["description"]
        .as_str()
        .unwrap_or("");
    for action in [
        "- search：",
        "- list_pages：",
        "- get_page：",
        "- write_page：",
        "- ingest：",
        "- archive_query：",
        "- graph：",
        "- lint：",
        "- delete_page：",
    ] {
        assert!(
            description.contains(action),
            "目录缺 {action}：{description}"
        );
    }

    // 管理台信息与 MCP 层同源：wiki 域工具 + 9 操作
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
    assert_eq!(wiki_tools.len(), 1, "管理台应展示 1 个 wiki 域工具");
    assert_eq!(
        wiki_tools[0]["actions"].as_array().unwrap().len(),
        14,
        "wiki 域应展示 14 个操作（含版本与原料通道）"
    );

    // instructions 应覆盖 wiki 域
    assert!(
        info["instructions"]
            .as_str()
            .unwrap()
            .contains("wiki 域用法")
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
        call(
            1,
            "wiki",
            "write_page",
            json!({
                "slug": "tokio-调度器",
                "title": "Tokio 调度器",
                "content": "# Tokio 调度器\n\n工作窃取式调度，参见 [[tokio-runtime]]。另有死链 [[not-exist-page]]。"
            }),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call wiki write_page");
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
        call(
            2,
            "wiki",
            "write_page",
            json!({
                "slug": "tokio-runtime",
                "title": "Tokio Runtime",
                "content": "# Tokio Runtime\n\n多线程运行时，与 tokio 调度器协同。"
            }),
        ),
    )
    .await;
    expect_result(&v, "tools/call wiki write_page #2");

    // 覆盖更新：同 slug 再写 → version +1
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            3,
            "wiki",
            "write_page",
            json!({
                "slug": "tokio-runtime",
                "title": "Tokio Runtime",
                "content": "# Tokio Runtime\n\n更新后的正文。"
            }),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call wiki write_page 覆盖更新");
    let updated: Value = serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(updated["version"], 2, "同 slug 覆盖应递增版本");

    // 读页面（slug 宽容匹配：空格→连字符）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(4, "wiki", "get_page", json!({"slug": "tokio runtime"})),
    )
    .await;
    let out = expect_result(&v, "tools/call wiki get_page");
    let got: Value = serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(got["slug"], "tokio-runtime");
    assert!(got["content"].as_str().unwrap().contains("更新后的正文"));

    // 列表（瘦身：不带正文）
    let (_, v) = mcp_rpc(&app, &key, call(5, "wiki", "list_pages", json!({}))).await;
    let out = expect_result(&v, "tools/call wiki list_pages");
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
        call(6, "wiki", "search", json!({"query": "tokio"})),
    )
    .await;
    let out = expect_result(&v, "tools/call wiki search");
    let result: Value = serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(result.get("purpose").is_some(), "检索应返回 purpose 字段");
    let hits = result["pages"].as_array().expect("pages 数组");
    assert!(
        hits.iter().any(|p| p["slug"] == "tokio-runtime"),
        "检索应命中 tokio-runtime：{hits:?}"
    );

    // 链接图：两个节点
    let (_, v) = mcp_rpc(&app, &key, call(7, "wiki", "graph", json!({}))).await;
    let out = expect_result(&v, "tools/call wiki graph");
    let graph: Value = serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        graph["nodes"].as_array().unwrap().len(),
        2,
        "图应有两个节点"
    );

    // lint：应报出正文里的死链
    let (_, v) = mcp_rpc(&app, &key, call(8, "wiki", "lint", json!({}))).await;
    let out = expect_result(&v, "tools/call wiki lint");
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
        call(
            9,
            "wiki",
            "archive_query",
            json!({
                "title": "tokio 调度原理",
                "question": "tokio 怎么调度任务？",
                "answer": "工作窃取式多队列调度。"
            }),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call wiki archive_query");
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
        call(
            10,
            "wiki",
            "archive_query",
            json!({
                "title": "tokio 调度原理",
                "question": "tokio 怎么调度任务？",
                "answer": "重复内容。"
            }),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call wiki archive_query 重复");
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
        call(
            11,
            "wiki",
            "ingest",
            json!({"title": "一份新文档", "text": "# 新文档\n\n正文内容供 LLM 织入。"}),
        ),
    )
    .await;
    let out = expect_result(&v, "tools/call wiki ingest");
    let ingested: Value =
        serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(ingested["skipped"], json!(false), "新内容首次织入不应跳过");

    // 非法 slug → JSON-RPC 层 invalid_params
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            12,
            "wiki",
            "write_page",
            json!({"slug": "bad slug/路径", "title": "x", "content": "x"}),
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

    // memory-only key 调 wiki 域 → JSON-RPC 层拒绝（HTTP 200）
    let mem_key = create_key(&app, &admin, &["memory"]).await;
    let (status, v) = mcp_rpc(
        &app,
        &mem_key,
        call(1, "wiki", "search", json!({"query": "x"})),
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

    // wiki-only key 调 memory 域 → 拒绝（域间分权双向生效）
    let wiki_key = create_key(&app, &admin, &["wiki"]).await;
    let (_, v) = mcp_rpc(
        &app,
        &wiki_key,
        call(2, "memory", "search", json!({"query": "x"})),
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
        call(3, "wiki", "search", json!({"query": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    expect_result(&v, "wiki key 调 wiki search");
}

#[tokio::test]
async fn wiki_mcp_tool_toggle_hides_and_rejects() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;
    let key = create_key(&app, &admin, &["wiki"]).await;

    // 停用 wiki.write_page（管理台校验应认识 action 键）
    let (status, info) =
        put_mcp_config(&app, &admin, json!({"disabled_tools": ["wiki.write_page"]})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "wiki.write_page 应通过管理台校验：{info}"
    );
    assert_eq!(info["disabled_tools"], json!(["wiki.write_page"]));

    // 域工具保留，描述目录里 write_page 隐身
    let (_, v) = mcp_rpc(&app, &key, rpc(1, "tools/list", json!({}))).await;
    let result = expect_result(&v, "tools/list");
    let names: Vec<String> = result["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str().map(String::from))
        .collect();
    let mut sorted: Vec<String> = names.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        vec!["search_all".to_string(), "wiki".to_string()],
        "域工具应保留（+跨域 search_all）：{names:?}"
    );
    let description = result["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "wiki")
        .expect("wiki 工具")["description"]
        .as_str()
        .unwrap_or("");
    assert!(
        !description.contains("- write_page："),
        "停用操作应从目录隐身：{description}"
    );
    assert!(
        description.contains("- search："),
        "其余操作不受影响：{description}"
    );

    // tools/call 拒绝
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            2,
            "wiki",
            "write_page",
            json!({"slug": "x", "title": "x", "content": "x"}),
        ),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("已停用"),
        "停用操作调用应报错：{v}"
    );

    // 未知键 → 管理台 400
    let (status, _) = put_mcp_config(&app, &admin, json!({"disabled_tools": ["wiki_bogus"]})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

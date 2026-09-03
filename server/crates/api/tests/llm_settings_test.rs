//! LLM 配置写入口校验集成测试（L1/L3）。
//!
//! L1：create_provider 全量校验（400 带明细，UNIQUE 冲突不再 503）；
//! L3：is_default 唯一性（新建默认时事务降级存量默认）。

mod support;

use agent_memory_api::routes;
use agent_memory_api::state::AppState;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;

async fn app() -> (Router, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");
    let state = AppState::new(pool)
        .with_admin_password(Some("test-admin-pw".into()))
        .with_master_key(Some("ab".repeat(32)));
    (routes::router(state), container)
}

async fn token(app: &Router) -> String {
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
    serde_json::from_slice::<serde_json::Value>(&body).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn create(app: &Router, token: &str, body: &serde_json::Value) -> StatusCode {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/settings/llm/providers")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    resp.status()
}

fn valid_body(name: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "base_url": "https://api.example.com/v1",
        "api_key": "sk-test-key-123456",
        "model_id": "m-chat",
        "capability": "chat",
        "is_default": false,
    })
}

#[tokio::test]
async fn l1_invalid_provider_inputs_rejected_400() {
    let (app, _pg) = app().await;
    let token = token(&app).await;

    // 空 name
    let mut b = valid_body("");
    assert_eq!(create(&app, &token, &b).await, StatusCode::BAD_REQUEST);
    // base_url 无协议
    b = valid_body("p1");
    b["base_url"] = "api.example.com".into();
    assert_eq!(create(&app, &token, &b).await, StatusCode::BAD_REQUEST);
    // base_url 带空白
    b = valid_body("p2");
    b["base_url"] = "https://api .example.com".into();
    assert_eq!(create(&app, &token, &b).await, StatusCode::BAD_REQUEST);
    // 空 api_key
    b = valid_body("p3");
    b["api_key"] = "  ".into();
    assert_eq!(create(&app, &token, &b).await, StatusCode::BAD_REQUEST);
    // SEC-C：过短 api_key（<8 字符，挡手滑占位串；真实性由 provider-test 判定）
    b = valid_body("p3b");
    b["api_key"] = "sk-x".into();
    assert_eq!(create(&app, &token, &b).await, StatusCode::BAD_REQUEST);
    // 非法 capability
    b = valid_body("p4");
    b["capability"] = "vision".into();
    assert_eq!(create(&app, &token, &b).await, StatusCode::BAD_REQUEST);
    // 全部非法输入都没有落库
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/settings/llm/providers")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let list: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 0, "非法输入不得入库");
}

#[tokio::test]
async fn l1_duplicate_name_maps_to_400_not_503() {
    let (app, _pg) = app().await;
    let token = token(&app).await;

    let b = valid_body("dup");
    assert_eq!(create(&app, &token, &b).await, StatusCode::CREATED);
    // 重名：UNIQUE 冲突必须 400（旧路径 503 storage_unavailable retryable）
    let status = create(&app, &token, &valid_body("dup")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "重名应 400 而非 {status}");
}

#[tokio::test]
async fn l3_new_default_demotes_existing() {
    let (app, _pg) = app().await;
    let token = token(&app).await;

    let mut a = valid_body("first");
    a["is_default"] = true.into();
    assert_eq!(create(&app, &token, &a).await, StatusCode::CREATED);

    let mut b = valid_body("second");
    b["is_default"] = true.into();
    assert_eq!(create(&app, &token, &b).await, StatusCode::CREATED);

    // 直接查库断言唯一性（响应 DTO 只反映请求值）
    // （借 registry 解析验证：默认只应是 second——通过 resolve 用的 provider name 检查）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/settings/llm/providers")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let list: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let defaults: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["is_default"].as_bool().unwrap_or(false))
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(defaults, vec!["second"], "新建默认时旧默认应被降级: {list}");
}

// ---------- L2：provider 生命周期（更新/删除/重加密） ----------

async fn create_full(app: &Router, token: &str, body: &serde_json::Value) -> serde_json::Value {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/settings/llm/providers")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn req_json(
    app: &Router,
    token: &str,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut b = Request::builder().method(method).uri(uri);
    if body.is_some() {
        b = b.header("content-type", "application/json");
    }
    let resp = app
        .clone()
        .oneshot(
            b.header("authorization", format!("Bearer {token}"))
                .body(Body::from(body.map(|v| v.to_string()).unwrap_or_default()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, v)
}

#[tokio::test]
async fn l2_provider_lifecycle_update_delete_reencrypt() {
    let (app, _pg) = app().await;
    let token = token(&app).await;

    // 建 default + 普通 provider
    let mut d = valid_body("main");
    d["is_default"] = true.into();
    let dp = create_full(&app, &token, &d).await;
    let sp = create_full(&app, &token, &valid_body("spare")).await;

    // PUT：换 key + 换 models + 升默认
    let (st, v) = req_json(
        &app,
        &token,
        "PUT",
        &format!("/settings/llm/providers/{}", sp["id"].as_str().unwrap()),
        Some(serde_json::json!({
            "api_key": "sk-new-key",
            "model_id": "s-chat",
            "is_default": true,
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v:?}");
    assert_eq!(v["is_default"], true, "spare 应升为默认");
    assert_eq!(v["model_id"], "s-chat");
    // 旧默认被降级
    let (_, list) = req_json(&app, &token, "GET", "/settings/llm/providers", None).await;
    let main = list
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "main")
        .unwrap();
    assert_eq!(main["is_default"], false, "PUT 升默认应降级旧默认");

    // PUT 不存在的 id → 404
    let (st, _) = req_json(
        &app,
        &token,
        "PUT",
        &format!("/settings/llm/providers/{}", uuid::Uuid::new_v4()),
        Some(serde_json::json!({"is_default": false})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // DELETE 默认 provider → 400（须先转移默认）
    let (st, v) = req_json(
        &app,
        &token,
        "DELETE",
        &format!("/settings/llm/providers/{}", sp["id"].as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v:?}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("默认")
    );

    // 把默认还给 main，再删 spare → 204
    let (st, _) = req_json(
        &app,
        &token,
        "PUT",
        &format!("/settings/llm/providers/{}", dp["id"].as_str().unwrap()),
        Some(serde_json::json!({"is_default": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = req_json(
        &app,
        &token,
        "DELETE",
        &format!("/settings/llm/providers/{}", sp["id"].as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    // 列表只剩 main
    let (_, list) = req_json(&app, &token, "GET", "/settings/llm/providers", None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);

    // re-encrypt：旧密钥（当前配置的 "ab"*32）→ 用同一把演示轮换（幂等验证解密链）
    // 正确旧密钥 → 成功计数；错误旧密钥 → 400 且密文未动
    let (st, v) = req_json(
        &app,
        &token,
        "POST",
        "/settings/llm/providers/re-encrypt",
        Some(serde_json::json!({"old_master_key": "ab".repeat(32)})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v:?}");
    assert_eq!(v["re_encrypted"], 1);

    let (st, _) = req_json(
        &app,
        &token,
        "POST",
        "/settings/llm/providers/re-encrypt",
        Some(serde_json::json!({"old_master_key": "cd".repeat(32)})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "错误旧密钥应 400");
}

// ---------- L4：routing PUT 校验 ----------

#[tokio::test]
async fn l4_routing_put_validation() {
    let (app, _pg) = app().await;
    let token = token(&app).await;

    // 建 provider 供合法路由引用
    let d = valid_body("routed");
    create_full(&app, &token, &d).await;

    // typo purpose → 400（旧路径静默入库永不生效）
    let (st, v) = req_json(
        &app,
        &token,
        "PUT",
        "/settings/llm/routing",
        Some(serde_json::json!({"extarct": [{"provider": "routed", "model": "m-chat"}]})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v:?}");
    assert!(v["error"]["message"].as_str().unwrap().contains("extarct"));

    // 幽灵 provider → 400 带明细
    let (st, v) = req_json(
        &app,
        &token,
        "PUT",
        "/settings/llm/routing",
        Some(serde_json::json!({"extract": [{"provider": "ghost", "model": "x"}]})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v:?}");
    assert!(v["error"]["message"].as_str().unwrap().contains("ghost"));

    // model 不在册 → 400
    let (st, v) = req_json(
        &app,
        &token,
        "PUT",
        "/settings/llm/routing",
        Some(serde_json::json!({"extract": [{"provider": "routed", "model": "no-such-model"}]})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v:?}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no-such-model")
    );

    // 合法表 → 204 且读回一致（routed 是 chat provider，model 需与 model_id 一致）
    let (st, _) = req_json(
        &app,
        &token,
        "PUT",
        "/settings/llm/routing",
        Some(serde_json::json!({
            "extract": [{"provider": "routed", "model": "m-chat"}],
        })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, v) = req_json(&app, &token, "GET", "/settings/llm/routing", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["extract"][0]["provider"], "routed");
    assert_eq!(v["extract"][0]["model"], "m-chat");

    // 空表合法（清空路由的文档化路径）
    let (st, _) = req_json(
        &app,
        &token,
        "PUT",
        "/settings/llm/routing",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
}

// ---------- L10：占位主密钥预警 ----------

#[tokio::test]
async fn l10_placeholder_master_key_warns_on_create() {
    let (app, _pg) = app().await;
    let token = token(&app).await;

    // 本测试 app 配置的是真实密钥（"ab"*32）→ 无 warning
    let v = create_full(&app, &token, &valid_body("normal-key")).await;
    assert!(v.get("warning").is_none(), "真实密钥下创建不告警: {v:?}");

    // 占位密钥检测的单元语义（AppState 方法）
    // —— 占位 app 需要单独构造：直接验证方法在两种配置下的返回值
    // （API 层的 warning 注入路径与上面 create 相同，状态由 with_master_key 决定）
}

#[tokio::test]
async fn l10_placeholder_state_detection() {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");

    let none_key = AppState::new(pool.clone());
    assert!(none_key.is_placeholder_master_key(), "未设密钥 = 占位态");

    let placeholder = AppState::new(pool.clone()).with_master_key(Some("00".repeat(32)));
    assert!(placeholder.is_placeholder_master_key(), "00*32 = 占位态");

    let real = AppState::new(pool.clone()).with_master_key(Some("ab".repeat(32)));
    assert!(!real.is_placeholder_master_key(), "真实密钥非占位态");

    drop(container);
}

// ---------- L3/L10 补强：热路径确定性与占位告警端到端 ----------

/// L3：resolve 热路径——存量多默认行时按 created_at 取最早（不再依赖物理顺序）。
#[tokio::test]
async fn l3_resolve_hot_path_deterministic_default() {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");

    use agent_memory_llm::KeyCipher;
    use agent_memory_llm::provider::ProviderRegistry;
    use agent_memory_llm::types::Purpose;
    let registry = ProviderRegistry::new(
        pool.clone(),
        KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
    );

    // 直插两行 default（绕过 API——模拟本修复前的存量脏数据）
    let cipher = KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap();
    for (name, created) in [
        ("older", "2020-01-01T00:00:00Z"),
        ("newer", "2024-01-01T00:00:00Z"),
    ] {
        sqlx::query(
            "INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, model_id, capability, is_default, created_at)
             VALUES ($1, $2, 'http://127.0.0.1:1', $3, $4, 'chat', true, $5::timestamptz)",
        )
        .bind(uuid::Uuid::new_v4())
        .bind(name)
        .bind(cipher.encrypt("k").unwrap())
        .bind(format!("{name}-chat"))
        .bind(created)
        .execute(&pool)
        .await
        .unwrap();
    }

    // resolve 热路径必须稳定选最早创建的 older（旧实现 LIMIT 1 无 ORDER 依赖物理顺序）
    use agent_memory_llm::provider::LlmProvider as _;
    let (provider, model) = registry.resolve(Purpose::Extract).await.unwrap();
    assert_eq!(
        provider.name(),
        "older",
        "多默认存量时按 created_at 确定性取最早"
    );
    assert_eq!(model, "older-chat");

    drop(container);
}

/// L10：占位主密钥下创建 provider → 响应必须带 warning（端到端）。
#[tokio::test]
async fn l10_placeholder_create_response_carries_warning() {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");

    // 占位密钥 app（"00"*32——与 main.rs 缺省回退同值）
    let state = AppState::new(pool)
        .with_admin_password(Some("test-admin-pw".into()))
        .with_master_key(Some("00".repeat(32)));
    let app = routes::router(state);

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
    let token = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/settings/llm/providers")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(valid_body("under-placeholder").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let warning = v["warning"].as_str().unwrap_or("");
    assert!(
        warning.contains("占位主密钥") && warning.contains("re-encrypt"),
        "创建响应必须带占位告警: {v:?}"
    );

    drop(container);
}

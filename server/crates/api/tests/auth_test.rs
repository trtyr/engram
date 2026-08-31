//! 鉴权行为集成测试：401/403 语义（Phase 1 出口标准）。
//! 全链路：真 PG（testcontainers）+ 完整 router + Bearer 中间件。

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
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
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
                    r#"{{"name":"t","scopes":{scopes_json}}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "签发 key 应成功");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    v["key"].as_str().unwrap().to_string()
}

async fn get_status(app: &Router, auth: Option<&str>, uri: &str) -> StatusCode {
    let mut req = Request::builder().method("GET").uri(uri);
    if let Some(a) = auth {
        req = req.header("authorization", format!("Bearer {a}"));
    }
    let resp = app
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    resp.status()
}

#[tokio::test]
async fn auth_401_403_matrix() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    // 401：无凭证 / 坏凭证
    assert_eq!(
        get_status(&app, None, "/jobs").await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get_status(&app, Some("amk_bogus"), "/jobs").await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get_status(&app, Some("ams_bogus"), "/jobs").await,
        StatusCode::UNAUTHORIZED
    );

    // 登录错误密码 → 401
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"password":"wrong"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 管理员全通
    assert_eq!(
        get_status(&app, Some(&admin), "/jobs").await,
        StatusCode::OK
    );
    assert_eq!(
        get_status(&app, Some(&admin), "/settings/api-keys").await,
        StatusCode::OK
    );

    // API key：jobs 可读（任意 scope），管理端点 403
    let key_full = create_key(&app, &admin, &["memory", "knowledge", "wiki", "codegraph"]).await;
    let key_mem = create_key(&app, &admin, &["memory"]).await;
    assert_eq!(
        get_status(&app, Some(&key_full), "/jobs").await,
        StatusCode::OK
    );
    assert_eq!(
        get_status(&app, Some(&key_full), "/settings/api-keys").await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get_status(&app, Some(&key_mem), "/llm/usage").await,
        StatusCode::FORBIDDEN
    );

    // key 明文永不回显
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/settings/api-keys")
                .header("authorization", format!("Bearer {admin}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(
        !text.contains(&key_full),
        "完整 API key 不得出现在列表响应中（key_prefix 展示除外，它是截断的）"
    );
}

/// amk_ key + memory scope 全旅程（AI 消费者契约面）：
/// 写会话→列表→原子→实体→检索→context→蒸馏→嵌入状态全通；跨域 403。
#[tokio::test]
async fn api_key_memory_journey() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;
    let key = create_key(&app, &admin, &["memory"]).await;

    let send = |app: &Router, method: &str, uri: &str, body: Option<&str>| {
        let mut b = Request::builder().method(method).uri(uri);
        if body.is_some() {
            b = b.header("content-type", "application/json");
        }
        app.clone().oneshot(
            b.header("authorization", format!("Bearer {key}"))
                .body(Body::from(body.unwrap_or_default().to_string()))
                .unwrap(),
        )
    };

    // 写会话（turns 契约 + distill off 防触发无 provider 蒸馏）
    let resp = send(
        &app,
        "POST",
        "/memory/sessions",
        Some(r#"{"agent":"ai-key","turns":[{"speaker":"user","text":"张三生日是 3 月 5 日"}],"distill":"off"}"#),
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "key 应能写会话");

    // 读路径全家通
    for uri in [
        "/memory/sessions?limit=5",
        "/memory/atoms?limit=5",
        "/memory/scenarios?limit=5",
        "/memory/persona",
        "/memory/entities",
        "/memory/entities/graph",
        "/memory/embeddings/status",
    ] {
        let resp = send(&app, "GET", uri, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "GET {uri} 应 200");
    }

    // 写路径：原子 + 实体 + 检索 + context + 蒸馏
    let resp = send(
        &app,
        "POST",
        "/memory/atoms",
        Some(r#"{"kind":"fact","content":"张三的生日是 3 月 5 日","confidence":0.9}"#),
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "key 应能直写原子");
    let resp = send(
        &app,
        "POST",
        "/memory/entities",
        Some(r#"{"name":"张三","kind":"person","summary":""}"#),
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "key 应能建实体");
    let resp = send(
        &app,
        "POST",
        "/memory/search",
        Some(r#"{"query":"张三 生日","limit":5}"#),
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "key 应能检索");
    let resp = send(&app, "GET", "/memory/context?query=张三", None)
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "key 应能取 context_pack");
    let resp = send(&app, "POST", "/memory/distill", Some("{}"))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::ACCEPTED,
        "key 应能触发蒸馏（202）"
    );

    // 跨域越权：memory-only key 摸别的域必须 403
    for uri in ["/knowledge/documents", "/wiki/pages", "/codegraph/projects"] {
        let resp = send(&app, "GET", uri, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "GET {uri} 应 403");
    }
}

/// 已撤销的 amk_ key → 401 文案区分「已撤销」与「不存在」（2026-08-31 测试方实测痛点：
/// 被误撤销的 key 与抄错的 key 报同一种错，排查靠猜）。
#[tokio::test]
async fn revoked_api_key_gets_distinct_401() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    // 建 key（拿 id + 明文）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/settings/api-keys")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {admin}"))
                .body(Body::from(r#"{"name":"rv-test","scopes":["memory"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let id = body["id"].as_str().unwrap().to_string();
    let key = body["key"].as_str().unwrap().to_string();

    // 撤销（204）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/settings/api-keys/{id}/revoke"))
                .header("authorization", format!("Bearer {admin}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // 撤销 key → 401 且文案含「撤销」
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/jobs")
                .header("authorization", format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let text = String::from_utf8_lossy(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .to_string();
    assert!(
        text.contains("撤销"),
        "撤销 key 的 401 应有区分文案，实得 {text}"
    );

    // 不存在的 key → 401 通用文案（不含「撤销」）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/jobs")
                .header("authorization", "Bearer amk_bogus")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let text = String::from_utf8_lossy(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .to_string();
    assert!(
        !text.contains("撤销"),
        "不存在 key 应走通用文案，实得 {text}"
    );
}

/// llm scope 的 amk_ key 可管 provider/路由/连通测试/用量（2026-08-31 方向：
/// 除 amk_ 管理外平台能力全暴露给 AI）；api-keys 管理与 re-encrypt 仍仅管理员。
#[tokio::test]
async fn llm_scope_key_manages_providers() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;
    let key = create_key(&app, &admin, &["llm"]).await;
    let mem_key = create_key(&app, &admin, &["memory"]).await;

    let send = |app: &Router, method: &str, uri: &str, auth: String, body: Option<&str>| {
        let mut b = Request::builder().method(method).uri(uri);
        if body.is_some() {
            b = b.header("content-type", "application/json");
        }
        app.clone().oneshot(
            b.header("authorization", format!("Bearer {auth}"))
                .body(Body::from(body.unwrap_or_default().to_string()))
                .unwrap(),
        )
    };

    // llm key：provider 列表 + 注册（base_url 不带 /v1——服务端自拼）+ 连通测试 + 路由读写
    let resp = send(&app, "GET", "/settings/llm/providers", key.clone(), None)
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "llm key 应能列 providers");
    let resp = send(
        &app,
        "POST",
        "/settings/llm/providers",
        key.clone(),
        Some(
            r#"{"name":"t","base_url":"https://gw.example.com","api_key":"sk-x","models":[{"id":"m","capabilities":["chat"]}],"is_default":false}"#,
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "llm key 应能注册 provider"
    );
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let pid = body["id"].as_str().unwrap();
    let resp = send(
        &app,
        "POST",
        &format!("/settings/llm/providers/{pid}/test"),
        key.clone(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "llm key 应能测连通");
    let resp = send(
        &app,
        "PUT",
        "/settings/llm/routing",
        key.clone(),
        Some(r#"{"extract":[{"provider":"t","model":"m"}]}"#),
    )
    .await
    .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NO_CONTENT,
        "llm key 应能配路由（204）"
    );
    // 清理（路由表清空后删 provider）
    let _ = send(
        &app,
        "PUT",
        "/settings/llm/routing",
        key.clone(),
        Some("{}"),
    )
    .await;
    let resp = send(
        &app,
        "DELETE",
        &format!("/settings/llm/providers/{pid}"),
        key.clone(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NO_CONTENT,
        "llm key 应能删 provider"
    );

    // llm key 越界：amk_ 管理与主密钥操作仍仅管理员
    let resp = send(&app, "GET", "/settings/api-keys", key.clone(), None)
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "llm key 摸 api-keys 应 403"
    );
    let resp = send(
        &app,
        "POST",
        "/settings/llm/providers/re-encrypt",
        key.clone(),
        Some(r#"{"old_master_key":"abab"}"#),
    )
    .await
    .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "llm key 摸 re-encrypt 应 403"
    );

    // memory key 越界：provider 面板 403
    let resp = send(&app, "GET", "/settings/llm/providers", mem_key, None)
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "memory key 摸 providers 应 403"
    );
}

/// erase 分权（2026-08-31 用户批准）：擦除不可逆，memory-only key 不得单独行使——
/// 需 memory + erase 双 scope（admin 全权）。
/// F1 deep purge：erase scope × confirm 短语双因子 + 计数 + 审计 job 行。
#[tokio::test]
async fn deep_purge_requires_scope_and_confirm_phrase() {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");
    let state = AppState::new(pool.clone())
        .with_admin_password(Some("test-admin-pw".into()))
        .with_master_key(Some("ab".repeat(32)));
    let app = routes::router(state.clone());
    let admin = login_token(&app).await;

    // 造数据：会话（经服务层）
    let svc = agent_memory_core::MemoryService::new(
        state.pool.clone(),
        agent_memory_llm::ProviderRegistry::new(
            state.pool.clone(),
            agent_memory_llm::KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
        ),
    );
    svc.write_session(
        "t",
        serde_json::json!([{"speaker":"user","text":"x"}]),
        "off",
    )
    .await
    .unwrap();

    let post = |key: &str, body: &str| {
        let app = app.clone();
        let key = key.to_string();
        let body = body.to_string();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/memory/purge")
                        .header("content-type", "application/json")
                        .header("authorization", format!("Bearer {key}"))
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            let code = resp.status();
            let b = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            (code, String::from_utf8_lossy(&b).to_string())
        }
    };

    // 无 erase scope 的 memory key → 403
    let mem_key = create_key(&app, &admin, &["memory"]).await;
    let r = post(&mem_key, r#"{"deep":true,"confirm":"清空记忆库"}"#).await;
    assert_eq!(r.0, StatusCode::FORBIDDEN, "无 erase scope 应 403：{}", r.1);

    // 有 erase scope 但无 confirm → 400
    let erase_key = create_key(&app, &admin, &["memory", "erase"]).await;
    let r = post(&erase_key, r#"{"deep":true}"#).await;
    assert_eq!(r.0, StatusCode::BAD_REQUEST, "缺确认短语应 400：{}", r.1);
    assert!(r.1.contains("确认短语"));

    // 错短语 → 400
    let r = post(&erase_key, r#"{"deep":true,"confirm":"随便"}"#).await;
    assert_eq!(r.0, StatusCode::BAD_REQUEST, "错短语应 400：{}", r.1);

    // 正确双因子 → 200 五计数 + 审计 job 行
    let r = post(&erase_key, r#"{"deep":true,"confirm":"清空记忆库"}"#).await;
    assert_eq!(r.0, StatusCode::OK, "双因子应 200：{}", r.1);
    assert!(r.1.contains("\"sessions\""), "计数响应：{}", r.1);
    let audit: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE kind = 'purge_memory' AND status = 'succeeded' AND payload->>'confirm' = '清空记忆库'",
    )
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(audit, 1, "审计 job 行应存在（短语+来源+计数）");

    // 清空后四层全零
    let n: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM raw_sessions) + (SELECT count(*) FROM atoms) + (SELECT count(*) FROM scenarios)",
    )
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(n, 0, "deep 清空后应真空");
}

// ============ 编辑分权（2026-08-31 编辑能力批一） ============

// AI 禁改改写语义（content/kind/confidence）→ 403 教正确姿势；
// admin 改 content → 200 + atom_revisions 留痕 + edit_atom 审计行；sensitive 对 AI 开放。
#[tokio::test]
async fn edit_split_atom_rewrite_user_only() {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");
    let state = agent_memory_api::state::AppState::new(pool.clone())
        .with_admin_password(Some("test-admin-pw".into()))
        .with_master_key(Some("ab".repeat(32)));
    let app = agent_memory_api::routes::router(state.clone());
    let admin = login_token(&app).await;

    let svc = agent_memory_core::MemoryService::new(
        pool.clone(),
        agent_memory_llm::ProviderRegistry::new(
            pool.clone(),
            agent_memory_llm::KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
        ),
    );
    let atom = svc
        .create_atom("fact", "用户不能吃辣", 0.9, None, None, false)
        .await
        .unwrap();

    let mem_key = create_key(&app, &admin, &["memory"]).await;
    let patch = |key: &str, body: String| {
        let app = app.clone();
        let key = key.to_string();
        async move {
            let resp = app
                .oneshot(
                    axum::http::Request::builder()
                        .method("PATCH")
                        .uri(format!("/memory/atoms/{}", atom.id))
                        .header("content-type", "application/json")
                        .header("authorization", format!("Bearer {key}"))
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            let code = resp.status();
            let b = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            (code, String::from_utf8_lossy(&b).to_string())
        }
    };

    // AI 改 content → 403 + 教 correction
    let (code, body) = patch(&mem_key, r#"{"content":"用户对辣过敏"}"#.into()).await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("correction"), "文案应教正确姿势：{body}");

    // AI 改 confidence → 同样 403
    let (code, _) = patch(&mem_key, r#"{"confidence":0.5}"#.into()).await;
    assert_eq!(code, StatusCode::FORBIDDEN);

    // AI 标 sensitive → 开放（保护语义）
    let (code, _) = patch(&mem_key, r#"{"sensitive":true}"#.into()).await;
    assert_eq!(code, StatusCode::OK);

    // 用户改 content → 200 + revision + 审计
    let (code, body) = patch(&admin, r#"{"content":"用户不能吃辣（体质原因）"}"#.into()).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let rev: i64 = sqlx::query_scalar("SELECT count(*) FROM atom_revisions WHERE atom_id = $1")
        .bind(atom.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rev, 1, "改写应留痕一条 revision");
    let by: String =
        sqlx::query_scalar("SELECT edited_by FROM atom_revisions WHERE atom_id = $1 LIMIT 1")
            .bind(atom.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(by, "admin");
    let audit: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE kind = 'edit_atom' AND payload->>'by' = 'admin'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audit, 1, "审计行应存在");
}

// persona 编辑/回滚、实体名/摘要：全部仅用户（amk_ 403）。
#[tokio::test]
async fn edit_split_persona_and_entity_user_only() {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");
    let state = agent_memory_api::state::AppState::new(pool.clone())
        .with_admin_password(Some("test-admin-pw".into()))
        .with_master_key(Some("ab".repeat(32)));
    let app = agent_memory_api::routes::router(state);
    let admin = login_token(&app).await;
    let mem_key = create_key(&app, &admin, &["memory"]).await;

    // amk_ PATCH persona → 403
    let resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("PATCH")
                .uri("/memory/persona")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {mem_key}"))
                .body(axum::body::Body::from(
                    r#"{"aspect":"constraints","content":"手编内容"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // amk_ rollback → 403；amk_ entity PATCH → 403
    for (method, uri, body) in [
        (
            "POST",
            "/memory/persona/rollback",
            r#"{"aspect":"constraints","to_version":1}"#.to_string(),
        ),
        (
            "PATCH",
            "/memory/entities/00000000-0000-0000-0000-000000000000",
            r#"{"summary":"x"}"#.to_string(),
        ),
    ] {
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {mem_key}"))
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method} {uri}");
    }

    // admin 编辑 persona → v1 手编钉住
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("PATCH")
                .uri("/memory/persona")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {admin}"))
                .body(axum::body::Body::from(
                    r#"{"aspect":"constraints","content":"用户手编的约束"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let (me, version): (bool, i32) = sqlx::query_as(
        "SELECT manually_edited, version FROM persona_aspects WHERE aspect = 'constraints' ORDER BY version DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(version, 1);
    assert!(me, "手编应置 manually_edited");
}

/// erase_session 分权：memory-only key 403 / memory+erase key 204（admin 全权）。
/// 需 memory + erase 双 scope（admin 全权）。
#[tokio::test]
async fn erase_requires_dedicated_scope() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;
    let mem_key = create_key(&app, &admin, &["memory"]).await;
    let erase_key = create_key(&app, &admin, &["memory", "erase"]).await;

    let send = |app: &Router, method: &str, uri: &str, auth: String, body: Option<&str>| {
        let mut b = Request::builder().method(method).uri(uri);
        if body.is_some() {
            b = b.header("content-type", "application/json");
        }
        app.clone().oneshot(
            b.header("authorization", format!("Bearer {auth}"))
                .body(Body::from(body.unwrap_or_default().to_string()))
                .unwrap(),
        )
    };

    // 造一条会话
    let resp = send(
        &app,
        "POST",
        "/memory/sessions",
        admin.clone(),
        Some(r#"{"agent":"t","turns":[{"speaker":"user","text":"x"}],"distill":"off"}"#),
    )
    .await
    .unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let sid = body["id"].as_str().unwrap();

    // memory-only → 403（可读可写但不可毁）
    let resp = send(
        &app,
        "DELETE",
        &format!("/memory/sessions/{sid}"),
        mem_key,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "memory-only 擦除应 403"
    );

    // memory+erase → 204
    let resp = send(
        &app,
        "DELETE",
        &format!("/memory/sessions/{sid}"),
        erase_key,
        None,
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT, "双 scope 擦除应 204");
}

#[tokio::test]
async fn openapi_snapshot() {
    let (app, _pg) = app().await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // 端点集合快照：增删端点必须显式更新此清单
    let mut paths: Vec<String> = v["paths"]
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.to_string())
        .collect();
    paths.sort();
    let expected = [
        "/auth/login",
        "/health",
        "/jobs",
        "/jobs/{id}",
        "/jobs/{id}/events",
        "/jobs/{id}/revive",
        "/llm/usage",
        "/openapi.json"
            .rsplit('/')
            .next()
            .map(|_| "/openapi.json")
            .unwrap(), // 不在 paths 里，占位过滤
    ];
    let _ = expected;
    assert_eq!(
        paths,
        vec![
            "/auth/login",
            "/codegraph/projects",
            "/codegraph/projects/{id}",
            "/codegraph/projects/{id}/index",
            "/codegraph/projects/{id}/query",
            "/codegraph/projects/{id}/sync",
            "/health",
            "/jobs",
            "/jobs/{id}",
            "/jobs/{id}/events",
            "/jobs/{id}/revive",
            "/knowledge/documents",
            "/knowledge/documents/{id}",
            "/knowledge/documents/{id}/chunks",
            "/knowledge/documents/{id}/re-embed",
            "/knowledge/search",
            "/knowledge/upload",
            "/llm/usage",
            "/memory/atoms",
            "/memory/atoms/{id}",
            "/memory/context",
            "/memory/distill",
            "/memory/embeddings/status",
            "/memory/entities",
            "/memory/entities/graph",
            "/memory/entities/{id}",
            "/memory/entities/{id}/atoms/{atom_id}",
            "/memory/entities/{id}/merge",
            "/memory/export",
            "/memory/persona",
            "/memory/persona/history",
            "/memory/persona/rollback",
            "/memory/purge",
            "/memory/reembed",
            "/memory/scenarios",
            "/memory/scenarios/{id}",
            "/memory/search",
            "/memory/sessions",
            "/memory/sessions/{id}",
            "/memory/sessions/{id}/append",
            "/memory/sessions/{id}/void",
            "/ready",
            "/search",
            "/settings/api-keys",
            "/settings/api-keys/{id}/revoke",
            "/settings/llm/providers",
            "/settings/llm/providers/re-encrypt",
            "/settings/llm/providers/{id}",
            "/settings/llm/providers/{id}/test",
            "/settings/llm/routing",
            "/wiki/graph",
            "/wiki/ingest",
            "/wiki/insights",
            "/wiki/insights/dismiss",
            "/wiki/insights/reset",
            "/wiki/lint",
            "/wiki/pages",
            "/wiki/pages/{slug}",
            "/wiki/proposals/apply",
            "/wiki/purpose",
            "/wiki/queries/archive",
            "/wiki/reviews",
            "/wiki/reviews/{id}/resolve",
            "/wiki/search",
            "/wiki/sources",
            "/wiki/sources/{id}",
        ],
        "API 端点集合发生变化时必须同步更新快照"
    );
}

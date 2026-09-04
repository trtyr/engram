//! 鉴权行为集成测试：401/403 语义（Phase 1 出口标准）。
//! 全链路：真 PG（testcontainers）+ 完整 router + Bearer 中间件。

mod support;

use engram_api::routes;
use engram_api::state::AppState;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;
use uuid::Uuid;

async fn app() -> (Router, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool)
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
    let key_full = create_key(&app, &admin, &["memory", "wiki", "codegraph"]).await;
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

    // 写路径：直写加工权已收回（atom-add/entity-add 403），AI 本职 = 写会话 + 蒸馏 + 检索
    let resp = send(
        &app,
        "POST",
        "/memory/atoms",
        Some(r#"{"kind":"fact","content":"张三的生日是 3 月 5 日","confidence":0.9}"#),
    )
    .await
    .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "AI 直写原子应 403（蒸馏代劳）"
    );
    let resp = send(
        &app,
        "POST",
        "/memory/entities",
        Some(r#"{"name":"张三","kind":"person","summary":""}"#),
    )
    .await
    .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "AI 建实体应 403（蒸馏代劳）"
    );
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
    for uri in ["/wiki/documents", "/wiki/pages", "/codegraph/projects"] {
        let resp = send(&app, "GET", uri, None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "GET {uri} 应 403");
    }
}

/// 删除的 amk_ key → 401 通用文案（物理删除不留记录，与「不存在」同路径）。
#[tokio::test]
async fn deleted_api_key_gets_generic_401() {
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

    // 删除的 key → 401 通用文案（物理删除后与「不存在」同路径，不再区分）
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
        !text.contains("撤销"),
        "删除的 key 应走通用文案，实得 {text}"
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
            r#"{"name":"t","base_url":"https://gw.example.com","api_key":"sk-test-key-123456","model_id":"m","capability":"chat","is_default":false}"#,
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
    engram_storage::run_migrations(&pool)
        .await
        .expect("迁移");
    let state = AppState::new(pool.clone())
        .with_admin_password(Some("test-admin-pw".into()))
        .with_master_key(Some("ab".repeat(32)));
    let app = routes::router(state.clone());
    let admin = login_token(&app).await;

    // 造数据：会话（经服务层）
    let svc = engram_core::MemoryService::new(
        state.pool.clone(),
        engram_llm::ProviderRegistry::new(
            state.pool.clone(),
            engram_llm::KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
        ),
    );
    svc.write_session(
        "t",
        serde_json::json!([{"speaker":"user","text":"x"}]),
        "off",
        false,
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

    // 无 erase scope 的 memory key → 403（gate 不变）
    let mem_key = create_key(&app, &admin, &["memory"]).await;
    let r = post(&mem_key, r#"{"deep":true,"confirm":"清空记忆库"}"#).await;
    assert_eq!(r.0, StatusCode::FORBIDDEN, "无 erase scope 应 403：{}", r.1);

    // SEC-D 收权（2026-09-03）：erase scope 的 key 打 deep → 403（deep 仅限管理员会话）
    let erase_key = create_key(&app, &admin, &["memory", "erase"]).await;
    let r = post(&erase_key, r#"{"deep":true,"confirm":"清空记忆库"}"#).await;
    assert_eq!(
        r.0,
        StatusCode::FORBIDDEN,
        "AI key 打 deep 应一律 403：{}",
        r.1
    );
    assert!(
        r.1.contains("仅限管理员"),
        "文案应指路 Settings 危险区：{}",
        r.1
    );

    // 收权只收 deep：erase key 的 agent 清场能力保留（AI 清自己的测试数据）
    let r = post(&erase_key, r#"{"agent":"no-such-agent"}"#).await;
    assert_eq!(
        r.0,
        StatusCode::OK,
        "agent 清场对 erase key 应保留：{}",
        r.1
    );

    // P-A：admin 打 deep + agent 组合 → 400（deep 无 agent 过滤语义，组合即误导）
    let r = post(
        &admin,
        r#"{"agent":"nonexistent-xyz","deep":true,"confirm":"清空记忆库"}"#,
    )
    .await;
    assert_eq!(r.0, StatusCode::BAD_REQUEST, "deep+agent 应 400：{}", r.1);
    assert!(
        r.1.contains("不接受 agent 参数"),
        "文案应点破语义陷阱：{}",
        r.1
    );
    // 400 后库必须原封不动（防线意义所在）
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM raw_sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(n >= 1, "被拒的 deep 不得有任何删除副作用");

    // admin 无 confirm → 400；错短语 → 400（短语防误操作，对 admin 同样生效）
    let r = post(&admin, r#"{"deep":true}"#).await;
    assert_eq!(r.0, StatusCode::BAD_REQUEST, "缺确认短语应 400：{}", r.1);
    assert!(r.1.contains("确认短语"));
    let r = post(&admin, r#"{"deep":true,"confirm":"随便"}"#).await;
    assert_eq!(r.0, StatusCode::BAD_REQUEST, "错短语应 400：{}", r.1);

    // admin 正确短语 → 200 armed（P-C 两阶段：5 分钟冷却，非即时执行）
    let r = post(&admin, r#"{"deep":true,"confirm":"清空记忆库"}"#).await;
    assert_eq!(r.0, StatusCode::OK, "admin 双条件应 200 armed：{}", r.1);
    assert!(r.1.contains("armed"), "阶段一响应：{}", r.1);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM raw_sessions")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "armed 不执行（冷却窗口 = 后悔药）");

    // 阶段二：token 立即执行 → 五计数 + job succeeded（payload 带 confirm/executed_by = 审计链）
    let job_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM jobs WHERE kind = 'deep_purge' AND status = 'pending' LIMIT 1",
    )
    .fetch_one(&state.pool)
    .await
    .unwrap();
    let body = format!(
        r#"{{"deep":true,"confirm":"清空记忆库","token":"{}"}}"#,
        job_id
    );
    let r = post(&admin, &body).await;
    assert_eq!(r.0, StatusCode::OK, "token 执行应 200：{}", r.1);
    assert!(r.1.contains("sessions"), "计数响应：{}", r.1);
    let audit: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE id = $1 AND status = 'succeeded' AND payload->>'confirm' = '清空记忆库' AND payload->>'executed_by' = 'admin'",
    )
    .bind(job_id)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(audit, 1, "审计链应完整（短语+执行者+计数）");

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
    engram_storage::run_migrations(&pool)
        .await
        .expect("迁移");
    let state = engram_api::state::AppState::new(pool.clone())
        .with_admin_password(Some("test-admin-pw".into()))
        .with_master_key(Some("ab".repeat(32)));
    let app = engram_api::routes::router(state.clone());
    let admin = login_token(&app).await;

    let svc = engram_core::MemoryService::new(
        pool.clone(),
        engram_llm::ProviderRegistry::new(
            pool.clone(),
            engram_llm::KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
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

// 权限收窄：AI 直写加工权收回（atom-add/entity-add/relation-add/attach/superseded-by 403，用户直写不变）。
#[tokio::test]
async fn ai_direct_write_revoked() {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool)
        .await
        .expect("迁移");
    let state = engram_api::state::AppState::new(pool.clone())
        .with_admin_password(Some("test-admin-pw".into()))
        .with_master_key(Some("ab".repeat(32)));
    let app = engram_api::routes::router(state.clone());
    let admin = login_token(&app).await;

    let svc = engram_core::MemoryService::new(
        pool.clone(),
        engram_llm::ProviderRegistry::new(
            pool.clone(),
            engram_llm::KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
        ),
    );
    // 预置数据：一个原子 + 两个实体（走 svc 直造，绕过 handler 的收窄检查）
    let atom = svc
        .create_atom("fact", "测试原子", 0.9, None, None, false)
        .await
        .unwrap();
    let e1 = svc.create_entity("张三", "person", "").await.unwrap();
    let e2 = svc.create_entity("李四", "person", "").await.unwrap();

    let mem_key = create_key(&app, &admin, &["memory"]).await;

    let call = |key: &str, method: &str, uri: String, body: String| {
        let app = app.clone();
        let key = key.to_string();
        let method = method.to_string();
        async move {
            let resp = app
                .oneshot(
                    axum::http::Request::builder()
                        .method(method.as_str())
                        .uri(uri)
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

    let atom_count: i64 = sqlx::query_scalar("SELECT count(*) FROM atoms WHERE status = 'active'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let entity_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM entities WHERE merged_into IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    let rel_count: i64 = sqlx::query_scalar("SELECT count(*) FROM entity_relations")
        .fetch_one(&pool)
        .await
        .unwrap();

    // T1.1.1 AI 直写原子 → 403 + 教学文案
    let (code, body) = call(
        &mem_key,
        "POST",
        "/memory/atoms".into(),
        r#"{"kind":"fact","content":"T1.1.1","confidence":0.9}"#.into(),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("session-write"), "文案应教写会话：{body}");

    // T1.1.2 AI 直建实体 → 403
    let (code, body) = call(
        &mem_key,
        "POST",
        "/memory/entities".into(),
        r#"{"name":"T1.1.2","kind":"person","summary":""}"#.into(),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("蒸馏"), "{body}");

    // T1.1.3 AI 直建关系 → 403
    let (code, body) = call(
        &mem_key,
        "POST",
        format!("/memory/entities/{}/relations", e1.id),
        format!(r#"{{"to_id":"{}","rel_type":"related_to"}}"#, e2.id),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");

    // T1.1.4 AI 挂原子 → 403
    let (code, _) = call(
        &mem_key,
        "POST",
        format!("/memory/entities/{}/atoms/{}", e1.id, atom.id),
        "{}".into(),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);

    // T1.1.5 AI 手动取代链 → 403
    let (code, body) = call(
        &mem_key,
        "PATCH",
        format!("/memory/atoms/{}", atom.id),
        r#"{"superseded_by":"00000000-0000-0000-0000-000000000001"}"#.into(),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("correction"), "{body}");

    // T1.1.6 库原封不动
    let atom_after: i64 = sqlx::query_scalar("SELECT count(*) FROM atoms WHERE status = 'active'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let entity_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM entities WHERE merged_into IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    let rel_after: i64 = sqlx::query_scalar("SELECT count(*) FROM entity_relations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(atom_after, atom_count, "原子数应原封不动");
    assert_eq!(entity_after, entity_count, "实体数应原封不动");
    assert_eq!(rel_after, rel_count, "关系数应原封不动");

    // T1.3 用户（admin）直写原子 → 201，编辑权不变
    let (code, _) = call(
        &admin,
        "POST",
        "/memory/atoms".into(),
        r#"{"kind":"fact","content":"用户直写仍可用","confidence":0.9}"#.into(),
    )
    .await;
    assert_eq!(code, StatusCode::CREATED);
}

// persona 编辑/回滚、实体名/摘要：全部仅用户（amk_ 403）。
#[tokio::test]
async fn edit_split_persona_and_entity_user_only() {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool)
        .await
        .expect("迁移");
    let state = engram_api::state::AppState::new(pool.clone())
        .with_admin_password(Some("test-admin-pw".into()))
        .with_master_key(Some("ab".repeat(32)));
    let app = engram_api::routes::router(state);
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

/// 权限收窄补充（2026-09-02）：删实体/摘原子/删关系与擦除会话同级——memory-only key 403。
#[tokio::test]
async fn delete_endpoints_require_erase_scope() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;
    let mem_key = create_key(&app, &admin, &["memory"]).await;
    let erase_key = create_key(&app, &admin, &["memory", "erase"]).await;

    let send = |app: &Router, method: &str, uri: &str, auth: &str, body: Option<&str>| {
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
    let id_of = |resp: axum::response::Response| async {
        let v: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        v["id"].as_str().unwrap().to_string()
    };

    // 造两个实体（admin，绕过 AI 直写收回）
    let a = id_of(
        send(
            &app,
            "POST",
            "/memory/entities",
            &admin,
            Some(r#"{"name":"删测A","kind":"person","summary":""}"#),
        )
        .await
        .unwrap(),
    )
    .await;
    let b = id_of(
        send(
            &app,
            "POST",
            "/memory/entities",
            &admin,
            Some(r#"{"name":"删测B","kind":"topic","summary":""}"#),
        )
        .await
        .unwrap(),
    )
    .await;

    // 造关系（admin）
    let rid = id_of(
        send(
            &app,
            "POST",
            &format!("/memory/entities/{a}/relations"),
            &admin,
            Some(&format!(r#"{{"to_id":"{b}","rel_type":"related_to"}}"#)),
        )
        .await
        .unwrap(),
    )
    .await;

    // ① 删关系
    let resp = send(
        &app,
        "DELETE",
        &format!("/memory/entities/{a}/relations/{rid}"),
        &mem_key,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "memory-only 删关系应 403"
    );
    let resp = send(
        &app,
        "DELETE",
        &format!("/memory/entities/{a}/relations/{rid}"),
        &erase_key,
        None,
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT, "erase 删关系应 204");

    // ② 删实体
    let resp = send(
        &app,
        "DELETE",
        &format!("/memory/entities/{a}"),
        &mem_key,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "memory-only 删实体应 403"
    );
    let resp = send(
        &app,
        "DELETE",
        &format!("/memory/entities/{a}"),
        &erase_key,
        None,
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT, "erase 删实体应 204");

    // ③ 摘原子（先造原子 + 挂接）
    let atom_id = id_of(
        send(
            &app,
            "POST",
            "/memory/atoms",
            &admin,
            Some(r#"{"kind":"fact","content":"摘测原子","confidence":0.9}"#),
        )
        .await
        .unwrap(),
    )
    .await;
    let _ = send(
        &app,
        "POST",
        &format!("/memory/entities/{b}/atoms/{atom_id}"),
        &admin,
        None,
    )
    .await
    .unwrap();
    let resp = send(
        &app,
        "DELETE",
        &format!("/memory/entities/{b}/atoms/{atom_id}"),
        &mem_key,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "memory-only 摘原子应 403"
    );
    let resp = send(
        &app,
        "DELETE",
        &format!("/memory/entities/{b}/atoms/{atom_id}"),
        &erase_key,
        None,
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT, "erase 摘原子应 204");
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
            "/llm/usage",
            "/memory/atoms",
            "/memory/atoms/{id}",
            "/memory/atoms/{id}/revisions",
            "/memory/context",
            "/memory/distill",
            "/memory/embeddings/status",
            "/memory/entities",
            "/memory/entities/batch",
            "/memory/entities/export",
            "/memory/entities/graph",
            "/memory/entities/search",
            "/memory/entities/{id}",
            "/memory/entities/{id}/atoms/{atom_id}",
            "/memory/entities/{id}/merge",
            "/memory/entities/{id}/relations",
            "/memory/entities/{id}/relations/{rid}",
            "/memory/entities/{id}/revisions",
            "/memory/export",
            "/memory/persona",
            "/memory/persona/history",
            "/memory/persona/rollback",
            "/memory/purge",
            "/memory/reembed",
            "/memory/rhythm/heartbeat",
            "/memory/rhythm/status",
            "/memory/scenarios",
            "/memory/scenarios/{id}",
            "/memory/search",
            "/memory/sessions",
            "/memory/sessions/import",
            "/memory/sessions/{id}",
            "/memory/sessions/{id}/append",
            "/memory/sessions/{id}/void",
            "/memory/timeline",
            "/projects",
            "/projects/batch-delete",
            "/projects/types",
            "/projects/{id}",
            "/projects/{id}/docs",
            "/projects/{id}/docs/{doc_id}",
            "/projects/{id}/locations",
            "/projects/{id}/locations/{loc_id}",
            "/ready",
            "/search",
            "/settings/api-keys",
            "/settings/api-keys/batch-revoke",
            "/settings/api-keys/{id}/revoke",
            "/settings/llm/providers",
            "/settings/llm/providers/re-encrypt",
            "/settings/llm/providers/{id}",
            "/settings/llm/providers/{id}/test",
            "/settings/llm/routing",
            "/settings/llm/routing/suggest",
            "/wiki/documents",
            "/wiki/documents/search",
            "/wiki/documents/{id}",
            "/wiki/documents/{id}/chunks",
            "/wiki/documents/{id}/re-embed",
            "/wiki/graph",
            "/wiki/ingest",
            "/wiki/insights",
            "/wiki/insights/dismiss",
            "/wiki/insights/reset",
            "/wiki/lint",
            "/wiki/pages",
            "/wiki/pages/{slug}",
            "/wiki/proposals",
            "/wiki/proposals/apply",
            "/wiki/purpose",
            "/wiki/queries/archive",
            "/wiki/reviews",
            "/wiki/reviews/{id}/resolve",
            "/wiki/search",
            "/wiki/sources",
            "/wiki/sources/{id}",
            "/wiki/upload",
        ],
        "API 端点集合发生变化时必须同步更新快照"
    );
}

#[tokio::test]
async fn batch_revoke_api_keys_revokes_selected_only() {
    let (app, _pg) = app().await;
    let token = login_token(&app).await;

    // 建 3 把 key，拿 id
    let mut ids: Vec<String> = Vec::new();
    for i in 0..3 {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/settings/api-keys")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(format!(
                        r#"{{"name":"batch-{i}","scopes":["memory"]}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        ids.push(v["id"].as_str().unwrap().to_string());
    }

    // 批量吊销前 2 把
    let ids_json = serde_json::json!([ids[0], ids[1]]);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/settings/api-keys/batch-revoke")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(format!(r#"{{"ids":{ids_json}}}"#)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["revoked"], 2, "应吊销 2 把: {v:?}");

    // 列表验证：前 2 把已物理删除，只剩第 3 把
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/settings/api-keys")
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
    let batch_keys: Vec<&serde_json::Value> = list
        .as_array()
        .unwrap()
        .iter()
        .filter(|k| k["name"].as_str().unwrap_or("").starts_with("batch-"))
        .collect();
    assert_eq!(batch_keys.len(), 1, "前 2 把应物理删除，只剩第 3 把");

    // 空数组 → 400
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/settings/api-keys/batch-revoke")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"ids":[]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "空 ids 应 400");
}

#[tokio::test]
async fn routing_suggest_reports_no_provider() {
    let (app, _pg) = app().await;
    let token = login_token(&app).await;

    // 无供应商 → 400 带明确报错（AI 建议的正路径需真实 LLM，走 live 验证）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/settings/llm/routing/suggest")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "无供应商应 400");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        v["error"]["message"].as_str().unwrap().contains("供应商"),
        "报错应提示注册供应商: {v:?}"
    );
}

/// memory-rhythm 分权：cron 通道与心跳只能由 cron scope 的 key 走——防 AI 伪造
/// via:"cron" 审计行、伪造心跳掩盖 cron 失联。status 则 memory scope 可读（AI 健康观察线）。
#[tokio::test]
async fn cron_scope_gates_distill_channel_and_heartbeat() {
    let (app, _pg) = app().await;
    let token = login_token(&app).await;
    let mem_key = create_key(&app, &token, &["memory"]).await;
    let cron_key = create_key(&app, &token, &["memory", "cron"]).await;

    // 1. memory-only key 带 via:cron → 403（AI 不能标 cron）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/memory/distill")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {mem_key}"))
                .body(Body::from(r#"{"full":true,"via":"cron"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN, "AI 标 cron 应 403");

    // 2. cron key 带 via:cron → 202
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/memory/distill")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {cron_key}"))
                .body(Body::from(r#"{"full":true,"via":"cron"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::ACCEPTED,
        "cron key 标 cron 应 202"
    );

    // 3. memory-only key 发 heartbeat → 403（AI 不能伪造在役）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/memory/rhythm/heartbeat")
                .header("authorization", format!("Bearer {mem_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN, "AI 发心跳应 403");

    // 4. cron key 带 via=cron 发 heartbeat → 200（crontab 命令模板自带 via）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/memory/rhythm/heartbeat?via=cron")
                .header("authorization", format!("Bearer {cron_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "cron key + via=cron 心跳应 200"
    );

    // 4b. cron key 不带 via → 403（R-1 加固：scope 是软挡，via 是显式声明——
    //     即使管理员误签了含 cron 的 key 给 AI，误调用也过不了）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/memory/rhythm/heartbeat")
                .header("authorization", format!("Bearer {cron_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "不带 via=cron 的心跳应 403（防 AI 无声伪造 cron 在役）"
    );

    // 5. memory-only key 读 status → 200（AI 健康观察线保留）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/memory/rhythm/status")
                .header("authorization", format!("Bearer {mem_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "AI 读 status 应 200");
}

/// 提案聚合端点：一条 SQL 取每 job 最新一条提案事件（替代前端 N+1）。
#[tokio::test]
async fn wiki_proposals_aggregates_latest_per_job() {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool)
        .await
        .expect("迁移");

    let state = AppState::new(pool.clone())
        .with_admin_password(Some("test-admin-pw".into()))
        .with_master_key(Some("ab".repeat(32)));
    let app = routes::router(state);

    // 两个 wiki_generate job：j1 两条提案（取最新）+ 一条噪音事件；j2 一条提案
    let j1: Uuid = Uuid::now_v7();
    let j2: Uuid = Uuid::now_v7();
    for (id, key) in [(j1, "k1"), (j2, "k2")] {
        sqlx::query("INSERT INTO jobs (id, kind, status, idempotency_key) VALUES ($1, 'wiki_generate', 'succeeded', $2)")
            .bind(id)
            .bind(key)
            .execute(&pool)
            .await
            .unwrap();
    }
    for (job, msg, data) in [
        (
            j1,
            "生成提案：旧版",
            r#"{"page_slug":"a","proposal_content":"旧"}"#,
        ),
        (j1, "记录完成", r#"{}"#),
        (
            j1,
            "生成提案：新版",
            r#"{"page_slug":"a","proposal_content":"新"}"#,
        ),
        (
            j2,
            "生成提案：b 页",
            r#"{"page_slug":"b","proposal_content":"内容b"}"#,
        ),
    ] {
        sqlx::query("INSERT INTO job_events (job_id, message, data) VALUES ($1, $2, $3::jsonb)")
            .bind(job)
            .bind(msg)
            .bind(data)
            .execute(&pool)
            .await
            .unwrap();
    }

    let token = login_token(&app).await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/wiki/proposals")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "GET /wiki/proposals 应 200");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 2, "每 job 聚合一条，共 2 条");
    let j1_hits: Vec<&serde_json::Value> = arr
        .iter()
        .filter(|e| e["job_id"] == j1.to_string())
        .collect();
    assert_eq!(j1_hits.len(), 1, "j1 只取最新提案");
    assert_eq!(j1_hits[0]["message"], "生成提案：新版");
    assert_eq!(
        arr.iter().filter(|e| e["job_id"] == j2.to_string()).count(),
        1
    );
}

/// W-3（2026-09-04）：doc-search 空 query 三问 400——与 /search 空查询口径对齐，
/// 不再放行返回全量命中。纯空白（空格）同样拒绝。
#[tokio::test]
async fn empty_search_query_rejected() {
    let (app, container) = app().await;
    let token = login_token(&app).await;
    let key = create_key(&app, &token, &["wiki"]).await;

    for q in ["", "   "] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/wiki/documents/search")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {key}"))
                    .body(Body::from(serde_json::json!({"query": q}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "空 query（{q:?}）应 400，不再返回全量"
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            v["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("query 不能为空"),
            "错误文案应三问指路：{v}"
        );
    }
    drop(container);
}

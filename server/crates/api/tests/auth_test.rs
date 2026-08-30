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
            "/memory/persona",
            "/memory/persona/history",
            "/memory/persona/rollback",
            "/memory/reembed",
            "/memory/scenarios",
            "/memory/scenarios/{id}",
            "/memory/search",
            "/memory/sessions",
            "/memory/sessions/{id}",
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

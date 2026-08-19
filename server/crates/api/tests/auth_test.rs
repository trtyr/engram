//! 鉴权行为集成测试：401/403 语义（Phase 1 出口标准）。
//! 全链路：真 PG（testcontainers）+ 完整 router + Bearer 中间件。

mod support;

use agent_memory_api::routes;
use agent_memory_api::state::AppState;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;

async fn app() -> Router {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");
    std::mem::forget(container); // 测试进程存活期间保容器

    let state = AppState::new(pool)
        .with_admin_password(Some("test-admin-pw".into()))
        .with_master_key(Some("ab".repeat(32)));
    routes::router(state)
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
    let app = app().await;
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

#[tokio::test]
async fn openapi_snapshot() {
    let app = app().await;
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
            "/health",
            "/jobs",
            "/jobs/{id}",
            "/jobs/{id}/events",
            "/jobs/{id}/revive",
            "/llm/usage",
            "/memory/atoms",
            "/memory/atoms/{id}",
            "/memory/context",
            "/memory/distill",
            "/memory/persona",
            "/memory/persona/history",
            "/memory/persona/rollback",
            "/memory/scenarios",
            "/memory/scenarios/{id}",
            "/memory/search",
            "/memory/sessions",
            "/memory/sessions/{id}",
            "/ready",
            "/settings/api-keys",
            "/settings/api-keys/{id}/revoke",
            "/settings/llm/providers",
            "/settings/llm/providers/{id}/test",
            "/settings/llm/routing",
        ],
        "API 端点集合发生变化时必须同步更新快照"
    );
}

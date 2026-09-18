//! migrate scope 集成测试（数据同步线 2026-09-18）：
//! /migrate/export 与 /migrate/import 准入放宽为「管理员会话 ∨ migrate scope 的 API key」，
//! /migrate/pull 保持 admin-only（远程拉取需向对端提供 admin 密码）。

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::util::ServiceExt;

mod support;

struct Ctx {
    app: axum::Router,
    admin_token: String,
    _pg: support::TestPg,
}

async fn setup() -> Ctx {
    let (app, pg) = support::app().await;
    let admin_token = support::login_token(&app).await;
    Ctx {
        app,
        admin_token,
        _pg: pg,
    }
}

/// 带 Authorization 的裸请求（key 与 admin token 通用）。
async fn send(
    app: &axum::Router,
    method: &str,
    path: &str,
    auth: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder().method(method).uri(path);
    let req = match body {
        Some(v) => builder
            .header("authorization", format!("Bearer {auth}"))
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => builder
            .header("authorization", format!("Bearer {auth}"))
            .body(Body::empty())
            .unwrap(),
    };
    let res = app.clone().oneshot(req).await.unwrap();
    let st = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (st, v)
}

/// 路径 1：migrate key 两端点全可用（导出 → 原包灌回，merge 幂等）。
#[tokio::test]
async fn migrate_key_can_export_and_import() {
    let ctx = setup().await;
    let key = support::create_key(&ctx.app, &ctx.admin_token, &["migrate"]).await;

    let (st, bundle) = send(&ctx.app, "GET", "/migrate/export", &key, None).await;
    assert_eq!(st, StatusCode::OK, "migrate key 导出应 200：{bundle}");
    assert_eq!(bundle["format"], "engram-transfer", "导出应是迁移包格式");

    let (st, report) = send(&ctx.app, "POST", "/migrate/import", &key, Some(bundle)).await;
    assert_eq!(st, StatusCode::OK, "migrate key 导入应 200：{report}");
    assert!(
        report.get("memory").is_some() && report.get("projects").is_some(),
        "导入报告应含分域计数（memory/projects/...）：{report}"
    );
}

/// 路径 2：无 migrate scope 的 key 两个端点都 403。
#[tokio::test]
async fn key_without_migrate_scope_forbidden() {
    let ctx = setup().await;
    let key = support::create_key(&ctx.app, &ctx.admin_token, &["projects"]).await;

    let (st, v) = send(&ctx.app, "GET", "/migrate/export", &key, None).await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "无 migrate scope 导出应 403：{v}"
    );

    let (st, v) = send(&ctx.app, "POST", "/migrate/import", &key, Some(json!({}))).await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "无 migrate scope 导入应 403：{v}"
    );
}

/// 路径 3：管理员会话照旧可用（向后兼容）。
#[tokio::test]
async fn admin_still_works() {
    let ctx = setup().await;
    let (st, bundle) = send(&ctx.app, "GET", "/migrate/export", &ctx.admin_token, None).await;
    assert_eq!(st, StatusCode::OK, "admin 导出照旧 200：{bundle}");
}

/// 迷你目标实例：/migrate/export 回最小 bundle、/migrate/import 回全零报告（sync 转发的对端）。
async fn spawn_target_app() -> String {
    let bundle = serde_json::json!({
        "format": "engram-transfer", "version": 1, "exported_at": "2026-01-01T00:00:00Z",
        "counts": {"sessions": 0, "atoms": 0, "scenarios": 0, "persona": 0, "entities": 0,
            "relations": 0, "skills": 0, "wiki_libraries": 0, "wiki_pages": 0, "projects": 0,
            "locations": 0, "docs": 0, "todos": 0, "kv_entries": 0, "wiki_promotions": 0},
        "memory": {"sessions": [], "atoms": [], "scenarios": [], "persona": [], "entities": [], "relations": []},
        "skills": [], "wiki": {"libraries": [], "pages": []},
        "projects": {"projects": [], "locations": [], "docs": []},
        "todos": [], "kv_entries": [], "wiki_promotions": []
    });
    let app = axum::Router::new()
        .route(
            "/migrate/export",
            axum::routing::get(move || async move { axum::Json(bundle) }),
        )
        .route(
            "/migrate/import",
            axum::routing::post(|| async {
                axum::Json(serde_json::json!({
                    "memory": {"sessions": {"imported": 0, "skipped": 0}}
                }))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// sync 端点全路径：migrate key 双向转发 + 权限 + 安全校验 + dry_run（2026-09-18）。
#[tokio::test]
async fn migrate_sync_endpoint_paths() {
    let ctx = setup().await;
    let key = support::create_key(&ctx.app, &ctx.admin_token, &["migrate"]).await;
    let target = spawn_target_app().await;

    // 1) push dry_run：migrate key 可用，不写目标，返回源 counts + dry_run 标记
    let (st, v) = send(
        &ctx.app,
        "POST",
        "/migrate/sync",
        &key,
        Some(json!({"target_url": target, "direction": "push", "token": "any", "dry_run": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "push dry_run 应 200：{v}");
    assert_eq!(v["dry_run"], true, "dry_run 标记应回显");
    assert!(v["source_counts"].is_object(), "应含源包分域计数：{v}");

    // 2) push 真跑：目标 import 被转发调用
    let (st, v) = send(
        &ctx.app,
        "POST",
        "/migrate/sync",
        &key,
        Some(json!({"target_url": target, "direction": "push", "token": "any"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "push 应 200：{v}");
    assert!(v["import_report"].is_object(), "应含目标导入报告：{v}");

    // 3) pull 真跑：从目标 export 拉包灌本地
    let (st, v) = send(
        &ctx.app,
        "POST",
        "/migrate/sync",
        &key,
        Some(json!({"target_url": target, "direction": "pull", "token": "any"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "pull 应 200：{v}");
    assert!(v["import_report"].is_object(), "应含本地导入报告：{v}");

    // 4) 无 migrate scope 的 key → 403
    let other = support::create_key(&ctx.app, &ctx.admin_token, &["projects"]).await;
    let (st, v) = send(
        &ctx.app,
        "POST",
        "/migrate/sync",
        &other,
        Some(json!({"target_url": target, "direction": "push", "token": "x"})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "无 scope 应 403：{v}");

    // 5) admin 照旧
    let (st, _) = send(
        &ctx.app,
        "POST",
        "/migrate/sync",
        &ctx.admin_token,
        Some(json!({"target_url": target, "direction": "push", "token": "x", "dry_run": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "admin 照旧 200");

    // 6) 非 loopback http 明文 → 400（可行动报错）
    let (st, v) = send(
        &ctx.app,
        "POST",
        "/migrate/sync",
        &key,
        Some(json!({"target_url": "http://cloud.example.com", "direction": "push", "token": "x"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "http 明文应 400：{v}");
    assert!(v.to_string().contains("TLS"), "报错应含 TLS 指引：{v}");

    // 7) direction 非法 → 400
    let (st, v) = send(
        &ctx.app,
        "POST",
        "/migrate/sync",
        &key,
        Some(json!({"target_url": target, "direction": "sideways", "token": "x"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "非法 direction 应 400：{v}");
}

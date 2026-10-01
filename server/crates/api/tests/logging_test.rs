//! P005-T001/T002：结构化日志落地 + request-id 贯穿回归——
//! PgLogLayer 事件入 channel、writer 批量落 `logs` 表、保留期清理、
//! request-id 中间件（生成/透传）与请求日志落表。

mod support;

use axum::body::Body;
use axum::http::Request;
use engram_api::logging;
use support::{app, connect_with_retry, connection_url, start_pgvector};
use tower::util::ServiceExt;
use tracing_subscriber::prelude::*;

#[tokio::test]
async fn pg_layer_events_land_in_logs_table() {
    let container = start_pgvector().await.expect("测试库");
    let url = connection_url(&container).await.unwrap();
    let pool = connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");

    let (layer, rx) = logging::channel();
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(doc = "测试文档", job_id = "01a0", "文档摄取失败");
        tracing::warn!(count = 3, "批次降级");
    });

    let writer = logging::spawn_log_writer(pool.clone(), rx);
    tokio::time::sleep(std::time::Duration::from_millis(900)).await;

    let (info_n, warn_n): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE level = 'INFO'), count(*) FILTER (WHERE level = 'WARN') FROM logs",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(info_n >= 1, "INFO 事件应落表: {info_n}");
    assert!(warn_n >= 1, "WARN 事件应落表: {warn_n}");

    let fields: serde_json::Value =
        sqlx::query_scalar("SELECT fields FROM logs WHERE message = '文档摄取失败' LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(fields["doc"], "测试文档", "结构化字段应保留: {fields}");

    writer.abort();
}

#[tokio::test]
async fn retention_deletes_old_keeps_recent() {
    let container = start_pgvector().await.expect("测试库");
    let url = connection_url(&container).await.unwrap();
    let pool = connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");

    sqlx::query(
        "INSERT INTO logs (ts, level, target, message) VALUES
         (now() - interval '31 days', 'INFO', 't', '旧 info'),
         (now() - interval '8 days', 'DEBUG', 't', '旧 debug'),
         (now() - interval '8 days', 'INFO', 't', '近期 info'),
         (now(), 'DEBUG', 't', '新 debug')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let deleted = logging::retain_old(&pool, 30, 7).await.unwrap();
    assert_eq!(deleted, 2, "应删 31 天前 info + 8 天前 debug");

    let msgs: Vec<String> = sqlx::query_scalar("SELECT message FROM logs ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(
        msgs.contains(&"近期 info".to_string()),
        "8 天前 info 应保留"
    );
    assert!(msgs.contains(&"新 debug".to_string()), "新 debug 应保留");
    assert!(!msgs.contains(&"旧 info".to_string()), "31 天前 info 应删");
    assert!(!msgs.contains(&"旧 debug".to_string()), "8 天前 debug 应删");
}

/// 无入站头 → 生成 uuid7 回带；有入站头 → 原值透传。
#[tokio::test]
async fn request_id_generated_and_echoed() {
    let (app, _pg) = app().await;

    let resp = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let generated = resp
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    assert!(
        generated.as_ref().is_some_and(|v| v.len() == 32),
        "无入站头应生成 uuid7 simple 并回带: {generated:?}"
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/")
                .header("x-request-id", "my-trace-123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let echoed = resp
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    assert_eq!(echoed.as_deref(), Some("my-trace-123"), "入站头应原值透传");
}

/// 请求日志落 logs 表：request_id + 方法/路由模板/状态/耗时齐全。
/// （set_default 独占全局 subscriber——本文件仅此一测用 layer 全局注册。）
#[tokio::test]
async fn request_log_lands_with_request_id() {
    let container = start_pgvector().await.expect("测试库");
    let url = connection_url(&container).await.unwrap();
    let pool = connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");

    let (layer, rx) = logging::channel();
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    let (app, _pg2) = app().await;
    let writer = logging::spawn_log_writer(pool.clone(), rx);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/")
                .header("x-request-id", "trace-abc-999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp.status().as_u16() >= 200);

    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    writer.abort();

    let row: Option<(String, String, i64)> = sqlx::query_as(
        "SELECT fields->>'http_method', fields->>'http_path', (fields->>'latency_ms')::bigint \
         FROM logs WHERE message = 'http 请求' AND request_id = 'trace-abc-999' LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    let (method, path, latency) = row.expect("请求日志应落表且带 request_id");
    assert_eq!(method, "GET");
    assert!(!path.is_empty(), "路由模板/路径应在");
    assert!(latency >= 0);
}

/// P005-T004：业务审计动作——登录/凭据/API key 的写删操作打点
/// （audit=true, action/target/actor 齐全，经 PgLogLayer 落 logs 表）。
#[tokio::test]
async fn audit_actions_land_in_logs() {
    let container = start_pgvector().await.expect("测试库");
    let url = connection_url(&container).await.unwrap();
    let pool = connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");

    let (layer, rx) = logging::channel();
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    let (app, _pg2) = app().await;
    let writer = logging::spawn_log_writer(pool.clone(), rx);

    // ① 登录（login handler 内部打点 auth.login）
    let token = support::login_token(&app).await;

    // ② 凭据写入 + 删除
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/credentials")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"name": "audit-test/key", "value": "v-审计"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::CREATED, "凭据写入");
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/credentials/audit-test%2Fkey")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK, "凭据删除");

    // ③ API key 创建 + 撤销
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/settings/api-keys")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"name": "audit-key", "scopes": ["wiki"]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::CREATED, "key 创建");
    let created: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let key_id = created["id"].as_str().expect("key id").to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/settings/api-keys/{key_id}/revoke"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        axum::http::StatusCode::NO_CONTENT,
        "key 撤销"
    );

    // 等 writer flush
    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    writer.abort();

    // 断言：6 条审计（login/credentials.put/credentials.delete/apikey.create/apikey.revoke）+ request_id 关联
    let actions: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT fields->>'action', fields->>'target', fields->>'actor' \
         FROM logs WHERE fields->>'audit' = 'true' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let actions_v: Vec<&str> = actions.iter().map(|(a, _, _)| a.as_str()).collect();
    for expected in [
        "auth.login",
        "credentials.put",
        "credentials.delete",
        "apikey.create",
        "apikey.revoke",
    ] {
        assert!(
            actions_v.contains(&expected),
            "审计缺 {expected}: {actions_v:?}"
        );
    }
    // target 断言（哪个对象）
    assert!(
        actions
            .iter()
            .any(|(a, t, _)| a == "credentials.put" && t.as_deref() == Some("audit-test/key")),
        "credentials.put 应带 target"
    );
    // actor 断言（谁）
    assert!(
        actions.iter().all(|(_, _, a)| a.is_some()),
        "全部审计应带 actor: {actions:?}"
    );
}

/// P005-T005：GET /logs 查询面——level/q/request_id 过滤 + 分页（admin 专属）。
#[tokio::test]
async fn logs_query_endpoint_filters_and_paginates() {
    let (app, pg2) = app().await;
    let url = connection_url(&pg2).await.unwrap();
    let pool = connect_with_retry(&url).await.expect("连接");

    // 造数：3 条不同 level/rid + 1 条审计
    sqlx::query(
        "INSERT INTO logs (level, target, message, fields, request_id) VALUES
         ('INFO', 't', 'alpha 消息', '{}', 'rid-1'),
         ('WARN', 't', 'beta 消息', '{}', 'rid-2'),
         ('ERROR', 't', 'gamma 错误', '{}', 'rid-1'),
         ('INFO', 't', '审计动作', '{\"audit\": \"true\"}', 'rid-3')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let token = support::login_token(&app).await;
    let get = |query: &str| {
        let app = app.clone();
        let token = token.clone();
        let uri = format!("/logs?{query}");
        async move {
            app.oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };

    // level 过滤
    let resp = get("level=ERROR").await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(v["logs"].as_array().unwrap().len(), 1);
    assert_eq!(v["logs"][0]["level"], "ERROR");

    // request_id 过滤
    let resp = get("request_id=rid-1").await;
    let v: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(v["logs"].as_array().unwrap().len(), 2, "rid-1 应两条");

    // q 模糊
    let resp = get("q=beta").await;
    let v: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(v["logs"].as_array().unwrap().len(), 1);

    // audit_only
    let resp = get("audit=true").await;
    let v: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(v["logs"].as_array().unwrap().len(), 1);
    assert_eq!(v["logs"][0]["message"], "审计动作");

    // 分页
    let resp = get("limit=2&offset=2").await;
    let v: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        v["logs"].as_array().unwrap().len(),
        2,
        "4 行 limit2 offset2 应剩 2"
    );

    // 非 admin 拒绝
    let key = support::create_key(&app, &token, &["wiki"]).await;
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/logs")
                .header("authorization", format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        axum::http::StatusCode::FORBIDDEN,
        "非 admin 应 403"
    );
}

/// P006-T003：error 级事件自动落 logs——source 链/category/request_id 三要素齐全，
/// internal_bug 类带 alert 告警标记（ErrorCategory::InternalBug 归因驱动）。
#[tokio::test]
async fn error_events_land_with_full_context() {
    let container = start_pgvector().await.expect("测试库");
    let url = connection_url(&container).await.unwrap();
    let pool = connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");

    let (layer, rx) = logging::channel();
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);
    let writer = logging::spawn_log_writer(pool.clone(), rx);

    let app_guard = logging::with_request_id("trace-err-1".to_string(), async {
        // error 级 + internal_bug 归因 + source 链（EngramError Display 形态）
        tracing::error!(
            category = "internal_bug",
            code = "INTERNAL-INCONSISTENT",
            source = "孤儿态: job dead + 文档 pending（根因: fetch Retryable 耗尽未落终态）",
            "内部状态不一致"
        );
    });
    app_guard.await;
    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    writer.abort();

    let row: Option<(serde_json::Value, String)> = sqlx::query_as(
        "SELECT fields, request_id FROM logs WHERE message = '内部状态不一致' AND level = 'ERROR' LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    let (fields, rid) = row.expect("error 事件应落表");
    assert_eq!(rid, "trace-err-1", "request_id 应贯穿");
    assert_eq!(fields["category"], "internal_bug");
    assert_eq!(fields["code"], "INTERNAL-INCONSISTENT");
    assert!(
        fields["source"].as_str().unwrap_or("").contains("根因"),
        "source 链应保留: {fields}"
    );
    assert_eq!(fields["alert"], true, "internal_bug 应升级告警标记");
}

//! P005-T001/T002：结构化日志落地 + request-id 贯穿回归——
//! PgLogLayer 事件入 channel、writer 批量落 `logs` 表、保留期清理、
//! request-id 中间件（生成/透传）与请求日志落表。

mod support;

use axum::body::Body;
use engram_api::logging;
use axum::http::Request;
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

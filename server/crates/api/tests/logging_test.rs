//! P005-T001：结构化日志落地回归——PgLogLayer 事件入 channel、writer 批量落 `logs` 表、
//! 保留期清理（info 30 天 / debug 7 天）删旧留新。层内零 IO（channel 满丢弃不反压）。

mod support;

use engram_api::logging;
use tracing_subscriber::prelude::*;
use support::{connect_with_retry, connection_url, start_pgvector};

#[tokio::test]
async fn pg_layer_events_land_in_logs_table() {
    let container = start_pgvector().await.expect("测试库");
    let url = connection_url(&container).await.unwrap();
    let pool = connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");

    // layer 注册（with_default 局部 subscriber）→ 事件进 channel
    let (layer, rx) = logging::channel();
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(doc = "测试文档", job_id = "01a0", "文档摄取失败");
        tracing::warn!(count = 3, "批次降级");
    });

    // writer 后台落表（flush tick 500ms）
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

    // fields 结构化落库
    let fields: serde_json::Value =
        sqlx::query_scalar("SELECT fields FROM logs WHERE message = '文档摄取失败' LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(fields["doc"], "测试文档", "结构化字段应保留: {fields}");

    writer.abort(); // 测试收尾（表随 TestPg 删库消失）
}

/// 保留期清理：31 天前 info 删、8 天前 debug 删、新行留。
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

    let msgs: Vec<String> =
        sqlx::query_scalar("SELECT message FROM logs ORDER BY id").fetch_all(&pool).await.unwrap();
    assert!(msgs.contains(&"近期 info".to_string()), "8 天前 info 应保留");
    assert!(msgs.contains(&"新 debug".to_string()), "新 debug 应保留");
    assert!(!msgs.contains(&"旧 info".to_string()), "31 天前 info 应删");
    assert!(!msgs.contains(&"旧 debug".to_string()), "8 天前 debug 应删");
}

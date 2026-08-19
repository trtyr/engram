//! 迁移执行。迁移文件在 `server/migrations/`（编译期嵌入）。

use sqlx::migrate::Migrator;
use sqlx::postgres::PgPool;

/// 嵌入的迁移器。schema 唯一定义处：`server/migrations/`。
pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// 执行全部未应用的迁移。幂等；并发启动安全（sqlx 内部有迁移锁）。
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}

/// 当前已应用的迁移版本（就绪检查用）。空库返回 None。
pub async fn current_version(pool: &PgPool) -> Result<Option<i64>, sqlx::Error> {
    let row: Option<(Option<i64>,)> = sqlx::query_as("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|(v,)| v))
}

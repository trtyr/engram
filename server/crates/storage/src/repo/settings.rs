//! settings KV 仓储（key/value JSON，updated_at 维护）。

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::PgPool;
use crate::error::StoreResult;

/// 读配置（行缺失 → None；调用方按缺省处理，不让配置损坏打死端点）。
pub async fn get_json<T: DeserializeOwned + Send + Unpin + 'static>(
    pool: &PgPool,
    key: &str,
) -> Option<T> {
    let row: Option<(sqlx::types::Json<T>,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = $1")
            .bind(key)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    row.map(|(j,)| j.0)
}

/// 写配置（KV upsert）。
pub async fn put_json<T: Serialize + Sync>(pool: &PgPool, key: &str, value: &T) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES ($1, $2)
         ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = now()",
    )
    .bind(key)
    .bind(sqlx::types::Json(value))
    .execute(pool)
    .await?;
    Ok(())
}

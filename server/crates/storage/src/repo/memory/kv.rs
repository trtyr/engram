//! `memory` 的实现切片（架构治理 2026-09-21：自 memory.rs 纯搬移，零行为变化）。

use super::*;

/// UPSERT：同 key 就地覆盖更新（可变状态不走取代链）。返回更新后的行。
pub async fn kv_upsert(
    pool: &PgPool,
    key: &str,
    value: &str,
    context: &str,
    tags: &[String],
    source: &str,
) -> StoreResult<KvEntryDto> {
    let row = sqlx::query_as::<_, KvEntryDto>(
        "INSERT INTO kv_entries (key, value, context, tags, source) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, context = EXCLUDED.context, \
           tags = EXCLUDED.tags, source = EXCLUDED.source, updated_at = now() \
         RETURNING *",
    )
    .bind(key)
    .bind(value)
    .bind(context)
    .bind(tags)
    .bind(source)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn kv_get(pool: &PgPool, key: &str) -> StoreResult<Option<KvEntryDto>> {
    sqlx::query_as::<_, KvEntryDto>("SELECT * FROM kv_entries WHERE key = $1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn kv_list(pool: &PgPool, limit: i64) -> StoreResult<Vec<KvEntryDto>> {
    Ok(sqlx::query_as::<_, KvEntryDto>(
        "SELECT * FROM kv_entries ORDER BY updated_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

/// 字面量直查（ILIKE 兜底通道——精确值不依赖分词）。
pub async fn kv_search_literal(
    pool: &PgPool,
    needle: &str,
    limit: i64,
) -> StoreResult<Vec<KvEntryDto>> {
    let pat = format!("%{}%", needle.replace('%', "\\%").replace('_', "\\_"));
    Ok(sqlx::query_as::<_, KvEntryDto>(
        "SELECT * FROM kv_entries WHERE key ILIKE $2 OR value ILIKE $2 OR context ILIKE $2 \
         ORDER BY updated_at DESC LIMIT $1",
    )
    .bind(limit)
    .bind(pat)
    .fetch_all(pool)
    .await?)
}

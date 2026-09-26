//! 凭据域仓库（credentials + credential_reads，0060）。
//! 值以 KeyCipher 加密后的字节落库（value_enc）；本仓库不感知加解密——加密在 core 服务层做。

use crate::error::{StoreError, StoreResult};
use crate::models::credential::{CredentialMetaDto, CredentialReadRow};
use sqlx::Row;
use uuid::Uuid;

type Row_ = CredentialMetaDto;

/// upsert（按 name 唯一，大小写不敏感）：存在则更新值/标记/说明并清零取用审计（值换了，旧痕作废）。
pub async fn upsert(
    pool: &sqlx::PgPool,
    id: Uuid,
    name: &str,
    value_enc: &[u8],
    sensitive: bool,
    description: &str,
    created_by: &str,
) -> StoreResult<Row_> {
    sqlx::query_as::<_, Row_>(
        "INSERT INTO credentials (id, name, value_enc, sensitive, description, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (lower(btrim(name))) DO UPDATE SET \
           value_enc = EXCLUDED.value_enc, \
           sensitive = EXCLUDED.sensitive, \
           description = EXCLUDED.description, \
           updated_at = now(), \
           last_read_at = NULL, \
           read_count = 0 \
         RETURNING id, name, sensitive, description, created_by, created_at, updated_at, \
                   last_read_at, read_count",
    )
    .bind(id)
    .bind(name)
    .bind(value_enc)
    .bind(sensitive)
    .bind(description)
    .bind(created_by)
    .fetch_one(pool)
    .await
    .map_err(StoreError::from)
}

/// 内部行（含加密值）——服务层解密用。
pub struct EncRow {
    pub id: Uuid,
    pub name: String,
    pub value_enc: Vec<u8>,
    pub sensitive: bool,
    pub description: String,
    pub last_read_at: Option<chrono::DateTime<chrono::Utc>>,
    pub read_count: i32,
}

/// 按名取加密行（大小写/首尾空白不敏感命中）。
pub async fn get_enc_by_name(pool: &sqlx::PgPool, name: &str) -> StoreResult<Option<EncRow>> {
    sqlx::query(
        "SELECT id, name, value_enc, sensitive, description, \
                last_read_at, read_count \
         FROM credentials WHERE lower(btrim(name)) = lower(btrim($1))",
    )
    .bind(name)
    .map(|r: sqlx::postgres::PgRow| EncRow {
        id: r.get("id"),
        name: r.get("name"),
        value_enc: r.get("value_enc"),
        sensitive: r.get("sensitive"),
        description: r.get("description"),
        last_read_at: r.get("last_read_at"),
        read_count: r.get("read_count"),
    })
    .fetch_optional(pool)
    .await
    .map_err(StoreError::from)
}

/// 元数据（不含值）。
pub async fn get_meta(pool: &sqlx::PgPool, id: Uuid) -> StoreResult<Option<Row_>> {
    sqlx::query_as::<_, Row_>(
        "SELECT id, name, sensitive, description, created_by, created_at, updated_at, \
                last_read_at, read_count \
         FROM credentials WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(StoreError::from)
}

/// 台账列表（不含值；按 updated_at 倒序）。
pub async fn list(pool: &sqlx::PgPool) -> StoreResult<Vec<Row_>> {
    sqlx::query_as::<_, Row_>(
        "SELECT id, name, sensitive, description, created_by, created_at, updated_at, \
                last_read_at, read_count \
         FROM credentials ORDER BY updated_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(StoreError::from)
}

/// 删除（级联清取用审计）。
pub async fn delete(pool: &sqlx::PgPool, name: &str) -> StoreResult<u64> {
    let r = sqlx::query("DELETE FROM credentials WHERE lower(btrim(name)) = lower(btrim($1))")
        .bind(name)
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

/// 记一笔取用审计 + 更新计数器（get 的副作用；与取用同事务语义由服务层保证）。
pub async fn record_read(
    pool: &sqlx::PgPool,
    credential_id: Uuid,
    reader: &str,
) -> StoreResult<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO credential_reads (id, credential_id, reader) VALUES ($1, $2, $3)")
        .bind(Uuid::now_v7())
        .bind(credential_id)
        .bind(reader)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE credentials SET read_count = read_count + 1, last_read_at = now() WHERE id = $1",
    )
    .bind(credential_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// 取用审计流水（最近在前，封顶 50）。
pub async fn list_reads(pool: &sqlx::PgPool, name: &str) -> StoreResult<Vec<CredentialReadRow>> {
    sqlx::query_as::<_, CredentialReadRow>(
        "SELECT r.id, r.credential_id, r.reader, r.read_at \
         FROM credential_reads r \
         JOIN credentials c ON c.id = r.credential_id \
         WHERE lower(btrim(c.name)) = lower(btrim($1)) \
         ORDER BY r.read_at DESC LIMIT 50",
    )
    .bind(name)
    .fetch_all(pool)
    .await
    .map_err(StoreError::from)
}

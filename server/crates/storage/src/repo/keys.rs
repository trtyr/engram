//! 鉴权凭证仓储：admin_sessions（opaque 会话）+ api_keys（scope 化机器凭证）。
//!
//! SQL 从 api/auth.rs 收口而来；token 生成/哈希/恒定时间比较等密码学语义
//! 留在调用方（api/mcp 层），这里只管存取。

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::PgPool;
use crate::error::StoreResult;
use crate::models::keys::ApiKeyRow;

// ---------- 管理员会话 ----------

pub async fn create_admin_session(
    pool: &PgPool,
    token_hash: &str,
    expires_at: DateTime<Utc>,
) -> StoreResult<()> {
    sqlx::query("INSERT INTO admin_sessions (token_hash, expires_at) VALUES ($1, $2)")
        .bind(token_hash)
        .bind(expires_at)
        .execute(pool)
        .await?;
    Ok(())
}

/// 校验管理员会话 token：返回过期时间（None = 不存在）。
pub async fn find_admin_session_expiry(
    pool: &PgPool,
    token_hash: &str,
) -> StoreResult<Option<DateTime<Utc>>> {
    let row: Option<(DateTime<Utc>,)> =
        sqlx::query_as("SELECT expires_at FROM admin_sessions WHERE token_hash = $1")
            .bind(token_hash)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(expires,)| expires))
}

/// 更新会话 last_used（失败不阻塞认证，调用方 best-effort）。
pub async fn touch_admin_session(pool: &PgPool, token_hash: &str) -> StoreResult<()> {
    sqlx::query("UPDATE admin_sessions SET last_used_at = now() WHERE token_hash = $1")
        .bind(token_hash)
        .execute(pool)
        .await?;
    Ok(())
}

// ---------- API key ----------

pub async fn insert_api_key(
    pool: &PgPool,
    id: Uuid,
    name: &str,
    key_hash: &str,
    key_prefix: &str,
    scopes: &[String],
    expires_at: Option<DateTime<Utc>>,
) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO api_keys (id, name, key_hash, key_prefix, scopes, expires_at) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(name)
    .bind(key_hash)
    .bind(key_prefix)
    .bind(sqlx::types::Json(scopes))
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// 按 hash 查有效 key 的返回：key_id / name / scopes / expires_at。
type ActiveApiKey = (
    Uuid,
    String,
    sqlx::types::Json<Vec<String>>,
    Option<DateTime<Utc>>,
);

/// 按 hash 查有效 key（已吊销的不算）。
/// 过期判断在调用方（401 要带具体到期时间，EN-62）。
pub async fn find_api_key_by_hash(
    pool: &PgPool,
    key_hash: &str,
) -> StoreResult<Option<(Uuid, String, Vec<String>, Option<DateTime<Utc>>)>> {
    let row: Option<ActiveApiKey> = sqlx::query_as(
        "SELECT id, name, scopes, expires_at FROM api_keys WHERE key_hash = $1 AND revoked_at IS NULL",
    )
    .bind(key_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id, name, scopes, expires_at)| (id, name, scopes.0, expires_at)))
}

/// 更新 key last_used（失败不阻塞认证，调用方 best-effort）。
pub async fn touch_api_key(pool: &PgPool, key_id: Uuid) -> StoreResult<()> {
    sqlx::query("UPDATE api_keys SET last_used_at = now() WHERE id = $1")
        .bind(key_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// key 列表（永不含完整 key / hash）。
pub async fn list_api_keys(pool: &PgPool) -> StoreResult<Vec<ApiKeyRow>> {
    let rows = sqlx::query_as::<_, ApiKeyRow>(
        "SELECT id, name, key_prefix, scopes, created_at, last_used_at, revoked_at, expires_at FROM api_keys ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 删除 API key（物理删除，不留记录）。返回生效行数。
pub async fn delete_api_key(pool: &PgPool, id: Uuid) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM api_keys WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// 批量删除 API key（物理删除）。返回实际删除数。
pub async fn delete_api_keys(pool: &PgPool, ids: &[Uuid]) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM api_keys WHERE id = ANY($1)")
        .bind(ids)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

// ---------- 管理员账号（单用户；0033） ----------

/// 账号行：(username, password_hash)。
pub async fn get_admin_account(pool: &PgPool) -> StoreResult<Option<(String, String)>> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT username, password_hash FROM admin_account WHERE id = 1")
            .fetch_optional(pool)
            .await?;
    Ok(row)
}

/// 创建/覆盖账号（单行 upsert）。
pub async fn upsert_admin_account(
    pool: &PgPool,
    username: &str,
    password_hash: &str,
) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO admin_account (id, username, password_hash, updated_at) \
         VALUES (1, $1, $2, now()) \
         ON CONFLICT (id) DO UPDATE SET username = $1, password_hash = $2, updated_at = now()",
    )
    .bind(username)
    .bind(password_hash)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------- 会话管理（列表 / 吊销） ----------

/// 会话行：(token_hash, created_at, expires_at, last_used_at)。
pub async fn list_admin_sessions(
    pool: &PgPool,
) -> StoreResult<Vec<(String, DateTime<Utc>, DateTime<Utc>, Option<DateTime<Utc>>)>> {
    let rows = sqlx::query_as(
        "SELECT token_hash, created_at, expires_at, last_used_at FROM admin_sessions \
         WHERE expires_at > now() ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 吊销指定会话。返回生效行数。
pub async fn delete_admin_session(pool: &PgPool, token_hash: &str) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM admin_sessions WHERE token_hash = $1")
        .bind(token_hash)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// 吊销除指定会话外的全部会话（改密码后/「登出其他设备」）。
pub async fn delete_other_admin_sessions(pool: &PgPool, keep_token_hash: &str) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM admin_sessions WHERE token_hash <> $1")
        .bind(keep_token_hash)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// 编辑已有 API key：名称/scope 部分更新（None 不动）。
/// scope 是安全边界——变更即时生效（bearer_auth 每请求查库），无需吊销重签。
pub async fn update_api_key(
    pool: &PgPool,
    id: Uuid,
    name: Option<&str>,
    scopes: Option<&[String]>,
    expires_at: Option<Option<DateTime<Utc>>>,
) -> StoreResult<u64> {
    // expires_at 外层 None=不改；Some(None)=改回永不过期；Some(Some(t))=设到期时间
    let res = sqlx::query(
        "UPDATE api_keys SET \
           name = COALESCE($2, name), \
           scopes = COALESCE($3, scopes), \
           expires_at = CASE WHEN $4::bool THEN $5 ELSE expires_at END \
         WHERE id = $1",
    )
    .bind(id)
    .bind(name)
    .bind(scopes.map(sqlx::types::Json))
    .bind(expires_at.is_some())
    .bind(expires_at.flatten())
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 按 id 查 key（编辑后回显用；永不含完整 key / hash）。
pub async fn get_api_key(pool: &PgPool, id: Uuid) -> StoreResult<Option<ApiKeyRow>> {
    let row: Option<ApiKeyRow> = sqlx::query_as(
        "SELECT id, name, key_prefix, scopes, created_at, last_used_at, revoked_at, expires_at \
         FROM api_keys WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

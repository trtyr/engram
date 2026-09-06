//! LLM provider 管理面仓储（llm_providers 表的注册/更新/删除/列举）。
//!
//! 运行时读取与路由表归 engram-llm（ProviderRegistry / PurposeRouter）；
//! 这里收口的是 Web 控制台管理面的 CRUD（SQL 从 api/llm_api.rs 收口而来）。
//! L1：重名 UNIQUE 冲突映射 StoreError::Conflict；L3：默认唯一性事务降级。

use uuid::Uuid;

use crate::PgPool;
use crate::error::{StoreError, StoreResult, is_unique_violation};

fn conflict(e: sqlx::Error) -> StoreError {
    if is_unique_violation(&e) {
        StoreError::Conflict("llm_providers.name".into())
    } else {
        StoreError::Sql(e)
    }
}

/// 注册 provider（key 加密落库）。L3：is_default 时同事务降级同 capability 的存量默认。
#[allow(clippy::too_many_arguments)]
pub async fn insert_provider_tx(
    pool: &PgPool,
    id: Uuid,
    name: &str,
    base_url: &str,
    api_key_encrypted: &[u8],
    model_id: &str,
    capability: &str,
    is_default: bool,
) -> StoreResult<()> {
    let mut tx = pool.begin().await?;
    if is_default {
        // 同 capability 的唯一默认（chat 与 embedding 各有一个默认）
        sqlx::query(
            "UPDATE llm_providers SET is_default = false WHERE is_default AND capability = $1",
        )
        .bind(capability)
        .execute(&mut *tx)
        .await?;
    }
    let insert = sqlx::query(
        "INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, model_id, capability, is_default)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(name)
    .bind(base_url)
    .bind(api_key_encrypted)
    .bind(model_id)
    .bind(capability)
    .bind(is_default)
    .execute(&mut *tx)
    .await;
    if let Err(e) = insert {
        return Err(conflict(e));
    }
    tx.commit().await?;
    Ok(())
}

/// 当前 capability（默认切换时新 capability 的回退来源）。
pub async fn provider_capability(pool: &PgPool, id: Uuid) -> StoreResult<Option<String>> {
    let cap: Option<String> =
        sqlx::query_scalar("SELECT capability FROM llm_providers WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    Ok(cap)
}

/// L2：更新 provider（COALESCE 逐字段；is_default 切换保持同 capability 唯一默认）。
/// 返回更新后的行（None = 不存在）。
#[allow(clippy::too_many_arguments)]
pub async fn update_provider_tx(
    pool: &PgPool,
    id: Uuid,
    base_url: Option<&str>,
    api_key_encrypted: Option<&[u8]>,
    model_id: Option<&str>,
    capability: Option<&str>,
    is_default: Option<bool>,
) -> StoreResult<Option<(Uuid, String, String, String, String, bool)>> {
    let mut tx = pool.begin().await?;
    if is_default == Some(true) {
        // 同 capability 的唯一默认（capability 变更时用新值降级存量）
        let cur_cap: Option<String> =
            sqlx::query_scalar("SELECT capability FROM llm_providers WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        let new_cap = capability
            .map(str::to_string)
            .or(cur_cap)
            .unwrap_or_else(|| "chat".to_string());
        sqlx::query(
            "UPDATE llm_providers SET is_default = false WHERE is_default AND capability = $2 AND id <> $1",
        )
        .bind(id)
        .bind(&new_cap)
        .execute(&mut *tx)
        .await?;
    }

    // COALESCE 逐字段更新；未提供的字段保持原值
    let row = sqlx::query_as::<_, (Uuid, String, String, String, String, bool)>(
        "UPDATE llm_providers SET \
            base_url = COALESCE($2, base_url), \
            api_key_encrypted = COALESCE($3, api_key_encrypted), \
            model_id = COALESCE($4, model_id), \
            capability = COALESCE($5, capability), \
            is_default = COALESCE($6, is_default), \
            updated_at = now() \
         WHERE id = $1 \
         RETURNING id, name, base_url, model_id, capability, is_default",
    )
    .bind(id)
    .bind(base_url)
    .bind(api_key_encrypted)
    .bind(model_id)
    .bind(capability)
    .bind(is_default)
    .fetch_optional(&mut *tx)
    .await
    .map_err(conflict)?;
    tx.commit().await?;
    Ok(row)
}

/// (name, is_default)——删除前校验（默认 provider 拒删）。
pub async fn get_provider_name_default(
    pool: &PgPool,
    id: Uuid,
) -> StoreResult<Option<(String, bool)>> {
    let row: Option<(String, bool)> =
        sqlx::query_as("SELECT name, is_default FROM llm_providers WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    Ok(row)
}

pub async fn delete_provider(pool: &PgPool, id: Uuid) -> StoreResult<()> {
    sqlx::query("DELETE FROM llm_providers WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// provider 列表（永不含密钥）。
pub async fn list_providers(
    pool: &PgPool,
) -> StoreResult<Vec<(Uuid, String, String, String, String, bool)>> {
    let rows = sqlx::query_as(
        "SELECT id, name, base_url, model_id, capability, is_default FROM llm_providers ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 连通测试取行：(name, base_url, api_key_encrypted, model_id, capability)。
pub async fn get_provider_full(
    pool: &PgPool,
    id: Uuid,
) -> StoreResult<Option<(String, String, Vec<u8>, String, String)>> {
    let row: Option<(String, String, Vec<u8>, String, String)> = sqlx::query_as(
        "SELECT name, base_url, api_key_encrypted, model_id, capability FROM llm_providers WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// master key 轮换：全部 provider 密钥密文（按创建序）。
pub async fn all_provider_keys(pool: &PgPool) -> StoreResult<Vec<(Uuid, Vec<u8>)>> {
    let rows =
        sqlx::query_as("SELECT id, api_key_encrypted FROM llm_providers ORDER BY created_at")
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

/// master key 轮换落库：全部新密文单事务写入。
pub async fn update_provider_keys_tx(pool: &PgPool, rows: &[(Uuid, Vec<u8>)]) -> StoreResult<()> {
    let mut tx = pool.begin().await?;
    for (id, enc) in rows {
        sqlx::query(
            "UPDATE llm_providers SET api_key_encrypted = $2, updated_at = now() WHERE id = $1",
        )
        .bind(id)
        .bind(enc)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// AI 路由建议用：(name, model_id, capability)。
pub async fn list_provider_name_model_cap(
    pool: &PgPool,
) -> StoreResult<Vec<(String, String, String)>> {
    let rows = sqlx::query_as("SELECT name, model_id, capability FROM llm_providers ORDER BY name")
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// 路由表校验用：(name, model_id)——一个 provider 一个模型。
pub async fn list_provider_models(pool: &PgPool) -> StoreResult<Vec<(String, String)>> {
    let rows = sqlx::query_as("SELECT name, model_id FROM llm_providers")
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

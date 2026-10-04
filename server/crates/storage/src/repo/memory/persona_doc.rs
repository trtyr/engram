//! 画像活文档（P015）：单行 Markdown + 历史版本链。
//! 整理 Agent 定期编辑；API 只读暴露。每次保存旧版进 history（可审计可回滚）。

use super::*;

#[derive(sqlx::FromRow, Debug, Clone, serde::Serialize)]
pub struct PersonaDoc {
    pub content: String,
    pub version: i32,
    pub updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow, Debug, Clone, serde::Serialize)]
pub struct PersonaDocHistory {
    pub version: i32,
    pub content: String,
    pub summary: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// 当前画像文档（未初始化时返回 None——整理 Agent 首跑或手动编辑后生成）。
pub async fn persona_doc_get(pool: &PgPool) -> StoreResult<Option<PersonaDoc>> {
    sqlx::query_as::<_, PersonaDoc>(
        "SELECT content, version, updated_at FROM persona_doc WHERE id = 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

/// 保存新版：旧版进 history，主行 version+1（单事务）。
/// 首次保存（主行不存在）version=1 直插，无 history 行。
pub async fn persona_doc_save(
    pool: &PgPool,
    content: &str,
    summary: Option<&str>,
) -> StoreResult<i32> {
    let mut tx = pool.begin().await?;
    let prev: Option<(String, i32)> =
        sqlx::query_as("SELECT content, version FROM persona_doc WHERE id = 1")
            .fetch_optional(&mut *tx)
            .await?;
    let new_version = match &prev {
        Some((old_content, old_version)) => {
            sqlx::query(
                "INSERT INTO persona_doc_history (version, content, summary) VALUES ($1, $2, $3)",
            )
            .bind(old_version)
            .bind(old_content)
            .bind(summary)
            .execute(&mut *tx)
            .await?;
            old_version + 1
        }
        None => 1,
    };
    sqlx::query(
        "INSERT INTO persona_doc (id, content, version, updated_at) VALUES (1, $1, $2, now())
         ON CONFLICT (id) DO UPDATE SET content = $1, version = $2, updated_at = now()",
    )
    .bind(content)
    .bind(new_version)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(new_version)
}

/// 历史版本（新→旧）。
pub async fn persona_doc_history(pool: &PgPool, limit: i64) -> StoreResult<Vec<PersonaDocHistory>> {
    sqlx::query_as::<_, PersonaDocHistory>(
        "SELECT version, content, summary, created_at FROM persona_doc_history ORDER BY version DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

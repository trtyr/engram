//! Review 系统（llm_wiki 模式）：ingest 时 LLM flag 人审项，
//! 预定义动作（create_page / deep_research / skip / flag）+ 预生成检索词，
//! 异步处理不阻塞 ingest。

use chrono::{DateTime, Utc};
use engram_jobs::types::JobError;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, serde::Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ReviewItem {
    pub id: Uuid,
    pub kind: String,
    #[schema(value_type = Object)]
    pub payload: serde_json::Value,
    pub action: Option<String>,
    #[schema(value_type = Vec<String>)]
    pub search_queries: serde_json::Value,
    pub source_id: Option<Uuid>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

/// LLM 输出的 review flag（预定义动作约束——防幻觉任意动作）。
#[derive(Debug, serde::Deserialize)]
pub struct LlmReviewFlag {
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub suggested_slug: Option<String>,
    #[serde(default)]
    pub search_queries: Vec<String>,
}

/// 校验 LLM flag 的动作合法性（预定义集合外的丢弃）。
pub fn valid_kind(k: &str) -> bool {
    matches!(k, "create_page" | "deep_research" | "skip" | "flag")
}

/// ingest 内落库 review 项。
pub async fn create_items(
    pool: &PgPool,
    source_id: Uuid,
    flags: &[LlmReviewFlag],
) -> Result<Vec<Uuid>, JobError> {
    let mut ids = Vec::new();
    for f in flags {
        if !valid_kind(&f.kind) {
            tracing::warn!(kind = %f.kind, "丢弃非法 review 动作");
            continue;
        }
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO wiki_review_items (id, kind, payload, search_queries, source_id) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(id)
        .bind(&f.kind)
        .bind(serde_json::json!({
            "title": f.title,
            "reason": f.reason,
            "suggested_slug": f.suggested_slug,
        }))
        .bind(sqlx::types::Json(&f.search_queries))
        .bind(source_id)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        ids.push(id);
    }
    Ok(ids)
}

pub async fn list_open(pool: &PgPool) -> Result<Vec<ReviewItem>, JobError> {
    sqlx::query_as::<_, ReviewItem>(
        "SELECT * FROM wiki_review_items WHERE status = 'open' ORDER BY created_at DESC LIMIT 200",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))
}

/// 处理（resolve/dismiss + 动作标签）。返回是否命中（false = 不存在或已处理）。
pub async fn resolve(
    pool: &PgPool,
    id: Uuid,
    action: Option<&str>,
    dismiss: bool,
) -> Result<bool, JobError> {
    let status = if dismiss { "dismissed" } else { "resolved" };
    let n = sqlx::query(
        "UPDATE wiki_review_items SET status = $2, action = $3, resolved_at = now() \
         WHERE id = $1 AND status = 'open'",
    )
    .bind(id)
    .bind(status)
    .bind(action)
    .execute(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?
    .rows_affected();
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_validation() {
        assert!(valid_kind("create_page"));
        assert!(valid_kind("deep_research"));
        assert!(valid_kind("skip"));
        assert!(valid_kind("flag"));
        assert!(!valid_kind("delete_everything")); // 幻觉动作被拒
        assert!(!valid_kind(""));
    }
}

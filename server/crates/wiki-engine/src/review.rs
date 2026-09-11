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

/// ingest 内落库 review 项（人审项挂库——多库后按 library_id 归属）。
pub async fn create_items(
    pool: &PgPool,
    lib: Uuid,
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
            "INSERT INTO wiki_review_items (id, library_id, kind, payload, search_queries, source_id) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(lib)
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

/// 语义 lint（lint_deep）产出的问题项落库——无 source_id（来源是页面集合而非单一原料），
/// kind="flag" 复用现有 review 动作白名单，payload 带 lint 上下文供人审 UI 展示。
pub async fn create_lint_items(
    pool: &PgPool,
    lib: Uuid,
    issues: &[crate::lint_deep::SemanticIssue],
) -> Result<Vec<Uuid>, JobError> {
    let mut ids = Vec::new();
    for it in issues {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO wiki_review_items (id, library_id, kind, payload, search_queries, source_id) \
             VALUES ($1, $2, 'flag', $3, '[]'::jsonb, NULL)",
        )
        .bind(id)
        .bind(lib)
        .bind(serde_json::json!({
            "title": it.title(),
            "via": "semantic_lint",
            "lint_type": it.r#type,
            "pages": it.pages,
            "reason": it.detail,
            "suggestion": it.suggestion,
        }))
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        ids.push(id);
    }
    Ok(ids)
}

/// 某库的审查项列表（按库过滤；status 缺省 open，可查 resolved/dismissed 全态）。
pub async fn list_by_status(
    pool: &PgPool,
    lib: Uuid,
    status: Option<&str>,
) -> Result<Vec<ReviewItem>, JobError> {
    // 白名单校验——防笔误静默返回空
    if let Some(s) = status
        && !matches!(s, "open" | "resolved" | "dismissed")
    {
        return Err(JobError::Permanent(format!(
            "status 仅接受 open/resolved/dismissed（收到 {s}）"
        )));
    }
    let s = status.unwrap_or("open");
    sqlx::query_as::<_, ReviewItem>(
        "SELECT * FROM wiki_review_items \
         WHERE status = $2 AND library_id = $1 ORDER BY created_at DESC LIMIT 200",
    )
    .bind(lib)
    .bind(s)
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

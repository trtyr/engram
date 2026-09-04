//! purpose.md（wiki 灵魂）：方向意图，ingest/query 注入，LLM 可建议更新。
//! 存 settings 表（key=wiki_purpose），API 走系统页语义。

use engram_jobs::types::JobError;
use sqlx::PgPool;

const PURPOSE_KEY: &str = "wiki_purpose";

#[derive(Debug, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Purpose {
    /// wiki 存在的目标（为什么建这个知识库）
    pub goals: Vec<String>,
    /// 关键问题（wiki 应能回答什么）
    pub key_questions: Vec<String>,
    /// 研究范围边界
    pub scope: Vec<String>,
    /// 演化中的核心论点
    pub thesis: Option<String>,
}

impl Purpose {
    pub fn default_for_user() -> Self {
        Self {
            goals: vec!["沉淀个人长期知识".into()],
            key_questions: vec![],
            scope: vec![],
            thesis: None,
        }
    }

    /// 渲染为注入 LLM 的 Markdown（llm_wiki 的 purpose.md 形态）。
    pub fn to_markdown(&self) -> String {
        let mut md = String::from("# Purpose\n\n## 目标\n");
        for g in &self.goals {
            md.push_str(&format!("- {g}\n"));
        }
        if !self.key_questions.is_empty() {
            md.push_str("\n## 关键问题\n");
            for q in &self.key_questions {
                md.push_str(&format!("- {q}\n"));
            }
        }
        if !self.scope.is_empty() {
            md.push_str("\n## 范围\n");
            for s in &self.scope {
                md.push_str(&format!("- {s}\n"));
            }
        }
        if let Some(t) = &self.thesis {
            md.push_str(&format!("\n## 核心论点\n{t}\n"));
        }
        md
    }
}

pub async fn get_purpose(pool: &PgPool) -> Result<Option<Purpose>, JobError> {
    let row: Option<(sqlx::types::Json<Purpose>,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = $1")
            .bind(PURPOSE_KEY)
            .fetch_optional(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(row.map(|(j,)| j.0))
}

pub async fn set_purpose(pool: &PgPool, p: &Purpose) -> Result<(), JobError> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES ($1, $2) \
         ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = now()",
    )
    .bind(PURPOSE_KEY)
    .bind(sqlx::types::Json(p))
    .execute(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(())
}

/// ingest/query 注入用的 Markdown（无配置时给最小默认）。
pub async fn purpose_context(pool: &PgPool) -> String {
    match get_purpose(pool).await {
        Ok(Some(p)) => p.to_markdown(),
        _ => Purpose::default_for_user().to_markdown(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_render() {
        let p = Purpose {
            goals: vec!["研究向量数据库".into()],
            key_questions: vec!["哪个最快?".into()],
            scope: vec!["仅开源产品".into()],
            thesis: Some("pgvector 够用".into()),
        };
        let md = p.to_markdown();
        assert!(md.contains("# Purpose"));
        assert!(md.contains("研究向量数据库"));
        assert!(md.contains("pgvector 够用"));
    }
}

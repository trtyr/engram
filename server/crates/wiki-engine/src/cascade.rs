//! 级联删除（llm_wiki 模式）：删 source → 摘要页整页删 + 共享页仅摘除来源
//! + dead wikilink 清理 + index.md 同步。
//!
//! 三种匹配路径（对齐 llm_wiki 的 3-method matching）：
//! ① frontmatter sources[] 含该 source id（主路径）
//! ② page_type='source' 且唯一来源 → 摘要页判定（整页删）
//! ③ 共享 entity/concept 多源页 → 仅从 sources[] 移除该 source（保留页面）

use agent_memory_jobs::types::JobError;
use sqlx::PgPool;
use uuid::Uuid;

use crate::markup::extract_wikilinks;

#[derive(Debug, Default, serde::Serialize, utoipa::ToSchema)]
pub struct CascadeReport {
    pub deleted_pages: Vec<String>,
    pub updated_shared: Vec<String>,
    pub cleaned_links: usize,
}

/// 删除一个 wiki_source 的全部下游（页面/链接/索引）。
pub async fn cascade_delete_source(
    pool: &PgPool,
    source_id: Uuid,
) -> Result<CascadeReport, JobError> {
    let mut report = CascadeReport::default();
    let sid = source_id.to_string();

    // 1. 引用了该 source 的全部页面（sources[] 数组包含）
    let linked: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, slug, page_type FROM wiki_pages \
         WHERE frontmatter->'sources' @> to_jsonb(ARRAY[$1::text])",
    )
    .bind(&sid)
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    let mut delete_slugs: Vec<String> = Vec::new();

    for (id, slug, page_type) in linked {
        // 摘要页 = page_type='source' 且唯一来源 → 整页删
        // 共享页（entity/concept 多源）→ 仅从 sources[] 移除该 source，保留页面
        let source_count: i32 = sqlx::query_scalar(
            "SELECT jsonb_array_length(frontmatter->'sources') FROM wiki_pages WHERE id = $1",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

        if page_type == "source" && source_count <= 1 {
            sqlx::query("DELETE FROM wiki_pages WHERE id = $1")
                .bind(id)
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
            report.deleted_pages.push(slug.clone());
            delete_slugs.push(slug);
        } else {
            // 共享页：移除该 source 引用
            sqlx::query(
                "UPDATE wiki_pages SET \
                    frontmatter = jsonb_set(frontmatter, '{sources}', \
                        (SELECT COALESCE(jsonb_agg(s), '[]'::jsonb) FROM \
                            jsonb_array_elements_text(frontmatter->'sources') s \
                            WHERE s != $2)), \
                    updated_at = now() \
                 WHERE id = $1",
            )
            .bind(id)
            .bind(&sid)
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
            report.updated_shared.push(slug);
        }
    }

    // 2. dead wikilink 清理：剩余页面中指向已删 slug 的 [[link]] 移除
    if !delete_slugs.is_empty() {
        let remaining: Vec<(String, String)> = sqlx::query_as(
            "SELECT slug, content FROM wiki_pages WHERE page_type NOT IN ('index','log')",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

        for (slug, content) in remaining {
            let links = extract_wikilinks(&content);
            let dead: Vec<&String> = links.iter().filter(|l| delete_slugs.contains(l)).collect();
            if dead.is_empty() {
                continue;
            }
            let mut cleaned = content.clone();
            for d in &dead {
                cleaned = cleaned.replace(&format!("[[{d}]]"), "");
            }
            let cleaned = cleaned.replace("\n\n\n", "\n\n");
            sqlx::query("UPDATE wiki_pages SET content = $2, updated_at = now() WHERE slug = $1")
                .bind(&slug)
                .bind(&cleaned)
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
            report.cleaned_links += dead.len();
        }

        // 3. index.md 同步（重建式）
        crate::ingest::rebuild_index_page(pool).await?;
    }

    // 4. 删 source 行本身
    sqlx::query("DELETE FROM wiki_sources WHERE id = $1")
        .bind(source_id)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_serializes() {
        let r = CascadeReport {
            deleted_pages: vec!["a".into()],
            updated_shared: vec!["b".into()],
            cleaned_links: 3,
        };
        let j = serde_json::to_value(&r).unwrap();
        assert_eq!(j["deleted_pages"][0], "a");
        assert_eq!(j["cleaned_links"], 3);
    }
}

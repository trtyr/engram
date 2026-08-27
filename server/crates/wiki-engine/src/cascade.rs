//! 级联删除（llm_wiki 模式）：删 source → 摘要页整页删 + 共享页仅摘除来源
//! + dead wikilink 清理 + index.md 同步。
//!
//! 三种匹配路径（对齐 llm_wiki 的 3-method matching）：
//! ① frontmatter sources[] 含该 source id（主路径）
//! ② page_type='source' 且唯一来源 → 摘要页判定（整页删）
//! ③ 共享 entity/concept 多源页 → 仅从 sources[] 移除该 source（保留页面）
//!
//! 所有写操作包进一个事务（R4）：删页/摘源/清 dead link/删 source 原子提交，
//! 失败整体回滚；index.md 重建是派生数据，放在事务外幂等重算。

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

    // 事务包裹全部写操作，失败整体回滚
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    // 1. 引用了该 source 的全部页面（sources[] 数组包含）
    let linked: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, slug, page_type FROM wiki_pages \
         WHERE frontmatter->'sources' @> to_jsonb(ARRAY[$1::text])",
    )
    .bind(&sid)
    .fetch_all(&mut *tx)
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
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

        if page_type == "source" && source_count <= 1 {
            sqlx::query("DELETE FROM wiki_pages WHERE id = $1")
                .bind(id)
                .execute(&mut *tx)
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
            .execute(&mut *tx)
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
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

        for (slug, content) in remaining {
            let links = extract_wikilinks(&content);
            let dead: Vec<&String> = links.iter().filter(|l| delete_slugs.contains(l)).collect();
            if dead.is_empty() {
                continue;
            }
            // W8：结构化移除（含 [[slug|别名]] 形式）——精确串替换会留 `|别名]]` 裸碎片
            let mut cleaned = content;
            for d in &dead {
                cleaned = crate::markup::remove_wikilinks(&cleaned, d);
            }
            let cleaned = cleaned.replace("\n\n\n", "\n\n");
            sqlx::query("UPDATE wiki_pages SET content = $2, updated_at = now() WHERE slug = $1")
                .bind(&slug)
                .bind(&cleaned)
                .execute(&mut *tx)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
            report.cleaned_links += dead.len();
        }
    }

    // 3. 删 source 行本身（事务内，与页面删除原子）
    sqlx::query("DELETE FROM wiki_sources WHERE id = $1")
        .bind(source_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    // W5：幽灵边——指向已删页面的 wiki_links 边必须在事务内一并删除
    // （旧实现只清 content 里的 [[link]] 文本，边表残留悬空边污染图视图与洞察）
    if !delete_slugs.is_empty() {
        sqlx::query("DELETE FROM wiki_links WHERE from_slug = ANY($1) OR to_slug = ANY($1)")
            .bind(&delete_slugs)
            .execute(&mut *tx)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
    }

    tx.commit()
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    // 4. index.md 同步（派生数据，事务外幂等重建）
    if !delete_slugs.is_empty() {
        crate::ingest::rebuild_index_page(pool).await?;
    }

    // 5. W5：无据边清理——摘源后，既无内容 wikilink 又无共享源的边不再成立，删除
    //    （源重叠补边只在 ingest 时新增，从不回收——摘源后成为无据残留）
    for slug in report.updated_shared.clone() {
        let others: Vec<String> = sqlx::query_scalar(
            "SELECT to_slug FROM wiki_links WHERE from_slug = $1 \
             UNION SELECT from_slug FROM wiki_links WHERE to_slug = $1",
        )
        .bind(&slug)
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        for other in others {
            let pages: Vec<(String, String, Vec<String>)> = sqlx::query_as(
                "SELECT slug, content, \
                 ARRAY(SELECT jsonb_array_elements_text(frontmatter->'sources')) \
                 FROM wiki_pages WHERE slug = ANY($1)",
            )
            .bind(vec![slug.clone(), other.clone()])
            .fetch_all(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
            if pages.len() != 2 {
                continue; // 对端已删（幽灵边已被上面事务内清理）
            }
            let (a, b) = (&pages[0], &pages[1]);
            let direct = extract_wikilinks(&a.1).contains(&other) || extract_wikilinks(&b.1).contains(&slug);
            let shared = a.2.iter().any(|s| b.2.contains(s));
            if !direct && !shared {
                sqlx::query(
                    "DELETE FROM wiki_links \
                     WHERE (from_slug = $1 AND to_slug = $2) OR (from_slug = $2 AND to_slug = $1)",
                )
                .bind(&slug)
                .bind(&other)
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
            }
        }
    }

    // 6. W5：权重重算（删除/摘源后剩余边的 4 信号权重修正）
    if !delete_slugs.is_empty() || !report.updated_shared.is_empty() {
        crate::relevance::rebuild_weights(pool).await?;
    }

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

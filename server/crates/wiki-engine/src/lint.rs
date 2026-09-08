//! Lint：Wiki 健康检查（只报告不改写）。

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::markup::extract_wikilinks;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LintIssue {
    pub rule: String,
    pub slug: String,
    pub detail: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LintReport {
    pub issues: Vec<LintIssue>,
    pub checked_pages: usize,
}

/// 全量 lint（某库）：死链 / 孤儿 / 过时源 / 损坏 frontmatter / 重复实体。
/// pages/links/sources 全部按 library_id 隔离（多库 0037）。
pub async fn lint(pool: &PgPool, lib: Uuid) -> Result<LintReport, sqlx::Error> {
    // D23：排除系统 log 页——list_pages 不可见的页不该进 lint 口径（此前 checked_pages 恒定 +1）
    let pages: Vec<(Uuid, String, String, String, serde_json::Value)> = sqlx::query_as(
        "SELECT id, slug, page_type, content, frontmatter FROM wiki_pages \
         WHERE page_type <> 'log' AND library_id = $1",
    )
    .bind(lib)
    .fetch_all(pool)
    .await?;
    let slugs: std::collections::HashSet<String> =
        pages.iter().map(|(_, s, ..)| s.clone()).collect();
    // W-1（2026-09-03）：小写索引——LLM 生成正文常把链接写成标题原文（[[Engram]]），
    // 与真实 slug（engram）只差大小写。仅大小写差异不再误判死链，改报 case_mismatch。
    let lower_slugs: std::collections::HashMap<String, String> = pages
        .iter()
        .map(|(_, s, ..)| (s.to_lowercase(), s.clone()))
        .collect();
    let system = ["index", "log", "overview"];
    let mut issues = Vec::new();

    // 入链计数（wikilink 边，库内）
    let inlinks: Vec<(String, i64)> = sqlx::query_as(
        "SELECT to_slug, count(*) FROM wiki_links WHERE library_id = $1 GROUP BY to_slug",
    )
    .bind(lib)
    .fetch_all(pool)
    .await?;
    let inlink_map: std::collections::HashMap<String, i64> = inlinks.into_iter().collect();

    for (_, slug, _page_type, content, fm) in &pages {
        // 1. 死链（W-1：精确未中→小写重查，仅大小写差异报 case_mismatch 而非 dead_link）
        for target in extract_wikilinks(content) {
            if !slugs.contains(&target) {
                if let Some(real) = lower_slugs.get(&target.to_lowercase()) {
                    issues.push(LintIssue {
                        rule: "case_mismatch".into(),
                        slug: slug.clone(),
                        detail: format!(
                            "[[{target}]] 与页面 slug「{real}」仅大小写不同——建议改用 [[{real}]]"
                        ),
                    });
                } else {
                    issues.push(LintIssue {
                        rule: "dead_link".into(),
                        slug: slug.clone(),
                        detail: format!("[[{target}]] 指向不存在的页面"),
                    });
                }
            }
        }
        // 2. 孤儿（系统页豁免）
        if !system.contains(&slug.as_str()) && inlink_map.get(slug).copied().unwrap_or(0) == 0 {
            issues.push(LintIssue {
                rule: "orphan".into(),
                slug: slug.clone(),
                detail: "无任何入链的孤立页面".into(),
            });
        }
        // 3. 损坏 frontmatter（sources 缺失或非数组；系统页豁免）
        if !system.contains(&slug.as_str())
            && !fm.get("sources").map(|s| s.is_array()).unwrap_or(false)
        {
            issues.push(LintIssue {
                rule: "broken_frontmatter".into(),
                slug: slug.clone(),
                detail: "frontmatter 缺少 sources 数组".into(),
            });
        }
    }

    // 4. 重复实体（同名标题不同 slug 的 entity/concept，库内）
    let by_title: Vec<(String, String)> = sqlx::query_as(
        "SELECT COALESCE(frontmatter->>'title', slug), slug FROM wiki_pages \
         WHERE page_type IN ('entity','concept') AND library_id = $1",
    )
    .bind(lib)
    .fetch_all(pool)
    .await?;
    let mut seen: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for (title, slug) in by_title {
        seen.entry(title).or_default().push(slug);
    }
    for (title, slugs_dup) in seen {
        if slugs_dup.len() > 1 {
            issues.push(LintIssue {
                rule: "duplicate_entity".into(),
                slug: slugs_dup.join(","),
                detail: format!("同标题「{title}」存在多个页面"),
            });
        }
    }

    // 5. 过时源：sha 已变但页面未重新 ingest（原料目录与页面 sources 对比，库内）
    let sources: Vec<(String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id::text, COALESCE(last_ingested_at, created_at) FROM wiki_sources WHERE library_id = $1",
    )
    .bind(lib)
    .fetch_all(pool)
    .await?;
    let page_source: Vec<String> = sqlx::query_scalar(
        "SELECT jsonb_array_elements_text(frontmatter->'sources') FROM wiki_pages \
         WHERE frontmatter->'sources' IS NOT NULL AND library_id = $1",
    )
    .bind(lib)
    .fetch_all(pool)
    .await?;
    for (sid, _) in &sources {
        if !page_source.iter().any(|p| p == sid) {
            // 原料存在但没有页面引用它（从未生成或已删）
            let cnt: i64 = sqlx::query_scalar(
                // $1 是 id::text 查出的字符串，必须显式 cast 回 uuid（uuid = text 会 503）
                "SELECT count(*) FROM wiki_sources \
                 WHERE id = $1::uuid AND library_id = $2 AND last_ingested_at IS NOT NULL",
            )
            .bind(sid)
            .bind(lib)
            .fetch_one(pool)
            .await?;
            if cnt > 0 {
                issues.push(LintIssue {
                    rule: "stale_source".into(),
                    slug: sid.clone(),
                    detail: "原料已 ingest 但无页面引用其内容".into(),
                });
            }
        }
    }

    Ok(LintReport {
        issues,
        checked_pages: pages.len(),
    })
}

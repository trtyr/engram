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

    // 逐页检查：死链 / 孤儿 / 损坏 frontmatter
    let mut issues = lint_page_issues(pool, lib, &pages, &slugs, &lower_slugs, &system).await?;
    // 库级检查：重复实体 / 过时源
    issues.extend(duplicate_title_issues(pool, lib).await?);
    issues.extend(stale_source_issues(pool, lib).await?);

    Ok(LintReport {
        issues,
        checked_pages: pages.len(),
    })
}

/// 逐页检查：死链（含跨库 / 大小写）、孤儿页、损坏 frontmatter。
async fn lint_page_issues(
    pool: &PgPool,
    lib: Uuid,
    pages: &[(Uuid, String, String, String, serde_json::Value)],
    slugs: &std::collections::HashSet<String>,
    lower_slugs: &std::collections::HashMap<String, String>,
    system: &[&str],
) -> Result<Vec<LintIssue>, sqlx::Error> {
    let mut issues = Vec::new();
    // 入链计数（wikilink 边，库内）
    let inlinks: Vec<(String, i64)> = sqlx::query_as(
        "SELECT to_slug, count(*) FROM wiki_links WHERE library_id = $1 GROUP BY to_slug",
    )
    .bind(lib)
    .fetch_all(pool)
    .await?;
    let inlink_map: std::collections::HashMap<String, i64> = inlinks.into_iter().collect();

    // R 多库补全：先批量收集跨库引用目标（lib/slug），一次查存在性——存在则跳过死链判定
    let mut cross_keys: Vec<(String, String)> = Vec::new();
    for (_, _, _, content, _) in pages {
        for target in extract_wikilinks(content) {
            if let Some(pair) = crate::markup::split_cross_lib(&target) {
                cross_keys.push(pair);
            }
        }
    }
    let cross_existing = crate::cross_links::filter_existing(pool, &cross_keys).await?;

    for (_, slug, _page_type, content, fm) in pages {
        // 1. 死链（逐链接判定，见助手）
        issues.extend(dead_link_issues(
            slug,
            content,
            slugs,
            lower_slugs,
            &cross_existing,
        ));
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
    Ok(issues)
}

/// 重复实体：同名标题不同 slug 的 entity/concept（库内）。
async fn duplicate_title_issues(pool: &PgPool, lib: Uuid) -> Result<Vec<LintIssue>, sqlx::Error> {
    let mut issues = Vec::new();
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
    Ok(issues)
}

/// 过时源：原料已 ingest 但没有页面引用其内容（库内）。
async fn stale_source_issues(pool: &PgPool, lib: Uuid) -> Result<Vec<LintIssue>, sqlx::Error> {
    let mut issues = Vec::new();
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
    Ok(issues)
}

/// 死链检查（逐 wikilink）：跨库引用按「目标库+页是否存在」判定，本库引用精确未中后
/// 小写重查——仅大小写差异报 `case_mismatch` 而非 `dead_link`（W-1）。
fn dead_link_issues(
    page_slug: &str,
    content: &str,
    slugs: &std::collections::HashSet<String>,
    lower_slugs: &std::collections::HashMap<String, String>,
    cross_existing: &std::collections::HashMap<(String, String), ()>,
) -> Vec<LintIssue> {
    let mut issues = Vec::new();
    // 1. 死链（W-1：精确未中→小写重查，仅大小写差异报 case_mismatch 而非 dead_link）
    for target in extract_wikilinks(content) {
        // 跨库引用：目标（库+页）存在则合法跳过；不存在报跨库 dead_link
        if let Some((to_lib, to_slug)) = crate::markup::split_cross_lib(&target) {
            if !cross_existing.contains_key(&(to_lib.clone(), to_slug.clone())) {
                issues.push(LintIssue {
                    rule: "dead_link".into(),
                    slug: page_slug.to_string(),
                    detail: format!(
                        "[[{target}]] 指向不存在的跨库页面（库「{to_lib}」无页「{to_slug}」）"
                    ),
                });
            }
            continue;
        }
        if !slugs.contains(&target) {
            if let Some(real) = lower_slugs.get(&target.to_lowercase()) {
                issues.push(LintIssue {
                    rule: "case_mismatch".into(),
                    slug: page_slug.to_string(),
                    detail: format!(
                        "[[{target}]] 与页面 slug「{real}」仅大小写不同——建议改用 [[{real}]]"
                    ),
                });
            } else {
                issues.push(LintIssue {
                    rule: "dead_link".into(),
                    slug: page_slug.to_string(),
                    detail: format!("[[{target}]] 指向不存在的页面"),
                });
            }
        }
    }
    issues
}

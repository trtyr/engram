//! `ingest` 的实现切片（架构治理 2026-09-20：自 ingest.rs 纯搬移，零行为变化）。

use super::*;

pub(super) async fn title_of(pool: &sqlx::PgPool, id: Uuid) -> Result<String, JobError> {
    let t: Option<Option<String>> =
        sqlx::query_scalar("SELECT title FROM wiki_sources WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(t.flatten().unwrap_or_else(|| "未命名源".into()))
}

pub(super) async fn read_source(pool: &sqlx::PgPool, id: Uuid) -> Result<String, JobError> {
    let path: String = sqlx::query_scalar("SELECT raw_path FROM wiki_sources WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| JobError::Permanent(format!("读原料失败: {e}")))
}

/// 某库的既有页面目录（注入 LLM 用）。
pub(super) async fn read_index(pool: &sqlx::PgPool, lib: Uuid) -> Result<String, JobError> {
    let pages: Vec<(String, String)> = sqlx::query_as(
        "SELECT slug, COALESCE(frontmatter->>'title', slug) FROM wiki_pages \
         WHERE library_id = $1 ORDER BY updated_at DESC LIMIT 200",
    )
    .bind(lib)
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(pages
        .into_iter()
        .map(|(slug, title)| format!("- [[{slug}]] {title}"))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// 重建给定页面的出边（wikilink 边）。W6：包事务——与并发 generate 的
/// 链接重建交错时不再留 DELETE/INSERT 半态。多库：边挂库、删除/插入都限定库内。
pub(super) async fn rebuild_links(
    pool: &sqlx::PgPool,
    lib: Uuid,
    slugs: &[String],
) -> Result<(), JobError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    for slug in slugs {
        let content: Option<String> = sqlx::query_scalar(
            "SELECT content FROM wiki_pages WHERE slug = $1 AND library_id = $2",
        )
        .bind(slug)
        .bind(lib)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?
        .flatten();
        let Some(content) = content else { continue };
        sqlx::query("DELETE FROM wiki_links WHERE from_slug = $1 AND library_id = $2")
            .bind(slug)
            .bind(lib)
            .execute(&mut *tx)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
        for target in extract_wikilinks(&content) {
            sqlx::query(
                "INSERT INTO wiki_links (library_id, from_slug, to_slug, weight) \
                 VALUES ($3, $1, $2, 3.0) \
                 ON CONFLICT (library_id, from_slug, to_slug) DO NOTHING",
            )
            .bind(slug)
            .bind(&target)
            .bind(lib)
            .execute(&mut *tx)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
        }
    }
    tx.commit()
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(())
}

/// index/log/overview 系统页维护（库内）。
pub(super) async fn update_index_and_log(
    pool: &sqlx::PgPool,
    lib: Uuid,
    source_id: Uuid,
    source_title: &str,
    created: usize,
    updated: usize,
    proposals: usize,
) -> Result<(), JobError> {
    rebuild_index_page(pool, lib).await?;

    // log：append 一行（存 latest 系统页；全量历史靠 job_events）
    let log_line = format!(
        "- {} ingest `{}` → 新建 {created} / 更新 {updated} / 提案 {proposals}{}",
        chrono::Utc::now().format("%Y-%m-%d %H:%M"),
        source_title,
        if created + updated + proposals == 0 {
            "（0 产物——薄内容未产出页面）"
        } else {
            ""
        }
    );
    let prev_log: Option<String> = sqlx::query_scalar(
        "SELECT content FROM wiki_pages WHERE slug = 'log' AND page_type = 'log' AND library_id = $1",
    )
    .bind(lib)
    .fetch_optional(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?
    .flatten();
    let log_md = format!(
        "{}\n{}",
        prev_log.unwrap_or_else(|| "# 操作日志\n".into()),
        log_line
    );
    // W7：截最近 500 行——log 不再无界增长（全量历史由 job_events 承担）
    let lines: Vec<&str> = log_md.lines().collect();
    let log_md = if lines.len() > 500 {
        lines[lines.len() - 500..].join("\n")
    } else {
        log_md
    };

    upsert_system_page(pool, lib, "log", "log", "日志", &log_md).await?;
    let _ = source_id;
    Ok(())
}

/// 重建 index 系统页（cascade 删除后同步复用；按库重建）。
pub async fn rebuild_index_page(pool: &sqlx::PgPool, lib: Uuid) -> Result<(), JobError> {
    let pages: Vec<(String, String)> = sqlx::query_as(
        "SELECT slug, COALESCE(frontmatter->>'title', slug) FROM wiki_pages \
         WHERE page_type NOT IN ('index','log') AND library_id = $1 ORDER BY updated_at DESC LIMIT 300",
    )
    .bind(lib)
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    let index_md = format!(
        "# 知识库索引\n\n{}\n",
        pages
            .iter()
            .map(|(s, t)| format!("- [[{s}]] — {t}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    upsert_system_page(pool, lib, "index", "index", "索引", &index_md).await?;
    Ok(())
}

/// 重建 overview 系统页：全局摘要（每页一行标题+summary），ingest 后调用（按库）。
pub async fn rebuild_overview_page(pool: &sqlx::PgPool, lib: Uuid) -> Result<(), JobError> {
    let pages: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT slug, COALESCE(frontmatter->>'title', slug), left(content, 120) FROM wiki_pages \
         WHERE page_type NOT IN ('index','log','overview') AND library_id = $1 \
         ORDER BY updated_at DESC LIMIT 100",
    )
    .bind(lib)
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    let body = pages
        .iter()
        .map(|(slug, title, excerpt)| format!("## [[{slug}]] {title}\n\n{excerpt}…"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let overview_md = format!(
        "# Overview\n\n知识库当前包含 {} 个页面。\n\n{body}\n",
        pages.len()
    );
    upsert_system_page(pool, lib, "overview", "overview", "总览", &overview_md).await?;
    Ok(())
}

/// 系统页 upsert（库内——多库后每库有自己的 index/log/overview）。
pub(super) async fn upsert_system_page(
    pool: &sqlx::PgPool,
    lib: Uuid,
    slug: &str,
    page_type: &str,
    title: &str,
    content: &str,
) -> Result<(), JobError> {
    sqlx::query(
        "INSERT INTO wiki_pages (id, library_id, slug, title, page_type, content, frontmatter, origin, version, folder) \
         VALUES ($1, $2, $3, $4, $5, $6, '{}'::jsonb, 'llm', 1, '系统') \
         ON CONFLICT (library_id, slug) DO UPDATE SET content = $6, updated_at = now()",
    )
    .bind(Uuid::now_v7())
    .bind(lib)
    .bind(slug)
    .bind(title)
    .bind(page_type)
    .bind(content)
    .execute(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(())
}

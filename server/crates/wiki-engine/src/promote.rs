//! 知识晋升页写入（EN-59）：从项目文档提炼的通用知识落库。
//!
//! 与 [`crate::service::WikiService::put_page`]（人工编辑，origin=human，page_type=concept）
//! 的差异：page_type=synthesis、origin=llm、frontmatter 带 `promoted_from` 源回链。
//! 版本快照、wikilinks 重算与 put_page 同口径（覆盖即快照、AI 互链对图与 lint 可见）。
//! 独立自由函数（不带 self）——调用方（core promote 编排）无需构造 ProviderRegistry。

use crate::WikiError;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn promote_page(
    pool: &PgPool,
    lib: Uuid,
    slug: &str,
    title: &str,
    content: &str,
    promoted_from: &str,
) -> Result<crate::WikiPageDto, WikiError> {
    if !crate::markup::is_valid_slug(slug) {
        return Err(WikiError::BadRequest(
            "slug 非法：仅允许字母/数字/-/_/·，≤80 字符，不含空格与路径分隔符".into(),
        ));
    }
    let fm = serde_json::json!({
        "title": title,
        "sources": [],
        "promoted_from": promoted_from,
    });
    // 版本快照（同 put_page 口径）：覆盖前把现状快照进 wiki_page_versions
    sqlx::query(
        "INSERT INTO wiki_page_versions (id, library_id, slug, version, title, page_type, folder, content, origin) \
         SELECT $1, $3, slug, version, title, page_type, folder, content, origin \
         FROM wiki_pages WHERE slug = $2 AND library_id = $3",
    )
    .bind(Uuid::now_v7())
    .bind(slug)
    .bind(lib)
    .execute(pool)
    .await?;
    crate::service::prune_page_versions(pool, lib, slug, crate::service::VERSION_KEEP).await;

    let row = sqlx::query_as::<_, crate::WikiPageDto>(
        "INSERT INTO wiki_pages (id, library_id, slug, title, page_type, content, frontmatter, origin, version, tsv) \
         VALUES ($1, $2, $3, $4, 'synthesis', $5, $6::jsonb, 'llm', 1, to_tsvector('simple', $7)) \
         ON CONFLICT (library_id, slug) DO UPDATE SET \
            title = $4, content = $5, origin = 'llm', \
            frontmatter = wiki_pages.frontmatter || $6::jsonb, \
            version = wiki_pages.version + 1, updated_at = now(), \
            tsv = to_tsvector('simple', $7) \
         RETURNING *",
    )
    .bind(Uuid::now_v7())
    .bind(lib)
    .bind(slug)
    .bind(title)
    .bind(content)
    .bind(fm.to_string())
    .bind(engram_search::tokenize::tsv_text_wiki(&format!("{slug} {title} {content}")))
    .fetch_one(pool)
    .await?;

    // D4 口径：落页后重算本页 wikilinks（graph/孤页检测与 lint 同源）
    sqlx::query("DELETE FROM wiki_links WHERE from_slug = $1 AND library_id = $2")
        .bind(slug)
        .bind(lib)
        .execute(pool)
        .await?;
    let mut cross_targets: Vec<(String, String)> = Vec::new();
    for target in crate::markup::extract_wikilinks(content) {
        if let Some((to_lib, to_slug)) = crate::markup::split_cross_lib(&target) {
            cross_targets.push((to_lib, to_slug));
            continue;
        }
        sqlx::query(
            "INSERT INTO wiki_links (library_id, from_slug, to_slug, weight) \
             VALUES ($3, $1, $2, 3.0) \
             ON CONFLICT (library_id, from_slug, to_slug) DO NOTHING",
        )
        .bind(slug)
        .bind(&target)
        .bind(lib)
        .execute(pool)
        .await
        .ok();
    }
    crate::cross_links::sync_page(pool, lib, slug, &cross_targets).await?;
    Ok(row)
}

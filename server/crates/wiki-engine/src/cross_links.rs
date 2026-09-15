//! R（wiki 多库补全）：跨库引用存储与查询。
//!
//! `[[lib/slug]]` 语法（markup::split_cross_lib 解析）的目标落本表；
//! 库内引用语义不变（wiki_links）。写入时校验目标库与页面存在，
//! 不存在则不建链（lint 会以跨库 dead_link 报告）。
use sqlx::{PgPool, Row as _};
use std::collections::HashMap;

/// 重建一页的跨库引用（write/restore 后调用）：全删再插，幂等。
/// 目标不存在（库或页面查无）的引用跳过——由 lint 报告，不阻塞写页。
pub async fn sync_page(
    pool: &PgPool,
    from_lib: uuid::Uuid,
    from_slug: &str,
    cross_targets: &[(String, String)], // (to_lib_slug, to_page_slug)
) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM wiki_cross_links WHERE from_library_id = $1 AND from_slug = $2")
        .bind(from_lib)
        .bind(from_slug)
        .execute(pool)
        .await?;
    for (to_lib_slug, to_slug) in cross_targets {
        // $1=from_library_id 值、$2=from_slug 值、$3=to_slug 值、$4=to 库 slug（WHERE 定位）
        sqlx::query(
            "INSERT INTO wiki_cross_links (from_library_id, from_slug, to_library_id, to_slug) \
             SELECT $1, $2, t.id, $3 \
             FROM wiki_libraries t \
             WHERE t.slug = $4 \
             AND EXISTS (SELECT 1 FROM wiki_pages p WHERE p.library_id = t.id AND p.slug = $3) \
             ON CONFLICT DO NOTHING",
        )
        .bind(from_lib)
        .bind(from_slug)
        .bind(to_slug)
        .bind(to_lib_slug)
        .execute(pool)
        .await
        .ok();
    }
    Ok(())
}

/// 删页/删库级联：清该页的跨库引用（from 侧与 to 侧）。
pub async fn delete_page_cleanup(pool: &PgPool, lib: uuid::Uuid, slug: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM wiki_cross_links WHERE from_library_id = $1 AND from_slug = $2")
        .bind(lib)
        .bind(slug)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM wiki_cross_links WHERE to_library_id = $1 AND to_slug = $2")
        .bind(lib)
        .bind(slug)
        .execute(pool)
        .await?;
    Ok(())
}

/// 反向跨库引用：谁（哪个库的哪个页）引用了我。返回 (from 库 slug, from 页 slug)。
pub async fn backlinks(
    pool: &PgPool,
    lib: uuid::Uuid,
    slug: &str,
) -> sqlx::Result<Vec<(String, String)>> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT fl.slug, cl.from_slug FROM wiki_cross_links cl \
         JOIN wiki_libraries fl ON fl.id = cl.from_library_id \
         WHERE cl.to_library_id = $1 AND cl.to_slug = $2 \
         ORDER BY fl.slug, cl.from_slug",
    )
    .bind(lib)
    .bind(slug)
    .fetch_all(pool)
    .await
}

/// lint 用：跨库目标是否存在（库 slug + 页面 slug 均命中）。
pub async fn target_exists(pool: &PgPool, to_lib_slug: &str, to_slug: &str) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (\
         SELECT 1 FROM wiki_libraries l \
         JOIN wiki_pages p ON p.library_id = l.id \
         WHERE l.slug = $1 AND p.slug = $2)",
    )
    .bind(to_lib_slug)
    .bind(to_slug)
    .fetch_one(pool)
    .await
    .unwrap_or(false)
}

/// 批量存在性（lint 一次查多个）：返回存在集合。
pub async fn filter_existing(
    pool: &PgPool,
    keys: &[(String, String)],
) -> sqlx::Result<HashMap<(String, String), ()>> {
    if keys.is_empty() {
        return Ok(HashMap::new());
    }
    let mut qb = sqlx::QueryBuilder::new(
        "SELECT l.slug AS lib, p.slug AS slug FROM wiki_libraries l \
         JOIN wiki_pages p ON p.library_id = l.id WHERE (l.slug, p.slug) IN (",
    );
    qb.push_values(keys, |mut b, (lib, slug)| {
        b.push_bind(lib).push_bind(slug);
    });
    qb.push(")");
    let rows = qb.build().fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|r| ((r.get::<String, _>("lib"), r.get::<String, _>("slug")), ()))
        .collect())
}

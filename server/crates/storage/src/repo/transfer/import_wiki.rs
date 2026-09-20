//! `transfer` 的实现切片（架构治理 2026-09-21：自 transfer.rs 纯搬移，零行为变化）。

use super::*;

/// 导入 wiki 库行（按 slug 幂等），返回 (目标库 id, 是否新插入)——同名库已存在时映射到现有 id。
pub async fn import_wiki_library(pool: &PgPool, v: &Value) -> StoreResult<(Uuid, bool)> {
    let slug = str_of(v, "slug", "");
    let res = sqlx::query(
        "INSERT INTO wiki_libraries (id, slug, name) VALUES ($1, $2, $3) \
         ON CONFLICT (slug) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(&slug)
    .bind(str_of(v, "name", ""))
    .execute(pool)
    .await?;
    let id: Uuid = sqlx::query_scalar("SELECT id FROM wiki_libraries WHERE slug = $1")
        .bind(&slug)
        .fetch_one(pool)
        .await?;
    Ok((id, res.rows_affected() > 0))
}

/// main 库 id（旧迁移包无 wiki_libraries 域时，页面 fallback 落主库用）。
pub async fn main_library_id(pool: &PgPool) -> StoreResult<Uuid> {
    let id: Uuid = sqlx::query_scalar("SELECT id FROM wiki_libraries WHERE slug = 'main'")
        .fetch_one(pool)
        .await?;
    Ok(id)
}

pub async fn import_wiki_page(
    pool: &PgPool,
    v: &Value,
    tsv_text: &str,
    target_lib: Uuid,
) -> StoreResult<bool> {
    let content = str_of(v, "content", "");
    // 多库迁移（2026-09-18 数据同步线补齐）：library 由调用方按 slug 映射传入，
    // 页面保留原库归属；旧包无 wiki_libraries 域时调用方 fallback main（v1 兼容）。
    // tsv 由调用方按 wiki 口径（slug+title+content、wiki 分词变体）算好传入——storage 不依赖分词器；
    // 系统页（index/log/overview）写 NULL（结构页不参与 FTS，EN-63 audit 回归教训）
    let res = sqlx::query(
        "INSERT INTO wiki_pages (id, library_id, slug, title, page_type, content, frontmatter, origin, version, folder, tsv, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
                 CASE WHEN $5 IN ('index','log','overview') THEN NULL ELSE to_tsvector('simple', $13) END, $11, $12) \
         ON CONFLICT (library_id, slug) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(target_lib)
    .bind(str_of(v, "slug", ""))
    .bind(str_of(v, "title", ""))
    .bind(str_of(v, "page_type", "concept"))
    .bind(&content)
    .bind(v.get("frontmatter").cloned().unwrap_or(serde_json::json!({})))
    .bind(str_of(v, "origin", "llm"))
    .bind(v.get("version").and_then(|x| x.as_i64()).unwrap_or(1) as i32)
    .bind(str_of(v, "folder", ""))
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
    .bind(ts(v, "updated_at").unwrap_or_else(Utc::now))
    .bind(tsv_text)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// promotions 导入（UNIQUE(project_id, doc_id, page_slug) 冲突跳过）。
/// library_id 统一映射目标库 main 库——多库 promotions 随 wiki 多库缺口记欠账（t9）。
pub async fn import_wiki_promotions(pool: &PgPool, items: &[Value]) -> StoreResult<(usize, usize)> {
    let Some(target_library_id): Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM wiki_libraries WHERE slug = 'main'")
            .fetch_optional(pool)
            .await?
    else {
        return Ok((0, items.len())); // 无 main 库（不该发生，0046 幂等保证）
    };
    let mut imported = 0usize;
    let mut skipped = 0usize;
    for v in items {
        let res = sqlx::query(
            "INSERT INTO wiki_promotions (id, library_id, page_slug, project_id, doc_id, anchor, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (project_id, doc_id, page_slug) DO NOTHING",
        )
        .bind(v.get("id").and_then(|x| x.as_str()).and_then(|s| Uuid::parse_str(s).ok()))
        .bind(target_library_id)
        .bind(str_of(v, "page_slug", ""))
        .bind(v.get("project_id").and_then(|x| x.as_str()).and_then(|s| Uuid::parse_str(s).ok()))
        .bind(v.get("doc_id").and_then(|x| x.as_str()).and_then(|s| Uuid::parse_str(s).ok()))
        .bind(str_of(v, "anchor", ""))
        .bind(ts(v, "created_at"))
        .execute(pool)
        .await?;
        if res.rows_affected() > 0 {
            imported += 1;
        } else {
            skipped += 1;
        }
    }
    Ok((imported, skipped))
}

//! wiki_promotions 仓储：项目文档 → wiki 知识晋升登记（EN-59）。
//!
//! UNIQUE(project_id, doc_id, page_slug)：同一来源文档对同一页只登记一次；
//! 重复插入走 `ON CONFLICT DO NOTHING` + `RETURNING` 探测，冲突转 [`StoreError::Conflict`]
//! 由服务层转成友好「已晋升」提示。
//! 级联：wiki_libraries / projects / project_docs 行删除时 DB 级 CASCADE 清登记；
//! wiki 页删除（page_slug 是文本无 FK）由服务层显式调 [`delete_by_page`]。

use crate::error::{StoreError, StoreResult};
use crate::models::wiki_promotions::WikiPromotionDto;
use sqlx::Row;
use sqlx::PgPool;
use uuid::Uuid;

/// 登记一次晋升。返回新登记行 id；该 (project, doc, page) 已登记时返回
/// [`StoreError::Conflict`]（服务层转「已晋升」友好提示）。
pub async fn insert(
    pool: &PgPool,
    library_id: Uuid,
    page_slug: &str,
    project_id: Uuid,
    doc_id: Uuid,
    anchor: &str,
) -> StoreResult<Uuid> {
    let row = sqlx::query(
        "INSERT INTO wiki_promotions (library_id, page_slug, project_id, doc_id, anchor) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (project_id, doc_id, page_slug) DO NOTHING \
         RETURNING id",
    )
    .bind(library_id)
    .bind(page_slug)
    .bind(project_id)
    .bind(doc_id)
    .bind(anchor)
    .fetch_optional(pool)
    .await?;
    // 冲突时 DO NOTHING → RETURNING 空集 → None：显式转 Conflict（fetch_one 会误报 RowNotFound）
    match row {
        Some(r) => Ok(r.get("id")),
        None => Err(StoreError::Conflict("该晋升组合已被登记".into())),
    }
}

/// 按项目列全部晋升登记（谁家的哪些知识出嫁了）。
pub async fn list_by_project(
    pool: &PgPool,
    project_id: Uuid,
) -> StoreResult<Vec<WikiPromotionDto>> {
    Ok(sqlx::query_as::<_, WikiPromotionDto>(
        "SELECT * FROM wiki_promotions WHERE project_id = $1 ORDER BY created_at DESC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?)
}

/// 按库 + 页 slug 列全部登记（一个 synthesis 页可承接多个来源文档的晋升）。
pub async fn list_by_page(
    pool: &PgPool,
    library_id: Uuid,
    page_slug: &str,
) -> StoreResult<Vec<WikiPromotionDto>> {
    Ok(sqlx::query_as::<_, WikiPromotionDto>(
        "SELECT * FROM wiki_promotions WHERE library_id = $1 AND page_slug = $2 \
         ORDER BY created_at DESC",
    )
    .bind(library_id)
    .bind(page_slug)
    .fetch_all(pool)
    .await?)
}

/// wiki 页删除时清理其全部登记（page_slug 无 FK，服务层显式调用）。
pub async fn delete_by_page(pool: &PgPool, library_id: Uuid, page_slug: &str) -> StoreResult<u64> {
    let r = sqlx::query("DELETE FROM wiki_promotions WHERE library_id = $1 AND page_slug = $2")
        .bind(library_id)
        .bind(page_slug)
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

/// 全量列表（缺省视图；project/library 过滤在服务层分流）。
pub async fn list_all(pool: &PgPool) -> StoreResult<Vec<WikiPromotionDto>> {
    Ok(sqlx::query_as::<_, WikiPromotionDto>(
        "SELECT * FROM wiki_promotions ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?)
}

/// 库 id → slug（promote 编排回链展示用）。
pub async fn library_slug(pool: &PgPool, library_id: Uuid) -> StoreResult<String> {
    sqlx::query_scalar("SELECT slug FROM wiki_libraries WHERE id = $1")
        .bind(library_id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

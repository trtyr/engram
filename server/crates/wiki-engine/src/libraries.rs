//! Wiki 多库管理（0037）：库是一级命名空间——页面/双链/原料/文档/审查/洞察/purpose
//! 全部挂库。本模块负责库本身的生命周期：列表/解析/建库/改名/删库。
//! 库数据隔离靠 FK ON DELETE CASCADE + 各查询按 library_id 过滤。

use sqlx::PgPool;
use uuid::Uuid;

use crate::service::WikiError;

/// 库摘要（带 pages/sources 计数，供 UI 展示与删除前确认）。
#[derive(Debug, serde::Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct WikiLibraryDto {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// 库内页面数（wiki_pages）
    pub pages: i64,
    /// 库内原料数（wiki_sources）
    pub sources: i64,
}

/// 列出全部库（带 pages/sources 计数；LEFT JOIN 聚合子查询，防双 JOIN 笛卡尔积虚增）。
/// 注意：按契约本函数不返回 Result——存储层故障此处 panic（列表是只读聚合，失败即基础设施故障）。
pub async fn list(pool: &PgPool) -> Vec<WikiLibraryDto> {
    sqlx::query_as::<_, WikiLibraryDto>(
        "SELECT l.id, l.slug, l.name, l.created_at, \
         COALESCE(p.cnt, 0) AS pages, COALESCE(s.cnt, 0) AS sources \
         FROM wiki_libraries l \
         LEFT JOIN (SELECT library_id, count(*) AS cnt FROM wiki_pages GROUP BY library_id) p \
           ON p.library_id = l.id \
         LEFT JOIN (SELECT library_id, count(*) AS cnt FROM wiki_sources GROUP BY library_id) s \
           ON s.library_id = l.id \
         ORDER BY l.created_at ASC, l.slug ASC",
    )
    .fetch_all(pool)
    .await
    .expect("查询 wiki 库列表失败")
}

/// 解析库标识 → 库 id。None/空串/"main" → 主库；未知名 → NotFound（文案给下一步指引）。
pub async fn resolve(pool: &PgPool, slug: Option<&str>) -> Result<Uuid, WikiError> {
    // 主库快捷路径：缺省/空串/"main" 一律落主库
    let name = slug.map(str::trim).filter(|s| !s.is_empty());
    let name = match name {
        None => "main",
        Some(s) => s,
    };
    let id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM wiki_libraries WHERE slug = $1")
        .bind(name)
        .fetch_optional(pool)
        .await?;
    id.ok_or_else(|| {
        WikiError::NotFound(format!("库 {name} 不存在——先用 libraries 操作列出可用库"))
    })
}

/// slug 校验：^[a-z0-9][a-z0-9-]{0,39}$（手写校验，免引 regex 依赖）。
fn valid_slug(slug: &str) -> bool {
    let b = slug.as_bytes();
    if b.is_empty() || b.len() > 40 {
        return false;
    }
    let first = b[0];
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    b[1..]
        .iter()
        .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// 建库。slug 非法 → BadRequest；撞名 → BadRequest。
pub async fn create(pool: &PgPool, slug: &str, name: &str) -> Result<WikiLibraryDto, WikiError> {
    let slug = slug.trim();
    if !valid_slug(slug) {
        return Err(WikiError::BadRequest(
            "slug 非法：仅允许小写字母/数字开头，含小写字母/数字/连字符，长度 1-40".into(),
        ));
    }
    let id = Uuid::now_v7();
    let row = sqlx::query_as::<_, WikiLibraryDto>(
        "INSERT INTO wiki_libraries (id, slug, name) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (slug) DO NOTHING \
         RETURNING id, slug, name, created_at, 0::bigint AS pages, 0::bigint AS sources",
    )
    .bind(id)
    .bind(slug)
    .bind(name)
    .fetch_optional(pool)
    .await?;
    row.ok_or_else(|| WikiError::BadRequest(format!("库 {slug} 已存在")))
}

/// 改库显示名（slug 不可改——它是解析入口）。
pub async fn rename(pool: &PgPool, id: Uuid, name: &str) -> Result<(), WikiError> {
    let n = sqlx::query("UPDATE wiki_libraries SET name = $2 WHERE id = $1")
        .bind(id)
        .bind(name)
        .execute(pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(WikiError::NotFound("库不存在".into()));
    }
    Ok(())
}

/// 删库。非空且 !force → BadRequest；force 时按库清空全部 wiki 数据 + purpose 键，
/// 再删库行（FK CASCADE 兜底），返回删除清单（审计行不必——库行本身即凭证）。
pub async fn delete(
    pool: &PgPool,
    slug: &str,
    force: bool,
) -> Result<serde_json::Value, WikiError> {
    let id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM wiki_libraries WHERE slug = $1")
        .bind(slug)
        .fetch_optional(pool)
        .await?;
    let Some(id) = id else {
        return Err(WikiError::NotFound(format!(
            "库 {slug} 不存在——先用 libraries 操作列出可用库"
        )));
    };
    // 主库不可删除：resolve(None)/全部缺省调用都落在 main 上，删掉它整条默认链路即断
    if slug == "main" {
        return Err(WikiError::BadRequest(
            "主库（main）不可删除——它是全部缺省调用的落点".into(),
        ));
    }

    let pages: i64 = sqlx::query_scalar("SELECT count(*) FROM wiki_pages WHERE library_id = $1")
        .bind(id)
        .fetch_one(pool)
        .await?;
    let sources: i64 =
        sqlx::query_scalar("SELECT count(*) FROM wiki_sources WHERE library_id = $1")
            .bind(id)
            .fetch_one(pool)
            .await?;
    let documents: i64 =
        sqlx::query_scalar("SELECT count(*) FROM wiki_documents WHERE library_id = $1")
            .bind(id)
            .fetch_one(pool)
            .await?;

    if pages + sources + documents > 0 && !force {
        return Err(WikiError::BadRequest(format!(
            "库非空（{pages} 页/{sources} 源）——先清空或 force=true"
        )));
    }

    // force：逐表按库清（FK CASCADE 兜底，显式删可读且不依赖级联配置）
    for table in [
        "wiki_pages",
        "wiki_links",
        "wiki_sources",
        "wiki_documents",
        "wiki_chunks",
        "wiki_review_items",
        "wiki_page_versions",
        "wiki_insight_dismissals",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE library_id = $1"))
            .bind(id)
            .execute(pool)
            .await?;
    }
    // purpose 键按库删除（settings 不挂 FK，手动清）
    sqlx::query("DELETE FROM settings WHERE key = $1")
        .bind(format!("wiki_purpose:{id}"))
        .execute(pool)
        .await?;
    // 删库行本体
    sqlx::query("DELETE FROM wiki_libraries WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;

    Ok(serde_json::json!({
        "deleted": slug,
        "pages": pages,
        "sources": sources,
        "documents": documents,
    }))
}

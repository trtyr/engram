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

/// 不可失败（架构治理 task-5 分类 A：不可失败，保留并注明理由）。
#[allow(clippy::expect_used)]
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

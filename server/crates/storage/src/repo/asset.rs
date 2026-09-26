//! 资产域仓储：`assets` 表 CRUD + 名称/别名占用检查。
//!
//! 约定同本目录其它域：函数不持状态、以 `&PgPool` 起参；「不存在」返回 `Option`，
//! 唯一冲突由服务层先查后写、并发窗口落 [`StoreError::Conflict`]。

use uuid::Uuid;

use crate::PgPool;
use crate::error::StoreResult;
use crate::models::asset::AssetDto;

const ASSET_COLS: &str = "id, kind, name, aliases, ip, os, note, fields, created_at, updated_at";

#[allow(clippy::too_many_arguments)]
pub async fn insert_asset(
    pool: &PgPool,
    id: Uuid,
    kind: &str,
    name: &str,
    aliases: &[String],
    ip: &str,
    os: &str,
    note: &str,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "INSERT INTO assets (id, kind, name, aliases, ip, os, note) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         ON CONFLICT (name) DO NOTHING",
    )
    .bind(id)
    .bind(kind)
    .bind(name)
    .bind(aliases)
    .bind(ip)
    .bind(os)
    .bind(note)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 列表：可按类型过滤；`q` 命中 名称 / 别名 / IP 三处（大小写不敏感）。
pub async fn list_assets(
    pool: &PgPool,
    kind: Option<&str>,
    q: Option<&str>,
) -> StoreResult<Vec<AssetDto>> {
    sqlx::query_as::<_, AssetDto>(&format!(
        "SELECT {ASSET_COLS} FROM assets \
         WHERE ($1::text IS NULL OR kind = $1) \
           AND ($2::text IS NULL \
                OR name ILIKE '%' || $2 || '%' \
                OR ip = $2 \
                OR EXISTS (SELECT 1 FROM unnest(aliases) a WHERE a ILIKE '%' || $2 || '%')) \
         ORDER BY kind, name"
    ))
    .bind(kind)
    .bind(q)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn get_asset(pool: &PgPool, id: Uuid) -> StoreResult<Option<AssetDto>> {
    sqlx::query_as::<_, AssetDto>(&format!("SELECT {ASSET_COLS} FROM assets WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

/// 名称或别名 → 资产 id（引用解析与占用检查共用；大小写不敏感）。
/// `exclude_id` 用于更新场景排除自身。
pub async fn find_by_name_or_alias(
    pool: &PgPool,
    key: &str,
    exclude_id: Option<Uuid>,
) -> StoreResult<Option<Uuid>> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM assets \
         WHERE (lower(name) = lower($1) \
                OR EXISTS (SELECT 1 FROM unnest(aliases) a WHERE lower(a) = lower($1))) \
           AND ($2::uuid IS NULL OR id <> $2) \
         LIMIT 1",
    )
    .bind(key)
    .bind(exclude_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

/// 全量替换式更新（补丁语义由服务层拼好再调）。
#[allow(clippy::too_many_arguments)]
pub async fn update_asset(
    pool: &PgPool,
    id: Uuid,
    kind: &str,
    name: &str,
    aliases: &[String],
    ip: &str,
    os: &str,
    note: &str,
    fields: Option<&serde_json::Value>,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE assets SET kind = $2, name = $3, aliases = $4, ip = $5, os = $6, note = $7, \
         fields = COALESCE($8, fields), updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(kind)
    .bind(name)
    .bind(aliases)
    .bind(ip)
    .bind(os)
    .bind(note)
    .bind(fields)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn delete_asset(pool: &PgPool, id: Uuid) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM assets WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// 该资产被哪些项目引用（反查；`project_locations.asset_id` 为真引用）。
pub async fn projects_using(pool: &PgPool, asset_id: Uuid) -> StoreResult<Vec<AssetUsageRow>> {
    sqlx::query_as::<_, AssetUsageRow>(
        "SELECT l.id AS location_id, l.project_id, p.name AS project_name, l.host, l.path, \
                coalesce(l.purpose, '') AS purpose \
           FROM project_locations l JOIN projects p ON p.id = l.project_id \
          WHERE l.asset_id = $1 \
          ORDER BY p.name, l.sort_order",
    )
    .bind(asset_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// 反查行（资产 → 用到它的项目位置）。
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct AssetUsageRow {
    pub location_id: Uuid,
    pub project_id: Uuid,
    pub project_name: String,
    pub host: String,
    pub path: String,
    pub purpose: String,
}

/// 项目用到的资产（项目 → 资产方向，含经由哪条位置登记引用）——项目详情的「关系」区数据源。
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ProjectAssetRow {
    pub asset_id: Uuid,
    pub kind: String,
    pub name: String,
    pub ip: String,
    pub os: String,
    pub location_id: Uuid,
    pub host: String,
    pub path: String,
    pub purpose: String,
}

/// 项目 → 资产引用对（关系图谱用：只要两端 id）。
#[derive(Debug, Clone, Copy, serde::Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ProjectAssetPair {
    pub project_id: Uuid,
    pub asset_id: Uuid,
}

/// 全量「项目 → 资产」引用对（图谱一次取全；去重到 pair 粒度）。
pub async fn all_project_asset_pairs(pool: &PgPool) -> StoreResult<Vec<ProjectAssetPair>> {
    sqlx::query_as::<_, ProjectAssetPair>(
        "SELECT DISTINCT l.project_id, l.asset_id \
           FROM project_locations l \
          WHERE l.asset_id IS NOT NULL \
          ORDER BY l.project_id, l.asset_id",
    )
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// 某个项目用到的全部资产（按位置登记聚合；`asset_id` 为真引用，NULL 的位置不出现）。
pub async fn assets_used_by_project(
    pool: &PgPool,
    project_id: Uuid,
) -> StoreResult<Vec<ProjectAssetRow>> {
    sqlx::query_as::<_, ProjectAssetRow>(
        "SELECT a.id AS asset_id, a.kind, a.name, a.ip, a.os, \
                l.id AS location_id, l.host, l.path, coalesce(l.purpose, '') AS purpose \
           FROM project_locations l JOIN assets a ON a.id = l.asset_id \
          WHERE l.project_id = $1 \
          ORDER BY a.kind, a.name, l.sort_order",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// 当前 runbook 正文（None = 资产不存在）。
pub async fn get_runbook(pool: &PgPool, id: Uuid) -> StoreResult<Option<String>> {
    let r: Option<String> = sqlx::query_scalar("SELECT runbook_md FROM assets WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(r)
}

/// 保存 runbook（同事务：旧文进修订史）。返回 false = 资产不存在。
pub async fn save_runbook(pool: &PgPool, id: Uuid, md: &str, editor: &str) -> StoreResult<bool> {
    let mut tx = pool.begin().await?;
    let cur: Option<String> = sqlx::query_scalar("SELECT runbook_md FROM assets WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(cur) = cur else {
        return Ok(false);
    };
    // 空串 = 从未有手册（初始态），不记为版本——史里只留真实旧文
    if !cur.is_empty() {
        sqlx::query(
            "INSERT INTO asset_revisions (id, asset_id, old_runbook_md, edited_by) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(Uuid::now_v7())
        .bind(id)
        .bind(cur)
        .bind(editor)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE assets SET runbook_md = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(md)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(true)
}

/// 修订史清单（新→旧；old_runbook_md = 该次保存前的正文）。
pub async fn runbook_versions(
    pool: &PgPool,
    id: Uuid,
) -> StoreResult<Vec<crate::models::asset::AssetRevisionRow>> {
    sqlx::query_as::<_, crate::models::asset::AssetRevisionRow>(
        "SELECT id, asset_id, old_runbook_md, edited_by, created_at \
         FROM asset_revisions WHERE asset_id = $1 ORDER BY created_at DESC",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// 单条修订（回滚取旧文用）。
pub async fn get_runbook_version(
    pool: &PgPool,
    vid: Uuid,
) -> StoreResult<Option<crate::models::asset::AssetRevisionRow>> {
    sqlx::query_as::<_, crate::models::asset::AssetRevisionRow>(
        "SELECT id, asset_id, old_runbook_md, edited_by, created_at FROM asset_revisions WHERE id = $1",
    )
    .bind(vid)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

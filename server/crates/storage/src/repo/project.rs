//! 项目记忆域仓储：projects / project_locations / project_docs 三表 CRUD。
//!
//! 事务说明：本域无跨表事务需求；重名/占用的唯一性预检由服务层先行（先查后写），
//! 并发窗口撞唯一约束时落 [`StoreError::Conflict`]。

use uuid::Uuid;

use crate::PgPool;
use crate::error::StoreResult;
use crate::models::project::{
    ProjectDocDto, ProjectDto, ProjectFileDto, ProjectLinkDto, ProjectLocationDto,
};
use chrono::{DateTime, Utc};

const PROJECT_COLS: &str =
    "id, name, type, status, description, categories, frontmatter, created_at, updated_at";
const LOCATION_COLS: &str =
    "id, project_id, ip, host, os, path, purpose, sort_order, asset_id, created_at, updated_at";
const DOC_COLS: &str = "id, project_id, category, folder, title, content, frontmatter, version, created_at, updated_at";

pub async fn insert_project(
    pool: &PgPool,
    id: Uuid,
    name: &str,
    type_: &str,
    description: Option<&str>,
    categories: &[String],
) -> StoreResult<u64> {
    let res = sqlx::query(
        "INSERT INTO projects (id, name, type, status, description, categories) \
         VALUES ($1, $2, $3, 'active', $4, $5) \
         ON CONFLICT (name) DO NOTHING",
    )
    .bind(id)
    .bind(name)
    .bind(type_)
    .bind(description)
    .bind(sqlx::types::Json(categories))
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn list_projects(
    pool: &PgPool,
    type_filter: Option<&str>,
) -> StoreResult<Vec<ProjectDto>> {
    let rows =
        match type_filter {
            Some(t) => sqlx::query_as::<_, ProjectDto>(&format!(
                "SELECT {PROJECT_COLS} FROM projects WHERE type = $1 ORDER BY created_at DESC, name"
            ))
            .bind(t)
            .fetch_all(pool)
            .await?,
            None => {
                sqlx::query_as::<_, ProjectDto>(&format!(
                    "SELECT {PROJECT_COLS} FROM projects ORDER BY created_at DESC, name"
                ))
                .fetch_all(pool)
                .await?
            }
        };
    Ok(rows)
}

pub async fn get_project(pool: &PgPool, id: Uuid) -> StoreResult<Option<ProjectDto>> {
    sqlx::query_as::<_, ProjectDto>(&format!(
        "SELECT {PROJECT_COLS} FROM projects WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

/// 按名精确定位项目 id（项目名唯一）。
pub async fn id_by_name(pool: &PgPool, name: &str) -> StoreResult<Option<Uuid>> {
    sqlx::query_scalar("SELECT id FROM projects WHERE name = $1")
        .bind(name)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

/// 同名项目占用检查（排除自身）。
pub async fn name_holder(pool: &PgPool, name: &str, exclude_id: Uuid) -> StoreResult<Option<Uuid>> {
    sqlx::query_scalar("SELECT id FROM projects WHERE name = $1 AND id <> $2")
        .bind(name)
        .bind(exclude_id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn update_project(
    pool: &PgPool,
    id: Uuid,
    name: &str,
    status: &str,
    description: Option<&str>,
    categories: &[String],
) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE projects SET name = $2, status = $3, description = $4, categories = $5, \
         updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(name)
    .bind(status)
    .bind(description)
    .bind(sqlx::types::Json(categories))
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn delete_project(pool: &PgPool, id: Uuid) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// 存量 id 集合（批量删除前区分「删掉」与「本就不存在」）。
pub async fn existing_ids(pool: &PgPool, ids: &[Uuid]) -> StoreResult<Vec<Uuid>> {
    let rows: Vec<(Uuid,)> = sqlx::query_as("SELECT id FROM projects WHERE id = ANY($1)")
        .bind(ids)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

pub async fn delete_projects(pool: &PgPool, ids: &[Uuid]) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM projects WHERE id = ANY($1)")
        .bind(ids)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

pub async fn list_locations(
    pool: &PgPool,
    project_id: Uuid,
) -> StoreResult<Vec<ProjectLocationDto>> {
    sqlx::query_as::<_, ProjectLocationDto>(&format!(
        "SELECT {LOCATION_COLS} FROM project_locations WHERE project_id = $1 ORDER BY sort_order, created_at"
    ))
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_location(
    pool: &PgPool,
    id: Uuid,
    project_id: Uuid,
    ip: &str,
    host: &str,
    os: &str,
    path: &str,
    purpose: Option<&str>,
    asset_id: Option<Uuid>,
) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO project_locations (id, project_id, ip, host, os, path, purpose, asset_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(project_id)
    .bind(ip)
    .bind(host)
    .bind(os)
    .bind(path)
    .bind(purpose)
    .bind(asset_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn update_location(
    pool: &PgPool,
    id: Uuid,
    ip: &str,
    host: &str,
    os: &str,
    path: &str,
    purpose: Option<&str>,
    asset_id: Option<Uuid>,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE project_locations SET ip = $2, host = $3, os = $4, path = $5, purpose = $6, \
         asset_id = $7, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(ip)
    .bind(host)
    .bind(os)
    .bind(path)
    .bind(purpose)
    .bind(asset_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn delete_location(pool: &PgPool, id: Uuid) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM project_locations WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

pub async fn get_location(pool: &PgPool, id: Uuid) -> StoreResult<Option<ProjectLocationDto>> {
    sqlx::query_as::<_, ProjectLocationDto>(&format!(
        "SELECT {LOCATION_COLS} FROM project_locations WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub async fn list_docs(pool: &PgPool, project_id: Uuid) -> StoreResult<Vec<ProjectDocDto>> {
    sqlx::query_as::<_, ProjectDocDto>(&format!(
        "SELECT {DOC_COLS} FROM project_docs WHERE project_id = $1 ORDER BY category, created_at"
    ))
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// 文档正文行（grep 式检索的原始行）：(id, title, category, content)。
pub async fn list_doc_contents(
    pool: &PgPool,
    project_id: Uuid,
) -> StoreResult<Vec<(Uuid, String, String, String)>> {
    sqlx::query_as(
        "SELECT id, title, category, content FROM project_docs WHERE project_id = $1 ORDER BY category, created_at",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn insert_doc(
    pool: &PgPool,
    id: Uuid,
    project_id: Uuid,
    category: &str,
    folder: &str,
    title: &str,
    content: &str,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "INSERT INTO project_docs (id, project_id, category, folder, title, content) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (project_id, category, folder, title) DO NOTHING",
    )
    .bind(id)
    .bind(project_id)
    .bind(category)
    .bind(folder)
    .bind(title)
    .bind(content)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 部分更新（COALESCE）：None 字段保持原值——消除读-改-写并发丢字段窗口
/// （MCP 黑盒测试 D2：两个并发 update 各改不同字段时后者曾整行覆盖前者）。
pub async fn update_doc(
    pool: &PgPool,
    id: Uuid,
    category: Option<&str>,
    folder: Option<&str>,
    title: Option<&str>,
    content: Option<&str>,
    expected_version: Option<i64>,
) -> StoreResult<u64> {
    // 乐观锁（公网多Agent P001 步骤2）：expected_version 给出时原子校验当前版本，
    // 不匹配则 0 行更新（core 层转 409 Conflict）；每次成功写入 version + 1。
    let res = sqlx::query(
        "UPDATE project_docs SET category = COALESCE($2, category), folder = COALESCE($3, folder), \
         title = COALESCE($4, title), content = COALESCE($5, content), version = version + 1, updated_at = now() \
         WHERE id = $1 AND ($6::bigint IS NULL OR version = $6)",
    )
    .bind(id)
    .bind(category)
    .bind(folder)
    .bind(title)
    .bind(content)
    .bind(expected_version)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn delete_doc(pool: &PgPool, id: Uuid) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM project_docs WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

pub async fn get_doc(pool: &PgPool, id: Uuid) -> StoreResult<Option<ProjectDocDto>> {
    sqlx::query_as::<_, ProjectDocDto>(&format!(
        "SELECT {DOC_COLS} FROM project_docs WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

// ---------- project_files（0045 项目文件：架构图 HTML 等制品） ----------

const FILE_COLS: &str = "id, project_id, name, mime, content, version, created_at, updated_at";

pub async fn list_files(pool: &PgPool, project_id: Uuid) -> StoreResult<Vec<ProjectFileDto>> {
    let rows = sqlx::query_as::<_, ProjectFileDto>(&format!(
        "SELECT {FILE_COLS} FROM project_files WHERE project_id = $1 ORDER BY name"
    ))
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get_file_by_name(
    pool: &PgPool,
    project_id: Uuid,
    name: &str,
) -> StoreResult<Option<ProjectFileDto>> {
    let row = sqlx::query_as::<_, ProjectFileDto>(&format!(
        "SELECT {FILE_COLS} FROM project_files WHERE project_id = $1 AND name = $2"
    ))
    .bind(project_id)
    .bind(name)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 覆盖式写入：存在则 version+1 并把旧内容存版本快照，不存在则插入 v1
pub async fn upsert_file(
    pool: &PgPool,
    project_id: Uuid,
    name: &str,
    mime: &str,
    content: &str,
) -> StoreResult<ProjectFileDto> {
    let existing = get_file_by_name(pool, project_id, name).await?;
    if let Some(prev) = existing {
        let new_version = prev.version + 1;
        sqlx::query(
            "INSERT INTO project_file_versions (file_id, version, content) VALUES ($1, $2, $3)",
        )
        .bind(prev.id)
        .bind(prev.version)
        .bind(&prev.content)
        .execute(pool)
        .await?;
        let row = sqlx::query_as::<_, ProjectFileDto>(&format!(
            "UPDATE project_files SET mime = $3, content = $4, version = $5, updated_at = now() \
                 WHERE id = $1 AND project_id = $2 RETURNING {FILE_COLS}"
        ))
        .bind(prev.id)
        .bind(project_id)
        .bind(mime)
        .bind(content)
        .bind(new_version)
        .fetch_one(pool)
        .await?;
        Ok(row)
    } else {
        let row = sqlx::query_as::<_, ProjectFileDto>(&format!(
            "INSERT INTO project_files (project_id, name, mime, content) \
                 VALUES ($1, $2, $3, $4) RETURNING {FILE_COLS}"
        ))
        .bind(project_id)
        .bind(name)
        .bind(mime)
        .bind(content)
        .fetch_one(pool)
        .await?;
        Ok(row)
    }
}

pub async fn delete_file(pool: &PgPool, project_id: Uuid, name: &str) -> StoreResult<u64> {
    let r = sqlx::query("DELETE FROM project_files WHERE project_id = $1 AND name = $2")
        .bind(project_id)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

pub async fn list_file_versions(
    pool: &PgPool,
    file_id: Uuid,
) -> StoreResult<Vec<(i32, DateTime<Utc>)>> {
    let rows = sqlx::query_as::<_, (i32, DateTime<Utc>)>(
        "SELECT version, created_at FROM project_file_versions WHERE file_id = $1 ORDER BY version DESC",
    )
    .bind(file_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get_file_version(
    pool: &PgPool,
    file_id: Uuid,
    version: i32,
) -> StoreResult<Option<String>> {
    let row = sqlx::query_scalar::<_, String>(
        "SELECT content FROM project_file_versions WHERE file_id = $1 AND version = $2",
    )
    .bind(file_id)
    .bind(version)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 晋升标记双写（EN-59）：frontmatter.promoted 数组追加（结构化）+ 正文末尾追加
/// 可见标记行（人读）。单语句原子——两处要么都写要么都不写。
/// 幂等由外层 wiki_promotions 登记的 UNIQUE 保证（标记只在登记成功后调用一次）。
pub async fn mark_doc_promoted(
    pool: &PgPool,
    doc_id: Uuid,
    wiki_ref: &str,
    anchor: &str,
) -> StoreResult<()> {
    let marker = format!("\n\n⛳ 本文「{anchor}」已晋升为 wiki:{wiki_ref}");
    sqlx::query(
        "UPDATE project_docs SET \
           frontmatter = frontmatter || jsonb_build_object('promoted', \
             (COALESCE(frontmatter->'promoted', '[]'::jsonb) || jsonb_build_object(\
               'wiki', $2, 'anchor', $3, 'at', now()))), \
           content = content || $4, \
           updated_at = now() \
         WHERE id = $1",
    )
    .bind(doc_id)
    .bind(wiki_ref)
    .bind(anchor)
    .bind(marker)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------- 项目关联（project_links，0058） ----------

/// 关联查询的公共 SELECT（两侧项目名一起带出，前端/CLI 免二次查询）。
const LINK_SELECT: &str = "SELECT l.id, l.from_project, f.name AS from_name, l.to_project, \
     t.name AS to_name, l.kind, l.note, l.created_at \
     FROM project_links l \
     JOIN projects f ON f.id = l.from_project \
     JOIN projects t ON t.id = l.to_project";

/// 建关联（同向同类重复由唯一索引兜底；调用方先查后写）。
pub async fn insert_link(
    pool: &PgPool,
    id: Uuid,
    from_project: Uuid,
    to_project: Uuid,
    kind: &str,
    note: &str,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "INSERT INTO project_links (id, from_project, to_project, kind, note) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (from_project, to_project, kind) DO NOTHING",
    )
    .bind(id)
    .bind(from_project)
    .bind(to_project)
    .bind(kind)
    .bind(note)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn get_link(pool: &PgPool, id: Uuid) -> StoreResult<Option<ProjectLinkDto>> {
    sqlx::query_as::<_, ProjectLinkDto>(&format!("{LINK_SELECT} WHERE l.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

/// 某项目的全部关联（**两向合并**：它作为起点或终点的都回，按 kind/项目名排序）。
pub async fn list_links(pool: &PgPool, project_id: Uuid) -> StoreResult<Vec<ProjectLinkDto>> {
    sqlx::query_as::<_, ProjectLinkDto>(&format!(
        "{LINK_SELECT} WHERE l.from_project = $1 OR l.to_project = $1 \
         ORDER BY l.kind, f.name, t.name"
    ))
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// 全量关联（关系图谱用——一次取全，前端不跑 N+1）。
pub async fn list_all_links(pool: &PgPool) -> StoreResult<Vec<ProjectLinkDto>> {
    sqlx::query_as::<_, ProjectLinkDto>(&format!("{LINK_SELECT} ORDER BY l.kind, f.name, t.name"))
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

/// 同向同类是否已存在（冲突预检）。
pub async fn find_link(
    pool: &PgPool,
    from_project: Uuid,
    to_project: Uuid,
    kind: &str,
) -> StoreResult<Option<Uuid>> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM project_links \
          WHERE from_project = $1 AND to_project = $2 AND kind = $3 LIMIT 1",
    )
    .bind(from_project)
    .bind(to_project)
    .bind(kind)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub async fn delete_link(pool: &PgPool, id: Uuid) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM project_links WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

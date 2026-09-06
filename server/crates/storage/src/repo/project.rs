//! 项目记忆域仓储：projects / project_locations / project_docs 三表 CRUD。
//!
//! 事务说明：本域无跨表事务需求；重名/占用的唯一性预检由服务层先行（先查后写），
//! 并发窗口撞唯一约束时落 [`StoreError::Conflict`]。

use uuid::Uuid;

use crate::PgPool;
use crate::error::StoreResult;
use crate::models::project::{ProjectDocDto, ProjectDto, ProjectLocationDto};

const PROJECT_COLS: &str =
    "id, name, type, status, description, categories, frontmatter, created_at, updated_at";
const LOCATION_COLS: &str =
    "id, project_id, ip, host, os, path, purpose, sort_order, created_at, updated_at";
const DOC_COLS: &str =
    "id, project_id, category, title, content, frontmatter, created_at, updated_at";

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
) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO project_locations (id, project_id, ip, host, os, path, purpose) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(project_id)
    .bind(ip)
    .bind(host)
    .bind(os)
    .bind(path)
    .bind(purpose)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_location(
    pool: &PgPool,
    id: Uuid,
    ip: &str,
    host: &str,
    os: &str,
    path: &str,
    purpose: Option<&str>,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE project_locations SET ip = $2, host = $3, os = $4, path = $5, purpose = $6, updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(ip)
    .bind(host)
    .bind(os)
    .bind(path)
    .bind(purpose)
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
    title: &str,
    content: &str,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "INSERT INTO project_docs (id, project_id, category, title, content) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (project_id, category, title) DO NOTHING",
    )
    .bind(id)
    .bind(project_id)
    .bind(category)
    .bind(title)
    .bind(content)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn update_doc(
    pool: &PgPool,
    id: Uuid,
    category: &str,
    title: &str,
    content: &str,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE project_docs SET category = $2, title = $3, content = $4, updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(category)
    .bind(title)
    .bind(content)
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

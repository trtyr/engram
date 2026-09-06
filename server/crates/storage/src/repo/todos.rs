//! 待办域仓储（0035）：不绑定项目的临时任务/灵感速记，速记→做完勾掉。

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::PgPool;
use crate::error::StoreResult;

/// 待办行（sqlx FromRow 由调用方按需 derive；此处返回元组避免重复结构体）。
pub type TodoTuple = (
    Uuid,
    String,
    String,
    String,
    String,
    Vec<String>,
    Option<DateTime<Utc>>,
    Option<String>,
    Option<DateTime<Utc>>,
    DateTime<Utc>,
    DateTime<Utc>,
);

const COLS: &str = "id, title, body, status, priority, tags, due_at, project_hint, done_at, created_at, updated_at";

pub async fn insert(
    pool: &PgPool,
    id: Uuid,
    title: &str,
    body: &str,
    priority: &str,
    tags: &[String],
    due_at: Option<DateTime<Utc>>,
    project_hint: Option<&str>,
) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO todos (id, title, body, status, priority, tags, due_at, project_hint)          VALUES ($1, $2, $3, 'open', $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(title)
    .bind(body)
    .bind(priority)
    .bind(tags)
    .bind(due_at)
    .bind(project_hint)
    .execute(pool)
    .await?;
    Ok(())
}

/// 列表：open 优先，其余按 updated_at 降序；status/priority/tag/q 过滤。
pub async fn list(
    pool: &PgPool,
    status: Option<&str>,
    priority: Option<&str>,
    tag: Option<&str>,
    q: Option<&str>,
    limit: i64,
) -> StoreResult<Vec<TodoTuple>> {
    Ok(sqlx::query_as::<_, TodoTuple>(
        "SELECT id, title, body, status, priority, tags, due_at, project_hint, done_at, created_at, updated_at          FROM todos          WHERE ($1::text IS NULL OR status = $1)            AND ($2::text IS NULL OR priority = $2)            AND ($3::text IS NULL OR tags @> ARRAY[$3::text])            AND ($4::text IS NULL OR title ILIKE '%' || $4 || '%' OR body ILIKE '%' || $4 || '%')          ORDER BY (status = 'open') DESC, updated_at DESC          LIMIT $5",
    )
    .bind(status)
    .bind(priority)
    .bind(tag)
    .bind(q)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

pub async fn get(pool: &PgPool, id: Uuid) -> StoreResult<Option<TodoTuple>> {
    Ok(sqlx::query_as::<_, TodoTuple>(
        "SELECT id, title, body, status, priority, tags, due_at, project_hint, done_at, created_at, updated_at          FROM todos WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

/// 更新：COALESCE 部分更新（None 不动）；status 变更时同步 done_at。
pub async fn update(
    pool: &PgPool,
    id: Uuid,
    title: Option<&str>,
    body: Option<&str>,
    priority: Option<&str>,
    status: Option<&str>,
    due_at: Option<Option<DateTime<Utc>>>,
    project_hint: Option<Option<&str>>,
    tags: Option<&[String]>,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE todos SET             title = COALESCE($2, title),             body = COALESCE($3, body),             priority = COALESCE($4, priority),             status = COALESCE($5, status),             due_at = COALESCE($6, due_at),             project_hint = COALESCE($7, project_hint),             tags = COALESCE($8::text[], tags),             done_at = CASE WHEN $5 = 'done' THEN now() WHEN $5 = 'open' THEN NULL ELSE done_at END,             updated_at = now()          WHERE id = $1",
    )
    .bind(id)
    .bind(title)
    .bind(body)
    .bind(priority)
    .bind(status)
    .bind(due_at)
    .bind(project_hint)
    .bind(tags)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 删除（物理删除；归档语义走 status=archived）。
pub async fn delete(pool: &PgPool, id: Uuid) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM todos WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// 全量导出行（与 TodoTuple 同构；显式别名避免深嵌套泛型）。
pub type ExportRow = (
    Uuid,
    String,
    String,
    String,
    String,
    Vec<String>,
    Option<DateTime<Utc>>,
    Option<String>,
    Option<DateTime<Utc>>,
    DateTime<Utc>,
    DateTime<Utc>,
);

/// 全量导出（P4 数据主权）。
pub async fn export_all(pool: &PgPool) -> StoreResult<Vec<ExportRow>> {
    let rows = sqlx::query_as::<_, ExportRow>(
        "SELECT id, title, body, status, priority, tags, due_at, project_hint, done_at, created_at, updated_at          FROM todos ORDER BY (status = 'open') DESC, updated_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 统一检索（/search）的待办段：open 待办的标题/正文 ILIKE 匹配。
/// 返回 (id, title, body 前 200 字, priority)。
pub async fn search_open(
    pool: &PgPool,
    q: &str,
    limit: i64,
) -> StoreResult<Vec<(Uuid, String, String, String)>> {
    let pattern = format!("%{q}%");
    let rows = sqlx::query_as::<_, (Uuid, String, String, String)>(
        "SELECT id, title, body, priority FROM todos \
         WHERE status = 'open' AND (title ILIKE $1 OR body ILIKE $1) \
         ORDER BY CASE priority WHEN 'high' THEN 0 WHEN 'normal' THEN 1 ELSE 2 END, updated_at DESC \
         LIMIT $2",
    )
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

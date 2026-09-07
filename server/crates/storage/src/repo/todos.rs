//! 待办域仓储（0035）：不绑定项目的临时任务/灵感速记，速记→做完勾掉。

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::PgPool;
use crate::error::StoreResult;

pub struct NewTodo<'a> {
    pub id: Uuid,
    pub title: &'a str,
    pub body: &'a str,
    pub priority: &'a str,
    pub tags: &'a [String],
    pub due_at: Option<DateTime<Utc>>,
    pub project_hint: Option<&'a str>,
}

pub async fn insert(pool: &PgPool, t: &NewTodo<'_>) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO todos (id, title, body, status, priority, tags, due_at, project_hint)          VALUES ($1, $2, $3, 'open', $4, $5, $6, $7)",
    )
    .bind(t.id)
    .bind(t.title)
    .bind(t.body)
    .bind(t.priority)
    .bind(t.tags)
    .bind(t.due_at)
    .bind(t.project_hint)
    .execute(pool)
    .await?;
    Ok(())
}

/// 列表：open 优先，其余按 updated_at 降序；status/priority/tag/q 过滤。
/// cursor（D29 keyset 分页）：(open 标记, updated_at, id) 三元组行比较——排序键
/// 含 open 优先旗标，纯时间游标会在 open/done 分界处丢行。ORDER BY 带 id 决稳。
#[allow(clippy::too_many_arguments)]
pub async fn list(
    pool: &PgPool,
    status: Option<&str>,
    priority: Option<&str>,
    tag: Option<&str>,
    q: Option<&str>,
    cursor: Option<(i32, DateTime<Utc>, Uuid)>,
    limit: i64,
) -> StoreResult<Vec<TodoTuple>> {
    if let Some((flag, ts, id)) = cursor {
        Ok(sqlx::query_as::<_, TodoTuple>(
            "SELECT id, title, body, status, priority, tags, due_at, project_hint, done_at, created_at, updated_at          FROM todos          WHERE ($1::text IS NULL OR status = $1)            AND ($2::text IS NULL OR priority = $2)            AND ($3::text IS NULL OR tags @> ARRAY[$3::text])            AND ($4::text IS NULL OR title ILIKE '%' || $4 || '%' OR body ILIKE '%' || $4 || '%')            AND (CASE WHEN status = 'open' THEN 1 ELSE 0 END, updated_at, id) < ($6::int, $7::timestamptz, $8::uuid)          ORDER BY (status = 'open') DESC, updated_at DESC, id DESC          LIMIT $5",
        )
        .bind(status)
        .bind(priority)
        .bind(tag)
        .bind(q)
        .bind(limit)
        .bind(flag)
        .bind(ts)
        .bind(id)
        .fetch_all(pool)
        .await?)
    } else {
        Ok(sqlx::query_as::<_, TodoTuple>(
            "SELECT id, title, body, status, priority, tags, due_at, project_hint, done_at, created_at, updated_at          FROM todos          WHERE ($1::text IS NULL OR status = $1)            AND ($2::text IS NULL OR priority = $2)            AND ($3::text IS NULL OR tags @> ARRAY[$3::text])            AND ($4::text IS NULL OR title ILIKE '%' || $4 || '%' OR body ILIKE '%' || $4 || '%')          ORDER BY (status = 'open') DESC, updated_at DESC, id DESC          LIMIT $5",
        )
        .bind(status)
        .bind(priority)
        .bind(tag)
        .bind(q)
        .bind(limit)
        .fetch_all(pool)
        .await?)
    }
}

pub async fn get(pool: &PgPool, id: Uuid) -> StoreResult<Option<TodoTuple>> {
    Ok(sqlx::query_as::<_, TodoTuple>(
        "SELECT id, title, body, status, priority, tags, due_at, project_hint, done_at, created_at, updated_at          FROM todos WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

/// 更新：COALESCE 部分更新（None 不动）；status **迁移**时同步 done_at
/// （D17：已是 done 再传 done 是 no-op，不刷新首次完成时间戳；done→open 清空；
/// open→done 盖 now()。todos.status 在 SET 表达式里引用的是更新前的旧值）。
pub struct TodoPatch<'a> {
    pub title: Option<&'a str>,
    pub body: Option<&'a str>,
    pub priority: Option<&'a str>,
    pub status: Option<&'a str>,
    pub due_at: Option<Option<DateTime<Utc>>>,
    pub project_hint: Option<Option<&'a str>>,
    pub tags: Option<&'a [String]>,
}

pub async fn update(pool: &PgPool, id: Uuid, p: &TodoPatch<'_>) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE todos SET             title = COALESCE($2, title),             body = COALESCE($3, body),             priority = COALESCE($4, priority),             status = COALESCE($5, status),             due_at = COALESCE($6, due_at),             project_hint = COALESCE($7, project_hint),             tags = COALESCE($8::text[], tags),             done_at = CASE WHEN $5 = 'done' AND todos.status IS DISTINCT FROM 'done' THEN now() WHEN $5 = 'open' THEN NULL ELSE done_at END,             updated_at = now()          WHERE id = $1",
    )
    .bind(id)
    .bind(p.title)
    .bind(p.body)
    .bind(p.priority)
    .bind(p.status)
    .bind(p.due_at)
    .bind(p.project_hint)
    .bind(p.tags)
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

/// 列表/单查的行类型。
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

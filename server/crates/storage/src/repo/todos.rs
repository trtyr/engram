//! 待办域仓储（0035；0041 双形态）：todo（微软式行动项）/ ticket（工单）。
//! todo 轻量两态（open/done/archived）；ticket 结构化问题跟踪
//! （severity/symptom/reproduce/acceptance/resolution + 五态状态机），状态机由
//! 0041 联合 CHECK 在数据库层强制。

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::PgPool;
use crate::error::StoreResult;

/// 列表/单查/导出统一行类型（FromRow——18 列超出 sqlx tuple 上限，0041 起用结构体）。
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TodoRow {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    pub kind: String,
    pub status: String,
    pub priority: String,
    pub severity: Option<String>,
    pub symptom: String,
    pub reproduce: String,
    pub acceptance: String,
    pub resolution: String,
    pub tags: Vec<String>,
    pub due_at: Option<DateTime<Utc>>,
    pub project_hint: Option<String>,
    pub done_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// 全局单调短号（显示为 EN-<n>；sequence 生成）
    pub short_no: i32,
}

const COLS: &str = "id, title, body, kind, status, priority, severity, symptom, reproduce, acceptance, resolution, tags, due_at, project_hint, done_at, resolved_at, created_at, updated_at, short_no";

pub struct NewTodo<'a> {
    pub id: Uuid,
    pub title: &'a str,
    pub body: &'a str,
    pub kind: &'a str,
    pub priority: &'a str,
    pub severity: Option<&'a str>,
    pub symptom: &'a str,
    pub reproduce: &'a str,
    pub acceptance: &'a str,
    pub tags: &'a [String],
    pub due_at: Option<DateTime<Utc>>,
    pub project_hint: Option<&'a str>,
}

pub async fn insert(pool: &PgPool, t: &NewTodo<'_>) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO todos (id, title, body, kind, status, priority, severity, symptom, reproduce, acceptance, tags, due_at, project_hint) \
         VALUES ($1, $2, $3, $4, 'open', $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(t.id)
    .bind(t.title)
    .bind(t.body)
    .bind(t.kind)
    .bind(t.priority)
    .bind(t.severity)
    .bind(t.symptom)
    .bind(t.reproduce)
    .bind(t.acceptance)
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
    kind: Option<&str>,
    priority: Option<&str>,
    tag: Option<&str>,
    q: Option<&str>,
    cursor: Option<(i32, DateTime<Utc>, Uuid)>,
    limit: i64,
) -> StoreResult<Vec<TodoRow>> {
    if let Some((flag, ts, id)) = cursor {
        Ok(sqlx::query_as::<_, TodoRow>(
            format!(
                "SELECT {COLS} FROM todos \
                 WHERE ($1::text IS NULL OR status = $1) \
                 AND ($2::text IS NULL OR priority = $2) \
                 AND ($3::text IS NULL OR tags @> ARRAY[$3::text]) \
                 AND ($4::text IS NULL OR title ILIKE '%' || $4 || '%' OR body ILIKE '%' || $4 || '%') \
                 AND ($5::text IS NULL OR kind = $5) \
                 AND (CASE WHEN status = 'open' THEN 1 ELSE 0 END, updated_at, id) < ($7::int, $8::timestamptz, $9::uuid) \
                 ORDER BY (status = 'open') DESC, updated_at DESC, id DESC \
                 LIMIT $6"
            )
            .as_str(),
        )
        .bind(status)
        .bind(priority)
        .bind(tag)
        .bind(q)
        .bind(kind)
        .bind(limit)
        .bind(flag)
        .bind(ts)
        .bind(id)
        .fetch_all(pool)
        .await?)
    } else {
        Ok(sqlx::query_as::<_, TodoRow>(
            format!(
                "SELECT {COLS} FROM todos \
                 WHERE ($1::text IS NULL OR status = $1) \
                 AND ($2::text IS NULL OR priority = $2) \
                 AND ($3::text IS NULL OR tags @> ARRAY[$3::text]) \
                 AND ($4::text IS NULL OR title ILIKE '%' || $4 || '%' OR body ILIKE '%' || $4 || '%') \
                 AND ($5::text IS NULL OR kind = $5) \
                 ORDER BY (status = 'open') DESC, updated_at DESC, id DESC \
                 LIMIT $6"
            )
            .as_str(),
        )
        .bind(status)
        .bind(priority)
        .bind(tag)
        .bind(q)
        .bind(kind)
        .bind(limit)
        .fetch_all(pool)
        .await?)
    }
}

pub async fn get(pool: &PgPool, id: Uuid) -> StoreResult<Option<TodoRow>> {
    Ok(
        sqlx::query_as::<_, TodoRow>(format!("SELECT {COLS} FROM todos WHERE id = $1").as_str())
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

/// 更新：COALESCE 部分更新（None 不动）；status **迁移**时同步完成/解决时间戳——
/// todo：done 盖 done_at（首次完成不刷新，D17）、回 open 清空；ticket：resolved 盖
/// resolved_at、回非 resolved 态清空。todos.status 引用更新前旧值。
pub struct TodoPatch<'a> {
    /// 形态转换（todo ↔ ticket；转换时 status/severity 等以新 kind 校验）
    pub kind: Option<&'a str>,
    pub title: Option<&'a str>,
    pub body: Option<&'a str>,
    pub priority: Option<&'a str>,
    pub status: Option<&'a str>,
    pub severity: Option<Option<&'a str>>,
    pub symptom: Option<&'a str>,
    pub reproduce: Option<&'a str>,
    pub acceptance: Option<&'a str>,
    pub resolution: Option<&'a str>,
    pub due_at: Option<Option<DateTime<Utc>>>,
    pub project_hint: Option<Option<&'a str>>,
    pub tags: Option<&'a [String]>,
}

pub async fn update(pool: &PgPool, id: Uuid, p: &TodoPatch<'_>) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE todos SET \
            kind = COALESCE($14, kind),             title = COALESCE($2, title), \
            body = COALESCE($3, body), \
            priority = COALESCE($4, priority), \
            status = COALESCE($5, status), \
            severity = COALESCE($6, severity), \
            symptom = COALESCE($7, symptom), \
            reproduce = COALESCE($8, reproduce), \
            acceptance = COALESCE($9, acceptance), \
            resolution = COALESCE($10, resolution), \
            due_at = COALESCE($11, due_at), \
            project_hint = COALESCE($12, project_hint), \
            tags = COALESCE($13::text[], tags), \
            done_at = CASE \
                WHEN kind = 'todo' AND $5 = 'done' AND todos.status IS DISTINCT FROM 'done' THEN now() \
                WHEN kind = 'todo' AND $5 = 'open' THEN NULL \
                ELSE done_at END, \
            resolved_at = CASE \
                WHEN kind = 'ticket' AND $5 IN ('resolved','verified') AND resolved_at IS NULL THEN now() \
                WHEN kind = 'ticket' AND $5 IN ('open','confirmed','in_progress') THEN NULL \
                ELSE resolved_at END, \
            updated_at = now() \
          WHERE id = $1",
    )
    .bind(id)
    .bind(p.title)
    .bind(p.body)
    .bind(p.priority)
    .bind(p.status)
    .bind(p.severity)
    .bind(p.symptom)
    .bind(p.reproduce)
    .bind(p.acceptance)
    .bind(p.resolution)
    .bind(p.due_at)
    .bind(p.project_hint)
    .bind(p.tags)
    .bind(p.kind)
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

/// 全量导出（P4 数据主权）——与 TodoRow 同构（0041 含工单字段）。
pub async fn export_all(pool: &PgPool) -> StoreResult<Vec<TodoRow>> {
    let rows = sqlx::query_as::<_, TodoRow>(
        format!("SELECT {COLS} FROM todos ORDER BY (status = 'open') DESC, updated_at DESC")
            .as_str(),
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 统一检索（/search）的待办段：open/在办 待办与工单的标题/正文 ILIKE 匹配。
/// 返回 (id, kind, title, body 前 200 字, priority)。
pub async fn search_open(
    pool: &PgPool,
    q: &str,
    limit: i64,
) -> StoreResult<Vec<(Uuid, String, String, String, String)>> {
    let pattern = format!("%{q}%");
    let rows = sqlx::query_as::<_, (Uuid, String, String, String, String)>(
        "SELECT id, kind, title, body, priority FROM todos \
         WHERE status IN ('open','confirmed','in_progress') AND (title ILIKE $1 OR body ILIKE $1) \
         ORDER BY CASE priority WHEN 'high' THEN 0 WHEN 'normal' THEN 1 ELSE 2 END, updated_at DESC \
         LIMIT $2",
    )
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 按短号取 todo（EN-<n> 引用直达）。
pub async fn find_by_short_no(pool: &sqlx::PgPool, short_no: i32) -> Option<TodoRow> {
    sqlx::query_as::<_, TodoRow>("SELECT * FROM todos WHERE short_no = $1")
        .bind(short_no)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

// ---------- 关联关系（工单模型细化：blocked_by / relates_to / parent） ----------

pub struct TodoLink {
    pub from_id: Uuid,
    pub to_id: Uuid,
    pub kind: String,
}

/// 建关联（幂等：重复 link 无变化）。返回是否新插入。
pub async fn link_add(
    pool: &sqlx::PgPool,
    from_id: Uuid,
    to_id: Uuid,
    kind: &str,
) -> Result<bool, sqlx::Error> {
    let n = sqlx::query(
        "INSERT INTO todo_links (from_id, to_id, kind) VALUES ($1, $2, $3) \
         ON CONFLICT (from_id, to_id, kind) DO NOTHING",
    )
    .bind(from_id)
    .bind(to_id)
    .bind(kind)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(n > 0)
}

/// 解除关联。
pub async fn link_remove(
    pool: &sqlx::PgPool,
    from_id: Uuid,
    to_id: Uuid,
    kind: &str,
) -> Result<bool, sqlx::Error> {
    let n = sqlx::query("DELETE FROM todo_links WHERE from_id = $1 AND to_id = $2 AND kind = $3")
        .bind(from_id)
        .bind(to_id)
        .bind(kind)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(n > 0)
}

/// 某 todo 的全部关联（含反向：别人指向它的行，direction 标注）。
pub async fn links_for(
    pool: &sqlx::PgPool,
    id: Uuid,
) -> Result<Vec<(Uuid, Uuid, String, String)>, sqlx::Error> {
    // out = 我指向别人；in = 别人指向我
    sqlx::query_as(
        "SELECT from_id, to_id, kind, 'out' AS direction FROM todo_links WHERE from_id = $1 \
         UNION ALL \
         SELECT from_id, to_id, kind, 'in' AS direction FROM todo_links WHERE to_id = $1",
    )
    .bind(id)
    .fetch_all(pool)
    .await
}

/// 关联计数（todo_id → 关联条数，双向并集）。
pub async fn link_count_map(
    pool: &sqlx::PgPool,
) -> Result<std::collections::HashMap<Uuid, i64>, sqlx::Error> {
    let rows: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT t.id, SUM(t.n)::bigint AS n FROM (\
           SELECT from_id AS id, count(*)::bigint AS n FROM todo_links GROUP BY from_id \
           UNION ALL \
           SELECT to_id AS id, count(*)::bigint AS n FROM todo_links GROUP BY to_id\
         ) t JOIN todos ON todos.id = t.id GROUP BY t.id, t.n",
    )
    .fetch_all(pool)
    .await?;
    let mut m = std::collections::HashMap::new();
    for (id, n) in rows {
        *m.entry(id).or_insert(0) += n;
    }
    Ok(m)
}

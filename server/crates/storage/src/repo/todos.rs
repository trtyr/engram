//! 待办域仓储（0035；0074 工单拆表后回归轻量定位）：
//! todo = 微软式行动项（速记→做完勾掉，open/done/archived 两态半）。
//! 工单已拆独立 tickets 表（repo::tickets），本模块只管行动项。

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::PgPool;
use crate::error::StoreResult;

/// 列表/单查/导出统一行类型。
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TodoRow {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    pub status: String,
    pub priority: String,
    pub tags: Vec<String>,
    pub due_at: Option<DateTime<Utc>>,
    pub done_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// 全局单调短号（显示为 EN-<n>；sequence 生成）
    pub short_no: i32,
}

const COLS: &str =
    "id, title, body, status, priority, tags, due_at, done_at, created_at, updated_at, short_no";

pub struct NewTodo<'a> {
    pub id: Uuid,
    pub title: &'a str,
    pub body: &'a str,
    pub priority: &'a str,
    pub tags: &'a [String],
    pub due_at: Option<DateTime<Utc>>,
}

pub async fn insert(pool: &PgPool, t: &NewTodo<'_>) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO todos (id, title, body, status, priority, tags, due_at) \
         VALUES ($1, $2, $3, 'open', $4, $5, $6)",
    )
    .bind(t.id)
    .bind(t.title)
    .bind(t.body)
    .bind(t.priority)
    .bind(t.tags)
    .bind(t.due_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// 列表：open 优先，其余按 updated_at 降序；status/priority/tag/q 过滤。
/// cursor（D29 keyset 分页）：标准序用 (open 旗标, updated_at, id) 三元组行比较——
/// 排序键含 open 优先旗标，纯时间游标会在 open/done 分界处丢行。ORDER BY 带 id 决稳。
/// P019-M4：due 过滤页排序键统一——旧实现首页按 due_at 升序、翻页按
/// (open,updated_at,id) 降序且游标缺 due_at，跨页丢行/重行。
/// due 游标 = (due_at, id) 纯 ASC 同向行比较；标准游标 = (open 旗标, updated_at, id)。
#[derive(Clone, Copy, Debug)]
pub enum TodoCursor {
    Standard(i32, DateTime<Utc>, Uuid),
    Due(DateTime<Utc>, Uuid),
}

#[allow(clippy::too_many_arguments)]
pub async fn list(
    pool: &PgPool,
    status: Option<&str>,
    priority: Option<&str>,
    tag: Option<&str>,
    q: Option<&str>,
    due: Option<&str>,
    cursor: Option<TodoCursor>,
    limit: i64,
) -> StoreResult<Vec<TodoRow>> {
    // due 过滤（白名单，无注入面）：overdue=未完成且已过期；today=今天到期。
    // 非游标分支按 due_at 升序（最紧急在前）；游标分支保持原序（游标元组一致性优先）。
    let due_clause = |n: u8| {
        format!(
            "AND (${n}::text IS NULL OR (${n} = 'overdue' AND status = 'open' \
             AND due_at IS NOT NULL AND due_at < now()) \
             OR (${n} = 'today' AND status = 'open' AND due_at IS NOT NULL \
             AND due_at >= now() \
             AND due_at < date_trunc('day', now()) + interval '1 day'))"
        )
    };
    let order = if due.is_some() {
        // due 过滤只命中 open 且 due_at 非空行；纯 ASC 序（id 唯一决稳），
        // 游标行比较同向合法
        "ORDER BY due_at ASC, id ASC"
    } else {
        "ORDER BY (status = 'open') DESC, updated_at DESC, id DESC"
    };
    if let Some(cur) = cursor {
        // 游标谓词与排序键同构（P019-M4）
        let pred = match cur {
            TodoCursor::Due(..) => "AND (due_at, id) > ($7::timestamptz, $8::uuid)",
            TodoCursor::Standard(..) => {
                "AND (CASE WHEN status = 'open' THEN 1 ELSE 0 END, updated_at, id) < ($7::int, $8::timestamptz, $9::uuid)"
            }
        };
        let sql = format!(
            "SELECT {COLS} FROM todos \
             WHERE ($1::text IS NULL OR status = $1) \
             AND ($2::text IS NULL OR priority = $2) \
             AND ($3::text IS NULL OR tags @> ARRAY[$3::text]) \
             AND ($4::text IS NULL OR title ILIKE '%' || $4 || '%' OR body ILIKE '%' || $4 || '%') \
             {} \
             {pred} \
             {order} \
             LIMIT $5",
            due_clause(6),
            pred = pred,
            order = order
        );
        let bq = sqlx::query_as::<_, TodoRow>(sql.as_str());
        let bq = bq
            .bind(status)
            .bind(priority)
            .bind(tag)
            .bind(q)
            .bind(limit)
            .bind(due);
        let bq = match cur {
            TodoCursor::Standard(flag, ts, id) => bq.bind(flag).bind(ts).bind(id),
            TodoCursor::Due(due_at, id) => bq.bind(due_at).bind(id),
        };
        Ok(bq.fetch_all(pool).await?)
    } else {
        Ok(sqlx::query_as::<_, TodoRow>(
            format!(
                "SELECT {COLS} FROM todos \
                 WHERE ($1::text IS NULL OR status = $1) \
                 AND ($2::text IS NULL OR priority = $2) \
                 AND ($3::text IS NULL OR tags @> ARRAY[$3::text]) \
                 AND ($4::text IS NULL OR title ILIKE '%' || $4 || '%' OR body ILIKE '%' || $4 || '%') \
                 {} \
                 {order} \
                 LIMIT $5",
                due_clause(6),
                order = order
            )
            .as_str(),
        )
        .bind(status)
        .bind(priority)
        .bind(tag)
        .bind(q)
        .bind(limit)
        .bind(due)
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

/// 更新：COALESCE 部分更新（None 不动）；status **迁移**时同步完成时间戳——
/// done 盖 done_at（首次完成不刷新，D17）、回 open 清空。todos.status 引用更新前旧值。
pub struct TodoPatch<'a> {
    pub title: Option<&'a str>,
    pub body: Option<&'a str>,
    pub priority: Option<&'a str>,
    pub status: Option<&'a str>,
    pub due_at: Option<Option<DateTime<Utc>>>,
    pub tags: Option<&'a [String]>,
}

pub async fn update(pool: &PgPool, id: Uuid, p: &TodoPatch<'_>) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE todos SET \
            title = COALESCE($2, title), \
            body = COALESCE($3, body), \
            priority = COALESCE($4, priority), \
            status = COALESCE($5, status), \
            due_at = CASE WHEN $8 THEN NULL ELSE COALESCE($6, due_at) END, \
            tags = COALESCE($7::text[], tags), \
            done_at = CASE \
                WHEN $5 = 'done' AND todos.status IS DISTINCT FROM 'done' THEN now() \
                WHEN $5 = 'open' THEN NULL \
                ELSE done_at END, \
            updated_at = now() \
          WHERE id = $1",
    )
    .bind(id)
    .bind(p.title)
    .bind(p.body)
    .bind(p.priority)
    .bind(p.status)
    .bind(p.due_at)
    .bind(p.tags)
    .bind(matches!(p.due_at, Some(None)))
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

/// 全量导出（P4 数据主权）——与 TodoRow 同构。
pub async fn export_all(pool: &PgPool) -> StoreResult<Vec<TodoRow>> {
    let rows = sqlx::query_as::<_, TodoRow>(
        format!("SELECT {COLS} FROM todos ORDER BY (status = 'open') DESC, updated_at DESC")
            .as_str(),
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 统一检索（/search）的待办段：open 待办的标题/正文 ILIKE 匹配。
/// 返回 (id, title, body 前 200 字, priority)。工单段在 repo::tickets::search_open。
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

/// 按短号取 todo（EN-<n> 引用直达）。P019-M4：返 StoreResult——DB 故障报错而非吞成 404。
pub async fn find_by_short_no(pool: &sqlx::PgPool, short_no: i32) -> StoreResult<Option<TodoRow>> {
    Ok(
        sqlx::query_as::<_, TodoRow>("SELECT * FROM todos WHERE short_no = $1")
            .bind(short_no)
            .fetch_optional(pool)
            .await?,
    )
}

// ---------- 关联关系（blocked_by / relates_to / parent；行动项之间） ----------

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

/// 总数（同 list 过滤，不含分页）。
#[allow(clippy::too_many_arguments)]
pub async fn count(
    pool: &PgPool,
    status: Option<&str>,
    priority: Option<&str>,
    tag: Option<&str>,
    q: Option<&str>,
) -> StoreResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM todos \
         WHERE ($1::text IS NULL OR status = $1) \
         AND ($2::text IS NULL OR priority = $2) \
         AND ($3::text IS NULL OR tags @> ARRAY[$3::text]) \
         AND ($4::text IS NULL OR title ILIKE '%' || $4 || '%' OR body ILIKE '%' || $4 || '%')",
    )
    .bind(status)
    .bind(priority)
    .bind(tag)
    .bind(q)
    .fetch_one(pool)
    .await?)
}

//! 工单域仓储（0074 拆独立表）：项目绑定制——project_id NOT NULL FK，
//! 绑定不了的工单不存在于这张表。状态机/解决必填由表级 CHECK 强制。

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::PgPool;
use crate::error::StoreResult;

/// 列表/单查统一行类型。
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TicketRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub title: String,
    pub body: String,
    pub status: String,
    pub severity: Option<String>,
    pub symptom: String,
    pub reproduce: String,
    pub acceptance: String,
    pub resolution: String,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// 全局单调短号（显示为 EN-<n>；序列与 todos 衔接，编号跨表连续）
    pub short_no: i32,
}

const COLS: &str = "id, project_id, title, body, status, severity, symptom, reproduce, acceptance, resolution, resolved_at, created_at, updated_at, short_no";

pub struct NewTicket<'a> {
    pub id: Uuid,
    pub project_id: Uuid,
    pub title: &'a str,
    pub body: &'a str,
    pub severity: Option<&'a str>,
    pub symptom: &'a str,
    pub reproduce: &'a str,
    pub acceptance: &'a str,
}

pub async fn insert(pool: &PgPool, t: &NewTicket<'_>) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO tickets (id, project_id, title, body, status, severity, symptom, reproduce, acceptance) \
         VALUES ($1, $2, $3, $4, 'open', $5, $6, $7, $8)",
    )
    .bind(t.id)
    .bind(t.project_id)
    .bind(t.title)
    .bind(t.body)
    .bind(t.severity)
    .bind(t.symptom)
    .bind(t.reproduce)
    .bind(t.acceptance)
    .execute(pool)
    .await?;
    Ok(())
}

/// 列表：open 优先，其余 updated_at 降序；status/severity/project/q 过滤。
/// cursor（keyset）：(open 标记, updated_at, id) 三元组行比较，与 todos 同款。
#[allow(clippy::too_many_arguments)]
pub async fn list(
    pool: &PgPool,
    status: Option<&str>,
    severity: Option<&str>,
    project_id: Option<Uuid>,
    q: Option<&str>,
    cursor: Option<(i32, DateTime<Utc>, Uuid)>,
    limit: i64,
) -> StoreResult<Vec<TicketRow>> {
    if let Some((flag, ts, id)) = cursor {
        Ok(sqlx::query_as::<_, TicketRow>(
            format!(
                "SELECT {COLS} FROM tickets \
                 WHERE ($1::text IS NULL OR status = $1) \
                 AND ($2::text IS NULL OR severity = $2) \
                 AND ($3::uuid IS NULL OR project_id = $3) \
                 AND ($4::text IS NULL OR title ILIKE '%' || $4 || '%' OR body ILIKE '%' || $4 || '%' \
                      OR symptom ILIKE '%' || $4 || '%') \
                 AND (CASE WHEN status = 'open' THEN 1 ELSE 0 END, updated_at, id) < ($5::int, $6::timestamptz, $7::uuid) \
                 ORDER BY (status = 'open') DESC, updated_at DESC, id DESC \
                 LIMIT $8"
            )
            .as_str(),
        )
        .bind(status)
        .bind(severity)
        .bind(project_id)
        .bind(q)
        .bind(flag)
        .bind(ts)
        .bind(id)
        .bind(limit)
        .fetch_all(pool)
        .await?)
    } else {
        Ok(sqlx::query_as::<_, TicketRow>(
            format!(
                "SELECT {COLS} FROM tickets \
                 WHERE ($1::text IS NULL OR status = $1) \
                 AND ($2::text IS NULL OR severity = $2) \
                 AND ($3::uuid IS NULL OR project_id = $3) \
                 AND ($4::text IS NULL OR title ILIKE '%' || $4 || '%' OR body ILIKE '%' || $4 || '%' \
                      OR symptom ILIKE '%' || $4 || '%') \
                 ORDER BY (status = 'open') DESC, updated_at DESC, id DESC \
                 LIMIT $5"
            )
            .as_str(),
        )
        .bind(status)
        .bind(severity)
        .bind(project_id)
        .bind(q)
        .bind(limit)
        .fetch_all(pool)
        .await?)
    }
}

pub async fn get(pool: &PgPool, id: Uuid) -> StoreResult<Option<TicketRow>> {
    Ok(
        sqlx::query_as::<_, TicketRow>(
            format!("SELECT {COLS} FROM tickets WHERE id = $1").as_str(),
        )
        .bind(id)
        .fetch_optional(pool)
        .await?,
    )
}

/// 更新：COALESCE 部分更新（None 不动）；status 迁移时同步解决时间戳——
/// 进 resolved/verified 盖 resolved_at（首次不刷新）、回 open/confirmed/in_progress 清空。
pub struct TicketPatch<'a> {
    pub title: Option<&'a str>,
    pub body: Option<&'a str>,
    pub status: Option<&'a str>,
    pub severity: Option<Option<&'a str>>,
    pub symptom: Option<&'a str>,
    pub reproduce: Option<&'a str>,
    pub acceptance: Option<&'a str>,
    pub resolution: Option<&'a str>,
}

pub async fn update(pool: &PgPool, id: Uuid, p: &TicketPatch<'_>) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE tickets SET \
            title = COALESCE($2, title), \
            body = COALESCE($3, body), \
            status = COALESCE($4, status), \
            severity = CASE WHEN $9 THEN NULL ELSE COALESCE($5, severity) END, \
            symptom = COALESCE($6, symptom), \
            reproduce = COALESCE($7, reproduce), \
            acceptance = COALESCE($8, acceptance), \
            resolution = COALESCE($10, resolution), \
            resolved_at = CASE \
                WHEN $4 IN ('resolved','verified') AND resolved_at IS NULL THEN now() \
                WHEN $4 IN ('open','confirmed','in_progress') THEN NULL \
                ELSE resolved_at END, \
            updated_at = now() \
          WHERE id = $1",
    )
    .bind(id)
    .bind(p.title)
    .bind(p.body)
    .bind(p.status)
    .bind(p.severity)
    .bind(p.symptom)
    .bind(p.reproduce)
    .bind(p.acceptance)
    .bind(matches!(p.severity, Some(None)))
    .bind(p.resolution)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 删除（物理删除；归档语义走 status=archived）。
pub async fn delete(pool: &PgPool, id: Uuid) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM tickets WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// 总数（同 list 过滤，不含分页）。
#[allow(clippy::too_many_arguments)]
pub async fn count(
    pool: &PgPool,
    status: Option<&str>,
    severity: Option<&str>,
    project_id: Option<Uuid>,
    q: Option<&str>,
) -> StoreResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM tickets \
         WHERE ($1::text IS NULL OR status = $1) \
         AND ($2::text IS NULL OR severity = $2) \
         AND ($3::uuid IS NULL OR project_id = $3) \
         AND ($4::text IS NULL OR title ILIKE '%' || $4 || '%' OR body ILIKE '%' || $4 || '%' \
              OR symptom ILIKE '%' || $4 || '%')",
    )
    .bind(status)
    .bind(severity)
    .bind(project_id)
    .bind(q)
    .fetch_one(pool)
    .await?)
}

/// 统一检索（/search）的工单段：未关闭工单的标题/正文/症状 ILIKE 匹配。
/// 返回 (id, project_id, title, body 前 200 字, severity)。
pub async fn search_open(
    pool: &PgPool,
    q: &str,
    limit: i64,
) -> StoreResult<Vec<(Uuid, Uuid, String, String, Option<String>)>> {
    let pattern = format!("%{q}%");
    let rows = sqlx::query_as::<_, (Uuid, Uuid, String, String, Option<String>)>(
        "SELECT id, project_id, title, body, severity FROM tickets \
         WHERE status IN ('open','confirmed','in_progress') \
         AND (title ILIKE $1 OR body ILIKE $1 OR symptom ILIKE $1) \
         ORDER BY updated_at DESC LIMIT $2",
    )
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 按短号取工单（EN-<n> 引用直达）。P019-M4：返 StoreResult——DB 故障报错而非吞成 404。
pub async fn find_by_short_no(pool: &PgPool, short_no: i32) -> StoreResult<Option<TicketRow>> {
    Ok(
        sqlx::query_as::<_, TicketRow>("SELECT * FROM tickets WHERE short_no = $1")
            .bind(short_no)
            .fetch_optional(pool)
            .await?,
    )
}

// ---------- 工单活动时间线（ticket_events，0063 建表；0074 外键重挂 → tickets） ----------

#[derive(sqlx::FromRow, serde::Serialize, Debug)]
pub struct TicketEventRow {
    pub id: Uuid,
    pub ticket_id: Uuid,
    pub kind: String,
    pub payload: serde_json::Value,
    pub actor: String,
    pub created_at: DateTime<Utc>,
}

/// 追加时间线条目（kind=event|comment 由 CHECK 兜底）
pub async fn event_insert(
    pool: &PgPool,
    ticket_id: Uuid,
    kind: &str,
    payload: &serde_json::Value,
    actor: &str,
) -> StoreResult<Uuid> {
    let id = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO ticket_events (id, ticket_id, kind, payload, actor) VALUES ($1, $2, $3, $4, $5)")
        .bind(id)
        .bind(ticket_id)
        .bind(kind)
        .bind(payload)
        .bind(actor)
        .execute(pool)
        .await?;
    Ok(id)
}

/// 时间线（升序——时序正确性断言友好；前端倒序展示自行 reverse）
pub async fn events_for(pool: &PgPool, ticket_id: Uuid) -> StoreResult<Vec<TicketEventRow>> {
    Ok(sqlx::query_as::<_, TicketEventRow>(
        "SELECT id, ticket_id, kind, payload, actor, created_at \
         FROM ticket_events WHERE ticket_id = $1 ORDER BY created_at ASC, id ASC",
    )
    .bind(ticket_id)
    .fetch_all(pool)
    .await?)
}

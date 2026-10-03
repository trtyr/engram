//! logs 查询仓储（P005-T005）：过滤条件动态拼装，按 ts 倒序 + offset 分页。
//! src 层用 QueryBuilder（本 crate 是持久化收口层，直接 SQL 合法）。

use sqlx::PgPool;

use crate::error::StoreResult;

/// 单行日志。
#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct LogRow {
    pub id: i64,
    pub ts: chrono::DateTime<chrono::Utc>,
    pub level: String,
    pub target: String,
    pub message: String,
    pub fields: serde_json::Value,
    pub request_id: Option<String>,
}

/// 日志过滤条件（全部可选；全部命中才返回）。
#[derive(Debug, Default)]
pub struct LogFilter<'a> {
    pub level: Option<&'a str>,
    /// message/target ILIKE 模糊
    pub q: Option<&'a str>,
    pub request_id: Option<&'a str>,
    /// 按任务筛（fields->>'job_id'）——P010：任务生命周期在同一条日志时间线上
    pub job_id: Option<&'a str>,
    /// 范围（P010）：Some(true)=仅后台（带 job_id）/ Some(false)=仅系统（不带 job_id）/ None=全部
    pub job_scope: Option<bool>,
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub until: Option<chrono::DateTime<chrono::Utc>>,
    /// 仅审计行（fields->>'audit' = 'true'）
    pub audit_only: bool,
    pub limit: i64,
    pub offset: i64,
}

/// 过滤条件推入 QueryBuilder——`query_logs` 与 `count_logs_filtered` **共用同一套 WHERE**。
/// 必须共用：否则「共 N 条」与列表内容会漂移（P010 修正：前端曾把「已拉取条数」冒充总数）。
fn push_filters<'args>(
    qb: &mut sqlx::QueryBuilder<'args, sqlx::Postgres>,
    f: &'args LogFilter<'args>,
) {
    if let Some(level) = f.level {
        qb.push(" AND upper(level) = upper(").push_bind(level).push(")");
    }
    if let Some(q) = f.q {
        qb.push(" AND (message ILIKE ")
            .push_bind(format!("%{q}%"))
            .push(" OR target ILIKE ")
            .push_bind(format!("%{q}%"))
            .push(")");
    }
    if let Some(rid) = f.request_id {
        qb.push(" AND request_id = ").push_bind(rid);
    }
    if let Some(jid) = f.job_id {
        qb.push(" AND fields->>'job_id' = ").push_bind(jid);
    }
    match f.job_scope {
        Some(true) => {
            qb.push(" AND fields ? 'job_id'");
        }
        Some(false) => {
            qb.push(" AND NOT (fields ? 'job_id')");
        }
        None => {}
    }
    if let Some(since) = f.since {
        qb.push(" AND ts >= ").push_bind(since);
    }
    if let Some(until) = f.until {
        qb.push(" AND ts <= ").push_bind(until);
    }
    if f.audit_only {
        qb.push(" AND fields->>'audit' = 'true'");
    }
}

/// 查询（ts DESC + id DESC 稳定排序）。
pub async fn query_logs(pool: &PgPool, f: &LogFilter<'_>) -> StoreResult<Vec<LogRow>> {
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT id, ts, level, target, message, fields, request_id FROM logs WHERE 1=1",
    );
    push_filters(&mut qb, f);
    qb.push(" ORDER BY ts DESC, id DESC LIMIT ")
        .push_bind(f.limit.clamp(1, 500))
        .push(" OFFSET ")
        .push_bind(f.offset.max(0));
    let rows = qb.build_query_as::<LogRow>().fetch_all(pool).await?;
    Ok(rows)
}

/// 同过滤条件下的**真实总数**（忽略 limit/offset）——前端真分页的依据。
pub async fn count_logs_filtered(pool: &PgPool, f: &LogFilter<'_>) -> StoreResult<i64> {
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT count(*) FROM logs WHERE 1=1");
    push_filters(&mut qb, f);
    let n: i64 = qb.build_query_scalar().fetch_one(pool).await?;
    Ok(n)
}

/// 单桶计数（group_by 维度值 + 行数）。
#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct LogBucket {
    pub bucket: String,
    pub count: i64,
}

/// 聚合计数（P010）：按 level 或 target 分组，时间窗内计数降序。
/// `group_by` 只接受 "level" / "target"（调用方校验，此处兜底为 level）。
pub async fn count_logs(
    pool: &PgPool,
    since: chrono::DateTime<chrono::Utc>,
    until: chrono::DateTime<chrono::Utc>,
    group_by: &str,
) -> StoreResult<Vec<LogBucket>> {
    let col = if group_by == "target" { "target" } else { "level" };
    let sql = format!(
        "SELECT {col} AS bucket, count(*) AS count
         FROM logs WHERE ts >= $1 AND ts <= $2
         GROUP BY {col} ORDER BY count DESC LIMIT 50"
    );
    let rows = sqlx::query_as::<_, LogBucket>(&sql)
        .bind(since)
        .bind(until)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

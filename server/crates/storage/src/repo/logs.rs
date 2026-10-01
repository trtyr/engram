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
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub until: Option<chrono::DateTime<chrono::Utc>>,
    /// 仅审计行（fields->>'audit' = 'true'）
    pub audit_only: bool,
    pub limit: i64,
    pub offset: i64,
}

/// 查询（ts DESC + id DESC 稳定排序）。
pub async fn query_logs(pool: &PgPool, f: &LogFilter<'_>) -> StoreResult<Vec<LogRow>> {
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT id, ts, level, target, message, fields, request_id FROM logs WHERE 1=1",
    );
    if let Some(level) = f.level {
        qb.push(" AND level = ").push_bind(level);
    }
    if let Some(q) = f.q {
        qb.push(" AND (message ILIKE ").push_bind(format!("%{q}%"))
            .push(" OR target ILIKE ").push_bind(format!("%{q}%")).push(")");
    }
    if let Some(rid) = f.request_id {
        qb.push(" AND request_id = ").push_bind(rid);
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
    qb.push(" ORDER BY ts DESC, id DESC LIMIT ")
        .push_bind(f.limit.clamp(1, 500))
        .push(" OFFSET ")
        .push_bind(f.offset.max(0));
    let rows = qb.build_query_as::<LogRow>().fetch_all(pool).await?;
    Ok(rows)
}

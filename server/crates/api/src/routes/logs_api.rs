//! 日志查询端点（P005-T005，admin 专属）——logs 表的读取面：
//! level/q/时间范围/request_id/audit_only 过滤 + limit/offset 分页。

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use utoipa::IntoParams;

use crate::auth::Principal;
use crate::error::ApiError;
use crate::state::AppState;

fn require_admin(p: &Principal) -> Result<(), ApiError> {
    match p {
        Principal::Admin => Ok(()),
        _ => Err(ApiError::Forbidden("仅管理员可查日志".into())),
    }
}

#[derive(Deserialize, IntoParams)]
pub struct LogsQuery {
    pub level: Option<String>,
    /// message/target ILIKE 模糊
    pub q: Option<String>,
    pub request_id: Option<String>,
    /// RFC3339
    pub since: Option<String>,
    pub until: Option<String>,
    /// "true" = 仅审计行
    pub audit: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// 查询日志（admin；ts 倒序分页）。
#[utoipa::path(get, path = "/logs", params(LogsQuery), responses((status = 200)))]
pub async fn list_logs(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<LogsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&principal)?;
    let parse_time = (|s: &Option<String>| {
        s.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                chrono::DateTime::parse_from_rfc3339(s)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .map_err(|_| ApiError::BadRequest("时间需 RFC3339（如 2026-10-01T00:00:00Z）".into()))
            })
            .transpose()
    })(&p.since)?;
    let until = (|s: &Option<String>| {
        s.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                chrono::DateTime::parse_from_rfc3339(s)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .map_err(|_| ApiError::BadRequest("时间需 RFC3339".into()))
            })
            .transpose()
    })(&p.until)?;

    let filter = engram_storage::repo::logs::LogFilter {
        level: p.level.as_deref(),
        q: p.q.as_deref(),
        request_id: p.request_id.as_deref(),
        since: parse_time,
        until,
        audit_only: p.audit.as_deref() == Some("true"),
        limit: p.limit.unwrap_or(100),
        offset: p.offset.unwrap_or(0),
    };
    let rows = engram_storage::repo::logs::query_logs(&state.pool, &filter)
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    Ok(Json(serde_json::json!({ "logs": rows })))
}

pub fn logs_routes() -> axum::Router<AppState> {
    use axum::routing::get;
    axum::Router::new().route("/logs", get(list_logs))
}

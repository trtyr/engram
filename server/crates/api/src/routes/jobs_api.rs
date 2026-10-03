//! 后台执行查询端点（**内部调度面**，P010 起定名）。
//!
//! 用户可见的概念只有「日志」——系统里发生的一切（含任务生命周期）都在 logs 时间线上。
//! 本组端点仅供**调度器/运维**用：查看队列积压、重跑失败条目。
//! 面向人的查询请用 GET /logs（支持 ?job_id= 筛出某次执行的完整过程）。
//! 保留理由：状态机（pending→running→终态）+ 重试 + 并发锁需要独立索引，是日志的投影而非并列概念。
//!
//! 权限：管理员与 API key 均可读——AI 客户端轮询执行状态用。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use uuid::Uuid;

use engram_jobs::JobQueue;
use engram_jobs::types::{Job, JobEvent, JobStatus};
use utoipa::IntoParams;

use crate::auth::Principal;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Deserialize, IntoParams)]
pub struct ListJobsParams {
    /// 执行种类过滤（逗号分隔）
    pub kind: Option<String>,
    /// 状态过滤（逗号分隔: pending,running,succeeded,failed,dead,cancelled；非法值报 400）
    pub status: Option<String>,
    /// 游标（上一页最后一条的 created_at）
    pub cursor: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<i64>,
}

/// 唯一解析收口在 `JobStatus::parse_filter`（六态含 cancelled；非法值报错不静默退化，RJ-02）。
fn parse_statuses(s: &Option<String>) -> Result<Vec<JobStatus>, ApiError> {
    JobStatus::parse_filter(s.as_deref()).map_err(ApiError::BadRequest)
}

/// 后台执行列表。
#[utoipa::path(get, path = "/jobs", params(ListJobsParams),
    responses((status = 200, body = [Job])))]
pub async fn list_jobs(
    State(state): State<AppState>,
    Query(params): Query<ListJobsParams>,
) -> Result<Json<Vec<Job>>, ApiError> {
    // 无域 scope 要求：任何合法凭证可读后台执行（AI 轮询自己触发的）
    let queue = JobQueue::new(state.pool);
    let kinds: Vec<String> = params
        .kind
        .as_deref()
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    let jobs = queue
        .list(
            &kinds,
            &parse_statuses(&params.status)?,
            params.cursor,
            params.limit.unwrap_or(50).min(200),
        )
        .await?;
    Ok(Json(jobs))
}

/// 后台执行详情。
#[utoipa::path(get, path = "/jobs/{id}",
    responses((status = 200, body = Job), (status = 404, body = crate::error::ErrorEnvelope)))]
pub async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Job>, ApiError> {
    let queue = JobQueue::new(state.pool);
    queue
        .get(id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("执行 {id} 不存在")))
}

#[derive(Deserialize, IntoParams)]
pub struct EventsParams {
    /// 增量游标（上一批最后事件 id）
    pub after: Option<i64>,
    pub limit: Option<i64>,
}

/// 后台执行的过程行（轮询）。
#[utoipa::path(get, path = "/jobs/{id}/events", params(EventsParams),
    responses((status = 200, body = [JobEvent])))]
pub async fn get_job_events(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(params): Query<EventsParams>,
) -> Result<Json<Vec<JobEvent>>, ApiError> {
    let queue = JobQueue::new(state.pool);
    Ok(Json(
        queue
            .events(id, params.after, params.limit.unwrap_or(100).min(1000))
            .await?,
    ))
}

/// 复活 dead/failed 的后台执行重跑（仅管理员）。
#[utoipa::path(post, path = "/jobs/{id}/revive",
    responses((status = 204), (status = 403, body = crate::error::ErrorEnvelope)))]
pub async fn revive_job(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    match principal.0 {
        Principal::Admin => {}
        Principal::ApiKey { .. } => return Err(ApiError::Forbidden("后台执行复活仅管理员".into())),
    }
    let queue = JobQueue::new(state.pool);
    queue.revive(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod parse_statuses_tests {
    use super::*;

    #[test]
    fn cancelled_filter_parses() {
        let got = parse_statuses(&Some("cancelled".into())).unwrap();
        assert_eq!(got, vec![JobStatus::Cancelled]);
    }

    #[test]
    fn illegal_value_maps_to_400() {
        let err = parse_statuses(&Some("nope".into())).unwrap_err();
        assert!(
            matches!(err, ApiError::BadRequest(_)),
            "非法值必须 400，实际：{err:?}"
        );
    }
}

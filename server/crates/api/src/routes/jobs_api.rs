//! 任务查询端点（管理员与 API key 均可读——AI 客户端轮询任务状态用）。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use uuid::Uuid;

use engram_jobs::JobQueue;
use engram_jobs::types::{Job, JobEvent, JobStatus};
use utoipa::IntoParams;

use crate::auth::{Principal, require_scope};
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Deserialize, IntoParams)]
pub struct ListJobsParams {
    /// 任务种类过滤（逗号分隔）
    pub kind: Option<String>,
    /// 状态过滤（逗号分隔: pending,running,succeeded,failed,dead）
    pub status: Option<String>,
    /// 游标（上一页最后一条的 created_at）
    pub cursor: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<i64>,
}

fn parse_statuses(s: &Option<String>) -> Vec<JobStatus> {
    s.as_deref()
        .map(|v| {
            v.split(',')
                .filter_map(|p| match p.trim() {
                    "pending" => Some(JobStatus::Pending),
                    "running" => Some(JobStatus::Running),
                    "succeeded" => Some(JobStatus::Succeeded),
                    "failed" => Some(JobStatus::Failed),
                    "dead" => Some(JobStatus::Dead),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 任务列表。
#[utoipa::path(get, path = "/jobs", params(ListJobsParams),
    responses((status = 200, body = [Job])))]
pub async fn list_jobs(
    State(state): State<AppState>,
    Query(params): Query<ListJobsParams>,
) -> Result<Json<Vec<Job>>, ApiError> {
    // 无域 scope 要求：任何合法凭证可读任务（AI 轮询自己触发的任务）
    let _ = state;
    let queue = JobQueue::new(state.pool);
    let kinds: Vec<String> = params
        .kind
        .as_deref()
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    let jobs = queue
        .list(
            &kinds,
            &parse_statuses(&params.status),
            params.cursor,
            params.limit.unwrap_or(50).min(200),
        )
        .await?;
    Ok(Json(jobs))
}

/// 任务详情。
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
        .ok_or_else(|| ApiError::NotFound(format!("任务 {id} 不存在")))
}

#[derive(Deserialize, IntoParams)]
pub struct EventsParams {
    /// 增量游标（上一批最后事件 id）
    pub after: Option<i64>,
    pub limit: Option<i64>,
}

/// 任务事件时间线（轮询；SSE 在 Phase 6 按需加）。
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

/// 复活 dead/failed 任务重跑（仅管理员）。
#[utoipa::path(post, path = "/jobs/{id}/revive",
    responses((status = 204), (status = 403, body = crate::error::ErrorEnvelope)))]
pub async fn revive_job(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    match principal.0 {
        Principal::Admin => {}
        Principal::ApiKey { .. } => return Err(ApiError::Forbidden("任务复活仅管理员".into())),
    }
    let queue = JobQueue::new(state.pool);
    queue.revive(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// require_scope 引用（未来按域拆 jobs 权限时启用）
#[allow(unused)]
fn _scope_helper(p: &Principal, scope: &str) -> Result<(), ApiError> {
    require_scope(p, scope)
}

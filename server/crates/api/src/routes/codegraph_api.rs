//! CodeGraph 域端点（codegraph scope）。
//!
//! 索引/同步走平台 job 队列（返回 202 + job_id，异步执行）——不再阻塞 HTTP 10 分钟。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use engram_core::codegraph::{CgBridge, CgError, CgProjectDto, CliStatus, QueryKind};
use engram_jobs::{JobQueue, JobTemplate};
use serde::Deserialize;
use serde_json::json;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::auth::{Principal, require_scope};
use crate::error::ApiError;
use crate::state::AppState;

fn require_cg(p: &Principal) -> Result<(), ApiError> {
    require_scope(p, "codegraph")
}

fn ce(e: CgError) -> ApiError {
    match e {
        CgError::NotFound(m) => ApiError::NotFound(m),
        CgError::BadRequest(m) => ApiError::BadRequest(m),
        CgError::VersionMismatch { need, got } => {
            ApiError::BadRequest(format!("CodeGraph 版本不匹配：需要 {need}，实际 {got}"))
        }
        CgError::Timeout(s, cmd) => ApiError::Unavailable(format!("CodeGraph {cmd} 超时（{s}s）")),
        CgError::Failed(code, msg) => {
            ApiError::Unavailable(format!("CodeGraph 失败（exit {code}）: {msg}"))
        }
        CgError::Parse(m) => ApiError::Unavailable(m),
        CgError::CliUnavailable(m) => ApiError::Unavailable(m),
        CgError::Storage(m) => ApiError::Unavailable(m),
    }
}

fn bridge(state: &AppState) -> CgBridge {
    CgBridge::new(state.pool.clone(), state.data_dir.join("codegraph"))
}

/// 入队索引/同步 job，返回 job_id。执行进度看 jobs 事件流与项目状态。
async fn enqueue(
    state: &AppState,
    kind: &str,
    id: Uuid,
) -> Result<engram_jobs::types::Job, ApiError> {
    JobQueue::new(state.pool.clone())
        .enqueue(JobTemplate::new(kind).with_payload(json!({ "project_id": id })))
        .await
        .map_err(|e| ApiError::Unavailable(format!("job 入队失败: {e}")))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct RegisterProjectRequest {
    pub name: String,
    /// 本地绝对路径或 git URL
    pub source_uri: String,
}

/// 注册项目（本地路径或 git URL；同源只许注册一次）。
#[utoipa::path(post, path = "/codegraph/projects",
    request_body = RegisterProjectRequest,
    responses((status = 201, body = CgProjectDto)))]
pub async fn register_project(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<RegisterProjectRequest>,
) -> Result<(StatusCode, Json<CgProjectDto>), ApiError> {
    require_cg(&principal)?;
    let p = bridge(&state)
        .register(&req.name, &req.source_uri)
        .await
        .map_err(ce)?;
    Ok((StatusCode::CREATED, Json(p)))
}

#[utoipa::path(get, path = "/codegraph/projects", responses((status = 200, body = [CgProjectDto])))]
pub async fn list_projects(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<CgProjectDto>>, ApiError> {
    require_cg(&principal)?;
    Ok(Json(bridge(&state).list().await.map_err(ce)?))
}

#[utoipa::path(get, path = "/codegraph/projects/{id}", responses((status = 200, body = CgProjectDto)))]
pub async fn get_project(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<CgProjectDto>, ApiError> {
    require_cg(&principal)?;
    Ok(Json(bridge(&state).get(id).await.map_err(ce)?))
}

/// 删除项目（git clone 的工作目录一并清理；本地路径项目不动源码）。
#[utoipa::path(delete, path = "/codegraph/projects/{id}", responses((status = 200, body = Object)))]
pub async fn delete_project(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_cg(&principal)?;
    let workdir_removed = bridge(&state).delete(id).await.map_err(ce)?;
    Ok(Json(
        json!({ "deleted": id, "workdir_removed": workdir_removed }),
    ))
}

/// 建索引/重建索引：入队 cg_index job 异步执行（202 + job_id；进度看 jobs 与项目状态）。
#[utoipa::path(post, path = "/codegraph/projects/{id}/index", responses((status = 202, body = Object)))]
pub async fn index_project(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    require_cg(&principal)?;
    // 拒绝对不存在项目的入队（404 语义）
    bridge(&state).get(id).await.map_err(ce)?;
    let job = enqueue(&state, "cg_index", id).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "job_id": job.id, "status": "queued", "kind": "cg_index" })),
    ))
}

/// 增量同步：入队 cg_sync job 异步执行。
#[utoipa::path(post, path = "/codegraph/projects/{id}/sync", responses((status = 202, body = Object)))]
pub async fn sync_project(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    require_cg(&principal)?;
    bridge(&state).get(id).await.map_err(ce)?;
    let job = enqueue(&state, "cg_sync", id).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "job_id": job.id, "status": "queued", "kind": "cg_sync" })),
    ))
}

/// CLI 可用性（前端状态条：装没装、版本、pin）。
#[utoipa::path(get, path = "/codegraph/status", responses((status = 200, body = CliStatus)))]
pub async fn status(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<CliStatus>, ApiError> {
    require_cg(&principal)?;
    Ok(Json(bridge(&state).cli_status().await))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CgQueryRequest {
    /// explore | search | node | callers | callees | impact
    pub kind: String,
    /// 查询文本或符号名
    pub target: String,
    /// explore→max-files；impact→depth
    pub depth: Option<u32>,
}

/// 代理查询（explore/node 返回 Markdown 文本，其余归一 JSON）。
#[utoipa::path(post, path = "/codegraph/projects/{id}/query",
    request_body = CgQueryRequest,
    responses((status = 200, body = Object)))]
pub async fn query(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<CgQueryRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_cg(&principal)?;
    let kind = QueryKind::from_str_opt(&req.kind)
        .ok_or_else(|| ApiError::BadRequest(format!("未知查询类型: {}", req.kind)))?;
    if req.target.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "target 不能为空——先用 kind=search 搜符号，再对具体符号做 callers/impact".into(),
        ));
    }
    let v = bridge(&state)
        .query(id, kind, &req.target, req.depth)
        .await
        .map_err(ce)?;
    Ok(Json(v))
}

#[derive(Deserialize, IntoParams, utoipa::ToSchema)]
pub struct GraphParams {
    /// 中心符号名（省略 = 返回文件级全图——全部跨文件依赖按文件聚合）
    pub symbol: Option<String>,
}

/// 调用图：带 symbol = 以该符号为中心的 callers/callees 子图；
/// 不带 symbol = 文件级全图（全部跨文件依赖按文件聚合，看项目全貌）。
#[utoipa::path(get, path = "/codegraph/projects/{id}/graph", params(("id" = Uuid, Path), GraphParams),
    responses((status = 200, body = Object)))]
pub async fn graph(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(p): Query<GraphParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_cg(&principal)?;
    let v = match p.symbol.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(symbol) => bridge(&state).graph(id, symbol).await.map_err(ce)?,
        None => bridge(&state).full_graph(id).await.map_err(ce)?,
    };
    Ok(Json(v))
}

//! CodeGraph 域端点（codegraph scope）。

use engram_core::codegraph::{CgBridge, CgError, CgProjectDto, QueryKind};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
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

#[derive(Deserialize, utoipa::ToSchema)]
pub struct RegisterProjectRequest {
    pub name: String,
    /// 本地绝对路径或 git URL
    pub source_uri: String,
}

/// 注册项目（本地路径或 git URL）。
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

#[utoipa::path(post, path = "/codegraph/projects/{id}/index", responses((status = 202, body = CgProjectDto)))]
pub async fn index_project(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<CgProjectDto>), ApiError> {
    require_cg(&principal)?;
    let p = bridge(&state).index(id).await.map_err(ce)?;
    Ok((StatusCode::ACCEPTED, Json(p)))
}

#[utoipa::path(post, path = "/codegraph/projects/{id}/sync", responses((status = 200, body = CgProjectDto)))]
pub async fn sync_project(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<CgProjectDto>, ApiError> {
    require_cg(&principal)?;
    Ok(Json(bridge(&state).sync(id).await.map_err(ce)?))
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
    let v = bridge(&state)
        .query(id, kind, &req.target, req.depth)
        .await
        .map_err(ce)?;
    Ok(Json(v))
}

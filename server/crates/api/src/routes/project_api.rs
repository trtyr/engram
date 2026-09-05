//! 项目记忆域端点（project scope）。
//!
//! 设计：docs/plantree/plans/project-memory/（0005 三表模型，双入口平等）。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use engram_core::project::{
    ProjectDetailDto, ProjectDocDto, ProjectDto, ProjectError, ProjectLocationDto, ProjectService,
    ProjectTypeDto,
};
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::auth::{Principal, require_scope};
use crate::error::ApiError;
use crate::state::AppState;

fn require_project(p: &Principal) -> Result<(), ApiError> {
    require_scope(p, "project")
}

fn pe(e: ProjectError) -> ApiError {
    match e {
        ProjectError::NotFound(m) => ApiError::NotFound(m),
        ProjectError::Conflict(m) => ApiError::Conflict(m),
        ProjectError::BadRequest(m) => ApiError::BadRequest(m),
        ProjectError::Storage(m) => ApiError::Unavailable(m),
    }
}

fn svc(state: &AppState) -> ProjectService {
    ProjectService::new(state.pool.clone())
}

/// 归属校验：路径里的 project_id 必须与资源实际所属一致（跨项目寻址一律 404，
/// 不泄露「别的项目下存在这个 id」）。
async fn owned_location(
    svc: &ProjectService,
    project_id: Uuid,
    loc_id: Uuid,
) -> Result<ProjectLocationDto, ApiError> {
    let loc = svc.get_location(loc_id).await.map_err(pe)?;
    if loc.project_id != project_id {
        return Err(ApiError::NotFound(
            "位置不存在或不属于该项目——先 project-get 看该项目 locations 列表取 id".into(),
        ));
    }
    Ok(loc)
}

async fn owned_doc(
    svc: &ProjectService,
    project_id: Uuid,
    doc_id: Uuid,
) -> Result<ProjectDocDto, ApiError> {
    let doc = svc.get_doc(doc_id).await.map_err(pe)?;
    if doc.project_id != project_id {
        return Err(ApiError::NotFound(
            "文档不存在或不属于该项目——先 project-get 看该项目 docs 列表取 id".into(),
        ));
    }
    Ok(doc)
}

// ---------- 请求体 ----------

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CreateProjectRequest {
    pub name: String,
    /// dev / research
    pub r#type: String,
    pub description: Option<String>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateProjectRequest {
    pub name: String,
    /// active / paused / done / abandoned
    pub status: String,
    pub description: Option<String>,
    pub categories: Vec<String>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct BatchDeleteRequest {
    pub ids: Vec<Uuid>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct LocationRequest {
    pub ip: String,
    pub host: String,
    pub os: String,
    pub path: String,
    pub purpose: Option<String>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct DocRequest {
    pub category: String,
    pub title: String,
    pub content: String,
}

#[derive(Deserialize, IntoParams)]
pub struct ListProjectsParams {
    #[serde(rename = "type")]
    pub type_: Option<String>,
}

// ---------- 项目 ----------

/// 类型模板（Web 建项目时选择类型）。
#[utoipa::path(get, path = "/projects/types", responses((status = 200, body = [ProjectTypeDto])))]
pub async fn list_types(
    principal: axum::Extension<Principal>,
) -> Result<Json<Vec<ProjectTypeDto>>, ApiError> {
    require_project(&principal)?;
    Ok(Json(ProjectService::list_types()))
}

/// 新建项目（type 决定初始分类，categories 从类型模板复制）。
#[utoipa::path(post, path = "/projects",
    request_body = CreateProjectRequest,
    responses((status = 201, body = ProjectDto)))]
pub async fn create_project(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<ProjectDto>), ApiError> {
    require_project(&principal)?;
    let p = svc(&state)
        .create_project(&req.name, &req.r#type, req.description.as_deref())
        .await
        .map_err(pe)?;
    Ok((StatusCode::CREATED, Json(p)))
}

/// 项目列表（可选 ?type=dev 筛选）。
#[utoipa::path(get, path = "/projects", params(ListProjectsParams),
    operation_id = "projects_list",
    responses((status = 200, body = [ProjectDto])))]
pub async fn list_projects(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ListProjectsParams>,
) -> Result<Json<Vec<ProjectDto>>, ApiError> {
    require_project(&principal)?;
    Ok(Json(
        svc(&state)
            .list_projects(p.type_.as_deref())
            .await
            .map_err(pe)?,
    ))
}

/// 项目详情（本体 + 位置 + 文档）。
#[utoipa::path(get, path = "/projects/{id}",
    operation_id = "projects_get",
    responses((status = 200, body = ProjectDetailDto)))]
pub async fn get_project(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ProjectDetailDto>, ApiError> {
    require_project(&principal)?;
    Ok(Json(svc(&state).get_project(id).await.map_err(pe)?))
}

/// 编辑项目（改名/状态/描述/分类列表）。
#[utoipa::path(put, path = "/projects/{id}",
    request_body = UpdateProjectRequest,
    responses((status = 200, body = ProjectDto)))]
pub async fn update_project(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProjectRequest>,
) -> Result<Json<ProjectDto>, ApiError> {
    require_project(&principal)?;
    Ok(Json(
        svc(&state)
            .update_project(
                id,
                &req.name,
                &req.status,
                req.description.as_deref(),
                &req.categories,
            )
            .await
            .map_err(pe)?,
    ))
}

/// 删除项目（级联删位置与文档）。
#[utoipa::path(delete, path = "/projects/{id}",
    responses((status = 204, description = "已删除")))]
pub async fn delete_project(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_project(&principal)?;
    svc(&state).delete_project(id).await.map_err(pe)?;
    Ok(StatusCode::NO_CONTENT)
}

/// 批量删除（列表多选）。
#[utoipa::path(post, path = "/projects/batch-delete",
    request_body = BatchDeleteRequest,
    responses((status = 200, body = BatchDeleteResult)))]
pub async fn batch_delete_projects(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<BatchDeleteRequest>,
) -> Result<Json<BatchDeleteResult>, ApiError> {
    require_project(&principal)?;
    let (deleted, failed) = svc(&state)
        .batch_delete_projects(&req.ids)
        .await
        .map_err(pe)?;
    Ok(Json(BatchDeleteResult { deleted, failed }))
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct BatchDeleteResult {
    pub deleted: usize,
    pub failed: Vec<Uuid>,
}

// ---------- 位置（多主机） ----------

/// 登记项目位置（多主机）。
#[utoipa::path(post, path = "/projects/{id}/locations",
    request_body = LocationRequest,
    responses((status = 201, body = ProjectLocationDto)))]
pub async fn add_location(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<LocationRequest>,
) -> Result<(StatusCode, Json<ProjectLocationDto>), ApiError> {
    require_project(&principal)?;
    let loc = svc(&state)
        .add_location(
            id,
            &req.ip,
            &req.host,
            &req.os,
            &req.path,
            req.purpose.as_deref(),
        )
        .await
        .map_err(pe)?;
    Ok((StatusCode::CREATED, Json(loc)))
}

/// 读单个位置（详情页/CLI 部分更新前取原值用）。
#[utoipa::path(get, path = "/projects/{id}/locations/{loc_id}",
    responses((status = 200, body = ProjectLocationDto)))]
pub async fn get_location(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path((id, loc_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<ProjectLocationDto>, ApiError> {
    require_project(&principal)?;
    let s = svc(&state);
    owned_location(&s, id, loc_id).await?;
    Ok(Json(s.get_location(loc_id).await.map_err(pe)?))
}

/// 编辑位置。
#[utoipa::path(put, path = "/projects/{id}/locations/{loc_id}",
    request_body = LocationRequest,
    responses((status = 200, body = ProjectLocationDto)))]
pub async fn update_location(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path((id, loc_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<LocationRequest>,
) -> Result<Json<ProjectLocationDto>, ApiError> {
    require_project(&principal)?;
    let s = svc(&state);
    owned_location(&s, id, loc_id).await?;
    Ok(Json(
        s.update_location(
            loc_id,
            &req.ip,
            &req.host,
            &req.os,
            &req.path,
            req.purpose.as_deref(),
        )
        .await
        .map_err(pe)?,
    ))
}

/// 删除位置。
#[utoipa::path(delete, path = "/projects/{id}/locations/{loc_id}",
    responses((status = 204, description = "已删除")))]
pub async fn delete_location(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path((id, loc_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    require_project(&principal)?;
    let s = svc(&state);
    owned_location(&s, id, loc_id).await?;
    s.delete_location(loc_id).await.map_err(pe)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- 分类文档 ----------

/// 项目下新增文档（分类 + markdown）。
#[utoipa::path(post, path = "/projects/{id}/docs",
    request_body = DocRequest,
    responses((status = 201, body = ProjectDocDto)))]
pub async fn add_doc(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<DocRequest>,
) -> Result<(StatusCode, Json<ProjectDocDto>), ApiError> {
    require_project(&principal)?;
    let doc = svc(&state)
        .add_doc(id, &req.category, &req.title, &req.content)
        .await
        .map_err(pe)?;
    Ok((StatusCode::CREATED, Json(doc)))
}

/// 读单个文档（详情页右侧编辑用）。
#[utoipa::path(get, path = "/projects/{id}/docs/{doc_id}",
    responses((status = 200, body = ProjectDocDto)))]
pub async fn get_doc(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path((id, doc_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<ProjectDocDto>, ApiError> {
    require_project(&principal)?;
    let s = svc(&state);
    owned_doc(&s, id, doc_id).await?;
    Ok(Json(s.get_doc(doc_id).await.map_err(pe)?))
}

/// 编辑文档。
#[utoipa::path(put, path = "/projects/{id}/docs/{doc_id}",
    request_body = DocRequest,
    responses((status = 200, body = ProjectDocDto)))]
pub async fn update_doc(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path((id, doc_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<DocRequest>,
) -> Result<Json<ProjectDocDto>, ApiError> {
    require_project(&principal)?;
    let s = svc(&state);
    owned_doc(&s, id, doc_id).await?;
    Ok(Json(
        s.update_doc(doc_id, &req.category, &req.title, &req.content)
            .await
            .map_err(pe)?,
    ))
}

/// 删除文档。
#[utoipa::path(delete, path = "/projects/{id}/docs/{doc_id}",
    responses((status = 204, description = "已删除")))]
pub async fn delete_doc(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path((id, doc_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    require_project(&principal)?;
    let s = svc(&state);
    owned_doc(&s, id, doc_id).await?;
    s.delete_doc(doc_id).await.map_err(pe)?;
    Ok(StatusCode::NO_CONTENT)
}

//! 资产台账域端点（assets scope）。
//!
//! 2026-09-21 新增的资产域（见《项目与资产模型 · README》§2）：资产 = 我拥有的、可以被操作的
//! 东西——身份唯一、无「收尾」、被项目**引用**（`project_locations.asset_id`）。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use engram_core::assets::{AssetDetailDto, AssetDto, AssetError, AssetKindDto, AssetService};
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::auth::{Principal, require_scope};
use crate::error::ApiError;
use crate::state::AppState;

fn require_assets(p: &Principal) -> Result<(), ApiError> {
    require_scope(p, "assets")
}

fn ae(e: AssetError) -> ApiError {
    match e {
        AssetError::NotFound(m) => ApiError::NotFound(m),
        AssetError::Conflict(m) => ApiError::Conflict(m),
        AssetError::BadRequest(m) => ApiError::BadRequest(m),
        AssetError::Storage(m) => ApiError::Unavailable(m),
    }
}

fn svc(state: &AppState) -> AssetService {
    AssetService::new(state.pool.clone())
}

#[derive(Deserialize, IntoParams)]
pub struct ListAssetsParams {
    /// 按类型过滤：host / cloud / domain / account / device / other
    pub kind: Option<String>,
    /// 检索词（命中 名称 / 别名 / IP）
    pub q: Option<String>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct AssetRequest {
    pub kind: String,
    pub name: String,
    /// 别名数组（历史写法 / 主机名 / ssh 别名；与名称共享命名空间）
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub note: String,
}

/// 补丁式更新（不传的字段不动；aliases 传了就整体替换）。
#[derive(Deserialize, utoipa::ToSchema)]
pub struct AssetPatchRequest {
    pub kind: Option<String>,
    pub name: Option<String>,
    pub aliases: Option<Vec<String>>,
    pub ip: Option<String>,
    pub os: Option<String>,
    pub note: Option<String>,
}

/// 资产类型模板（建档选类型用；与 core 的 ASSET_KINDS 同源）。
#[utoipa::path(get, path = "/assets/types",
    responses((status = 200, body = [AssetKindDto])))]
pub async fn list_kinds(
    principal: axum::Extension<Principal>,
) -> Result<Json<Vec<AssetKindDto>>, ApiError> {
    require_assets(&principal)?;
    Ok(Json(AssetService::list_kinds()))
}

/// 资产台账列表（可按类型过滤 / 关键词检索）。
#[utoipa::path(get, path = "/assets",
    params(ListAssetsParams),
    responses((status = 200, body = [AssetDto])))]
pub async fn list_assets(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(params): Query<ListAssetsParams>,
) -> Result<Json<Vec<AssetDto>>, ApiError> {
    require_assets(&principal)?;
    let q = params.q.as_deref();
    Ok(Json(
        svc(&state)
            .list(params.kind.as_deref(), q)
            .await
            .map_err(ae)?,
    ))
}

/// 建档一台资产。
#[utoipa::path(post, path = "/assets",
    request_body = AssetRequest,
    responses((status = 201, body = AssetDto)))]
pub async fn create_asset(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<AssetRequest>,
) -> Result<(StatusCode, Json<AssetDto>), ApiError> {
    require_assets(&principal)?;
    let a = svc(&state)
        .create(
            &req.kind,
            &req.name,
            &req.aliases,
            &req.ip,
            &req.os,
            &req.note,
        )
        .await
        .map_err(ae)?;
    Ok((StatusCode::CREATED, Json(a)))
}

/// 读资产详情（本体 + 被哪些项目位置引用）。
#[utoipa::path(get, path = "/assets/{id}",
    responses((status = 200, body = AssetDetailDto)))]
pub async fn get_asset(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<AssetDetailDto>, ApiError> {
    require_assets(&principal)?;
    Ok(Json(svc(&state).get(id).await.map_err(ae)?))
}

/// 编辑资产（补丁式）。
#[utoipa::path(put, path = "/assets/{id}",
    request_body = AssetPatchRequest,
    responses((status = 200, body = AssetDto)))]
pub async fn update_asset(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<AssetPatchRequest>,
) -> Result<Json<AssetDto>, ApiError> {
    require_assets(&principal)?;
    let a = svc(&state)
        .update(
            id,
            req.kind.as_deref(),
            req.name.as_deref(),
            req.aliases.as_deref(),
            req.ip.as_deref(),
            req.os.as_deref(),
            req.note.as_deref(),
        )
        .await
        .map_err(ae)?;
    Ok(Json(a))
}

/// 删除资产（【破坏性】被项目位置引用的会被拒绝——先解绑）。
#[utoipa::path(delete, path = "/assets/{id}",
    responses((status = 204)))]
pub async fn delete_asset(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_assets(&principal)?;
    svc(&state).delete(id).await.map_err(ae)?;
    Ok(StatusCode::NO_CONTENT)
}

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

use crate::auth::{Principal, require_scope, require_scope_read};
use crate::error::ApiError;
use crate::state::AppState;

fn require_assets(p: &Principal) -> Result<(), ApiError> {
    require_scope(p, "assets")
}
/// 读语义变体：:ro 只读 key 放行（RJ-20，对齐 MCP 读动作口径）。
fn require_assets_read(p: &Principal) -> Result<(), ApiError> {
    require_scope_read(p, "assets")
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
    require_assets_read(&principal)?;
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
    require_assets_read(&principal)?;
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
    require_assets_read(&principal)?;
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

// ---------- 运行手册（runbook）——主机运维台账的主动记录面（0062） ----------

/// 读某资产的运行手册 Markdown 全文。
#[utoipa::path(get, path = "/assets/{id}/runbook", responses((status = 200)))]
pub async fn get_runbook(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_assets_read(&principal)?;
    let md = svc(&state).runbook(id).await.map_err(ae)?;
    Ok(Json(
        serde_json::json!({ "asset_id": id, "runbook_md": md }),
    ))
}

/// 保存请求体（Markdown 整体替换；旧文自动入修订史）。
#[derive(Deserialize, utoipa::ToSchema)]
pub struct RunbookSaveRequest {
    pub md: String,
    /// 编辑人标识（可选，留痕用；缺省 console）
    #[serde(default)]
    pub editor: Option<String>,
}

/// 保存运行手册（旧文入修订史——错改可回滚）。
#[utoipa::path(put, path = "/assets/{id}/runbook", responses((status = 200)))]
pub async fn put_runbook(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<RunbookSaveRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_assets(&principal)?;
    let editor = req.editor.as_deref().unwrap_or("console");
    svc(&state)
        .save_runbook(id, &req.md, editor)
        .await
        .map_err(ae)?;
    Ok(Json(serde_json::json!({
        "asset_id": id,
        "saved": true,
        "hint": "旧文已入修订史（GET /assets/{id}/runbook/versions），错改可 POST restore 回滚。",
    })))
}

/// 修订史清单（新→旧）。
#[utoipa::path(get, path = "/assets/{id}/runbook/versions", responses((status = 200)))]
pub async fn runbook_versions(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_assets_read(&principal)?;
    let versions = svc(&state).runbook_versions(id).await.map_err(ae)?;
    Ok(Json(serde_json::json!({
        "asset_id": id,
        "count": versions.len(),
        "versions": serde_json::to_value(&versions).unwrap_or(serde_json::json!([])),
    })))
}

/// 回滚请求体。
#[derive(Deserialize, utoipa::ToSchema)]
pub struct RunbookRestoreRequest {
    pub version_id: Uuid,
    /// 编辑人标识（可选；缺省 console）
    #[serde(default)]
    pub editor: Option<String>,
}

/// 回滚运行手册到某修订（回滚前正文先入史）。
#[utoipa::path(post, path = "/assets/{id}/runbook/restore", responses((status = 200)))]
pub async fn restore_runbook(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<RunbookRestoreRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_assets(&principal)?;
    let editor = req.editor.as_deref().unwrap_or("console");
    svc(&state)
        .restore_runbook(id, req.version_id, editor)
        .await
        .map_err(ae)?;
    Ok(Json(serde_json::json!({
        "asset_id": id,
        "restored_to": req.version_id,
        "hint": "已回滚；回滚前的正文也已入史（可再滚回来）。",
    })))
}

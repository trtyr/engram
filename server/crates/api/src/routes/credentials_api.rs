//! 凭据域端点（credentials scope）——EN-234 机密值一等台账的控制台治理面。
//!
//! 列表/元数据永不回显值；值只在显式 reveal 端点返回且每次取用留审计痕
//! （与 MCP credentials 域同一 core 服务，安全语义零分叉）。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use engram_core::credentials::{CredentialError, CredentialsService};
use serde::Deserialize;

use crate::auth::{Principal, require_scope, require_scope_read};
use crate::error::ApiError;
use crate::state::AppState;

fn require_credentials(p: &Principal) -> Result<(), ApiError> {
    require_scope(p, "credentials")
}
/// 读语义变体：:ro 只读 key 放行（对齐 MCP 读动作口径）。
fn require_credentials_read(p: &Principal) -> Result<(), ApiError> {
    require_scope_read(p, "credentials")
}

fn ce(e: CredentialError) -> ApiError {
    match e {
        CredentialError::NotFound(m) => ApiError::NotFound(m),
        CredentialError::Conflict(m) => ApiError::Conflict(m),
        CredentialError::BadRequest(m) => ApiError::BadRequest(m),
        CredentialError::Crypto(m) => ApiError::Unavailable(m),
        CredentialError::Storage(m) => ApiError::Unavailable(m),
    }
}

fn svc(state: &AppState) -> CredentialsService {
    state.credentials()
}

/// PUT 请求体（sensitive 由服务端恒置 true——控制台不提供降密通道）。
#[derive(Deserialize, utoipa::ToSchema)]
pub struct CredentialPutRequest {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub description: Option<String>,
    /// 分组标签（按系统/环境归组）
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// 到期时间（RFC3339；可选）
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// 台账列表（元数据，永不回显值）。
#[derive(Deserialize, utoipa::IntoParams)]
pub struct ListCredentialsParams {
    /// 按标签过滤
    pub tag: Option<String>,
    /// 关键词（命中 name/description/tags）
    pub q: Option<String>,
}

#[utoipa::path(get, path = "/credentials",
    params(ListCredentialsParams),
    responses((status = 200)))]
pub async fn list_credentials(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ListCredentialsParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_credentials_read(&principal)?;
    let items = svc(&state)
        .list(p.tag.as_deref(), p.q.as_deref())
        .await
        .map_err(ce)?;
    Ok(Json(serde_json::json!({ "items": items })))
}

/// 取用流水（谁/何时；倒序 LIMIT 50）。
#[utoipa::path(get, path = "/credentials/{name}/reads",
    responses((status = 200)))]
pub async fn credential_reads(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_credentials_read(&principal)?;
    let rows = svc(&state).reads(&name).await.map_err(ce)?;
    Ok(Json(serde_json::json!({ "name": name, "reads": rows })))
}

/// 按名取值（显式揭示；每次调用留取用痕——read_count+1、流水+1）。
#[utoipa::path(get, path = "/credentials/{name}/value",
    responses((status = 200)))]
pub async fn reveal_credential(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<engram_storage::models::credential::CredentialValueDto>, ApiError> {
    require_credentials(&principal)?;
    let reader = "console";
    let v = svc(&state).get(&name, reader).await.map_err(ce)?;
    Ok(Json(v))
}

/// 写入/换值（同名换值清零旧取用审计——值变了旧痕作废）。
#[utoipa::path(post, path = "/credentials",
    responses((status = 201)))]
pub async fn put_credential(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<CredentialPutRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    require_credentials(&principal)?;
    let expires_at = match req.expires_at.as_deref() {
        Some(s) if !s.trim().is_empty() => Some(
            chrono::DateTime::parse_from_rfc3339(s.trim())
                .map_err(|_| {
                    ApiError::BadRequest("expires_at 需 RFC3339（如 2026-12-31T00:00:00Z）".into())
                })?
                .with_timezone(&chrono::Utc),
        ),
        _ => None,
    };
    let tags: Vec<String> = req
        .tags
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    let meta = svc(&state)
        .put(
            &req.name,
            &req.value,
            req.description.as_deref(),
            "console",
            &tags,
            expires_at,
        )
        .await
        .map_err(ce)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "credential": meta,
            "hint": "同名换值：旧取用流水已清零（值变了旧痕作废）。",
        })),
    ))
}

/// 删除（级联清取用流水）。
#[utoipa::path(delete, path = "/credentials/{name}",
    responses((status = 200)))]
pub async fn delete_credential(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_credentials(&principal)?;
    let deleted = svc(&state).delete(&name).await.map_err(ce)?;
    if !deleted {
        return Err(ApiError::NotFound(format!("凭据不存在：{name}")));
    }
    Ok(Json(serde_json::json!({ "deleted": name })))
}

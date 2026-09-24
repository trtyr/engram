//! `llm_api` 的实现切片（架构治理 2026-09-21：自 llm_api.rs 纯搬移，零行为变化）。

use super::*;

#[derive(Deserialize, ToSchema)]
pub struct CreateApiKeyRequest {
    pub name: String,
    /// scope 全量集合（EN-62：显式必填——缺字段由 serde 拒绝为 422；未知 scope 值 400，不再有任何隐式默认；
    /// 常见误写「projects」自动归一为「project」）
    pub scopes: Vec<String>,
    /// 可选：过期时间（RFC3339，如 2027-01-01T00:00:00Z）——缺省永不过期；到期后该 key 返回 401（带到期说明）（EN-62）
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize, ToSchema)]
pub struct ApiKeyCreated {
    pub id: Uuid,
    pub name: String,
    /// 明文 key（amk_ 前缀；只在创建响应出现一次）
    pub key: String,
    /// 过期时间（null = 永不过期）
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// 签发 API key。
#[utoipa::path(post, path = "/settings/api-keys",
    request_body = CreateApiKeyRequest,
    responses((status = 201, body = ApiKeyCreated)))]
pub async fn create_api_key_handler(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<ApiKeyCreated>), ApiError> {
    require_admin(&principal)?;
    let (id, key) = create_api_key(&state.pool, &req.name, req.scopes, req.expires_at).await?;
    Ok((
        StatusCode::CREATED,
        Json(ApiKeyCreated {
            id,
            name: req.name,
            key,
            expires_at: req.expires_at,
        }),
    ))
}

#[derive(Serialize, ToSchema)]
pub struct ApiKeyDto {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 过期时间（null = 永不过期）
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// API key 列表（永不含完整 key）。
#[utoipa::path(get, path = "/settings/api-keys", responses((status = 200, body = [ApiKeyDto])))]
pub async fn list_api_keys(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ApiKeyDto>>, ApiError> {
    require_admin(&principal)?;
    let rows = keys_repo::list_api_keys(&state.pool).await?;
    Ok(Json(rows.into_iter().map(key_dto).collect()))
}

/// 删除 API key（物理删除，不留记录）。
#[utoipa::path(post, path = "/settings/api-keys/{id}/revoke", responses((status = 204)))]
pub async fn revoke_api_key(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_admin(&principal)?;
    let deleted = keys_repo::delete_api_key(&state.pool, id).await?;
    if deleted == 0 {
        return Err(ApiError::NotFound(format!("API key {id} 不存在")));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct BatchRevokeRequest {
    pub ids: Vec<Uuid>,
}

#[derive(Serialize, ToSchema)]
pub struct BatchRevokeResult {
    pub revoked: usize,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateApiKeyRequest {
    /// 新名称（不传保持不变）
    pub name: Option<String>,
    /// 新 scope 全量集合（不传保持不变；传 [] 即清空全部权限）
    pub scopes: Option<Vec<String>>,
    /// 过期时间（EN-62）：不传保持不变；null = 改回永不过期
    pub expires_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
}

pub(crate) fn key_dto(r: engram_storage::models::keys::ApiKeyRow) -> ApiKeyDto {
    ApiKeyDto {
        id: r.id,
        name: r.name,
        key_prefix: r.key_prefix,
        scopes: r.scopes,
        created_at: r.created_at,
        last_used_at: r.last_used_at,
        revoked_at: r.revoked_at,
        expires_at: r.expires_at,
    }
}

/// 编辑已有 API key：改名 / 调整 scope（全量替换，即时生效——bearer 每请求查库，无需吊销重签）。
#[utoipa::path(put, path = "/settings/api-keys/{id}",
    request_body = UpdateApiKeyRequest,
    responses(
        (status = 200, body = ApiKeyDto),
        (status = 400, body = crate::error::ErrorEnvelope),
        (status = 404, body = crate::error::ErrorEnvelope)))]
pub async fn update_api_key(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateApiKeyRequest>,
) -> Result<Json<ApiKeyDto>, ApiError> {
    require_admin(&principal)?;
    if let Some(name) = req.name.as_deref() {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            return Err(ApiError::BadRequest("名称需 1~64 字符".into()));
        }
    }
    let scopes = req
        .scopes
        .as_ref()
        .map(|list| {
            list.iter()
                .map(|s| {
                    crate::auth::normalize_scope(s)
                        .ok_or_else(|| ApiError::BadRequest(crate::auth::unknown_scope_message(s)))
                })
                .collect::<Result<Vec<String>, _>>()
        })
        .transpose()?;
    let n = keys_repo::update_api_key(
        &state.pool,
        id,
        req.name.as_deref().map(str::trim),
        scopes.as_deref(),
        req.expires_at,
    )
    .await?;
    if n == 0 {
        return Err(ApiError::NotFound(format!("API key {id} 不存在")));
    }
    let row = keys_repo::get_api_key(&state.pool, id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("API key {id} 不存在")))?;
    Ok(Json(key_dto(row)))
}

/// 批量删除 API key（物理删除，返回实际删除数）。
#[utoipa::path(post, path = "/settings/api-keys/batch-revoke",
    request_body = BatchRevokeRequest,
    responses((status = 200, body = BatchRevokeResult)))]
pub async fn batch_revoke_api_keys(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<BatchRevokeRequest>,
) -> Result<Json<BatchRevokeResult>, ApiError> {
    require_admin(&principal)?;
    if req.ids.is_empty() {
        return Err(ApiError::BadRequest("ids 不能为空".into()));
    }
    let revoked = keys_repo::delete_api_keys(&state.pool, &req.ids).await?;
    Ok(Json(BatchRevokeResult {
        revoked: revoked as usize,
    }))
}

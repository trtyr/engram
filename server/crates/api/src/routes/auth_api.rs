//! 登录 / 管理员账号 / 会话管理端点。

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use engram_storage::repo::keys as keys_repo;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use crate::auth::Principal;
use crate::error::ApiError;
use crate::state::AppState;

fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn default_username() -> String {
    "admin".into()
}

#[derive(Deserialize, ToSchema)]
pub struct LoginRequest {
    /// 用户名（缺省 admin——env 兼容模式与默认账号；改过用户名后必须显式传）
    #[serde(default = "default_username")]
    pub username: String,
    pub password: String,
}

#[derive(Serialize, ToSchema)]
pub struct LoginResponse {
    /// 会话 token（ams_ 前缀，7 天有效；只在登录响应出现一次）
    pub token: String,
}

fn env_password(state: &AppState) -> Option<String> {
    state.admin_password.as_ref().map(|p| p.0.clone())
}

/// 管理员登录（唯一无鉴权写端点；Web UI 用）。
#[utoipa::path(post, path = "/auth/login",
    request_body = LoginRequest,
    responses(
        (status = 200, body = LoginResponse),
        (status = 401, body = crate::error::ErrorEnvelope),
    ))]
pub async fn login_handler(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let (token, _hash) = crate::auth::login(
        &state.pool,
        &req.username,
        &req.password,
        env_password(&state).as_deref(),
    )
    .await?;
    Ok(Json(LoginResponse { token }))
}

/// 当前用户名（管理页展示用；未初始化 → null）。
#[utoipa::path(get, path = "/auth/username", responses((status = 200, body = Object)))]
pub async fn username(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !matches!(principal.0, Principal::Admin) {
        return Err(ApiError::Forbidden("仅管理员会话可查看账号".into()));
    }
    let username = engram_core::auth::get_username(&state.pool)
        .await
        .map_err(ApiError::Unavailable)?;
    Ok(Json(serde_json::json!({ "username": username })))
}

/// 登录状态（无鉴权，登录页用）：账号是否已初始化。
#[utoipa::path(get, path = "/auth/status", responses((status = 200, body = Object)))]
pub async fn status(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let initialized = engram_core::auth::account_exists(&state.pool)
        .await
        .map_err(ApiError::Unavailable)?;
    Ok(Json(serde_json::json!({
        "initialized": initialized,
        "env_fallback": state.admin_password.is_some(),
    })))
}

#[derive(Deserialize, ToSchema)]
pub struct InitAccountRequest {
    /// 用户名（3~32 字符：字母/数字/_-.）
    pub username: String,
    /// 密码（≥8 位）
    pub password: String,
}

/// 初始化管理员账号（仅账号未创建时有效；创建后即以此登录）。
#[utoipa::path(post, path = "/auth/init", request_body = InitAccountRequest,
    responses((status = 201, body = LoginResponse), (status = 409, body = crate::error::ErrorEnvelope)))]
pub async fn init_account(
    State(state): State<AppState>,
    Json(req): Json<InitAccountRequest>,
) -> Result<(StatusCode, Json<LoginResponse>), ApiError> {
    let username = req.username.trim();
    if username.len() < 3
        || username.len() > 32
        || !username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err(ApiError::BadRequest(
            "用户名需 3~32 字符（字母/数字/_-.）".into(),
        ));
    }
    if req.password.chars().count() < 8 {
        return Err(ApiError::BadRequest("密码至少 8 位".into()));
    }
    let created = engram_core::auth::init_account(&state.pool, username, &req.password)
        .await
        .map_err(ApiError::Unavailable)?;
    if !created {
        return Err(ApiError::Conflict("账号已存在——直接登录即可".into()));
    }
    let (token, _hash) = crate::auth::login(&state.pool, username, &req.password, None).await?;
    Ok((StatusCode::CREATED, Json(LoginResponse { token })))
}

#[derive(Deserialize, ToSchema)]
pub struct ChangeCredentialsRequest {
    /// 当前密码（必填校验）
    pub current_password: String,
    /// 新用户名（可选；不传保持不变）
    pub new_username: Option<String>,
    /// 新密码（可选；不传保持不变）
    pub new_password: Option<String>,
}

/// 修改用户名/密码（改后自动吊销其他会话；当前会话保留）。
#[utoipa::path(put, path = "/auth/account", request_body = ChangeCredentialsRequest,
    responses((status = 200, body = Object), (status = 400, body = crate::error::ErrorEnvelope)))]
pub async fn change_credentials(
    principal: axum::Extension<Principal>,
    headers: axum::http::HeaderMap,
    State(state): State<AppState>,
    Json(req): Json<ChangeCredentialsRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // 仅管理员会话（API key 无账号语义）
    if !matches!(principal.0, Principal::Admin) {
        return Err(ApiError::Forbidden("仅管理员会话可修改账号".into()));
    }
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::Unauthorized("缺 Bearer 凭证".into()))?;
    let token_hash = sha256_hex(token);

    let (ok, body) = engram_core::auth::change_credentials(
        &state.pool,
        &req.current_password,
        req.new_username.as_deref(),
        req.new_password.as_deref(),
        &token_hash,
    )
    .await
    .map_err(ApiError::BadRequest)?;
    if !ok {
        return Err(ApiError::BadRequest(
            body.get("error")
                .and_then(|x| x.as_str())
                .unwrap_or("当前密码错误")
                .into(),
        ));
    }
    Ok(Json(body))
}

#[derive(Serialize, ToSchema)]
pub struct AdminSessionDto {
    /// 会话标识（token hash 前 12 位）
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    /// 是否为当前请求的会话
    pub current: bool,
}

/// 会话列表（活跃会话；标记当前会话）。
#[utoipa::path(get, path = "/auth/sessions", responses((status = 200, body = [AdminSessionDto])))]
pub async fn list_sessions(
    principal: axum::Extension<Principal>,
    headers: axum::http::HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<Vec<AdminSessionDto>>, ApiError> {
    if !matches!(principal.0, Principal::Admin) {
        return Err(ApiError::Forbidden("仅管理员会话可管理会话".into()));
    }
    let current_hash = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(sha256_hex);
    let rows = keys_repo::list_admin_sessions(&state.pool)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(
        rows.into_iter()
            .map(
                |(hash, created_at, expires_at, last_used_at)| AdminSessionDto {
                    current: current_hash.as_deref() == Some(hash.as_str()),
                    id: hash.chars().take(12).collect(),
                    created_at,
                    expires_at,
                    last_used_at,
                },
            )
            .collect(),
    ))
}

/// 吊销指定会话（不可吊销当前会话——用「吊销其他」或登出）。
#[utoipa::path(delete, path = "/auth/sessions/{id}", responses((status = 204), (status = 404, body = crate::error::ErrorEnvelope)))]
pub async fn revoke_session(
    principal: axum::Extension<Principal>,
    headers: axum::http::HeaderMap,
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<StatusCode, ApiError> {
    if !matches!(principal.0, Principal::Admin) {
        return Err(ApiError::Forbidden("仅管理员会话可管理会话".into()));
    }
    let current_hash = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(sha256_hex);
    let rows = keys_repo::list_admin_sessions(&state.pool)
        .await
        .map_err(ApiError::from)?;
    let Some((hash, ..)) = rows
        .into_iter()
        .find(|(h, ..)| h.chars().take(12).collect::<String>() == id)
    else {
        return Err(ApiError::NotFound(format!("会话 {id} 不存在或已过期")));
    };
    if Some(hash.as_str()) == current_hash.as_deref() {
        return Err(ApiError::BadRequest(
            "不能吊销当前会话——登出其他设备用 POST /auth/sessions/revoke-others".into(),
        ));
    }
    let n = keys_repo::delete_admin_session(&state.pool, &hash)
        .await
        .map_err(ApiError::from)?;
    if n == 0 {
        return Err(ApiError::NotFound(format!("会话 {id} 不存在")));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
pub struct RevokeOthersResult {
    pub revoked: u64,
}

/// 吊销除当前会话外的全部会话（改密码后自动执行；也可手动触发）。
#[utoipa::path(post, path = "/auth/sessions/revoke-others",
    responses((status = 200, body = RevokeOthersResult)))]
pub async fn revoke_others(
    principal: axum::Extension<Principal>,
    headers: axum::http::HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<RevokeOthersResult>, ApiError> {
    if !matches!(principal.0, Principal::Admin) {
        return Err(ApiError::Forbidden("仅管理员会话可管理会话".into()));
    }
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::Unauthorized("缺 Bearer 凭证".into()))?;
    let revoked = keys_repo::delete_other_admin_sessions(&state.pool, &sha256_hex(token))
        .await
        .map_err(ApiError::from)?;
    Ok(Json(RevokeOthersResult { revoked }))
}

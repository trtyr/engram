//! 登录端点。

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::auth::login;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Deserialize, utoipa::ToSchema)]
pub struct LoginRequest {
    pub password: String,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct LoginResponse {
    /// 会话 token（ams_ 前缀，7 天有效；只在登录响应出现一次）
    pub token: String,
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
    let Some(pw) = &state.admin_password else {
        return Err(ApiError::Unavailable(
            "服务端未配置管理员密码（AGENT_MEMORY_ADMIN_PASSWORD）".into(),
        ));
    };
    let token = login(&state.pool, &req.password, &pw.0).await?;
    Ok(Json(LoginResponse { token }))
}

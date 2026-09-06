//! 鉴权：管理员会话（opaque token, D0011）+ API key（scopes）。
//! Bearer 中间件：admin session → admin 上下文；api key → 携带 scopes 的机器上下文。

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::{Duration, Utc};
use engram_storage::PgPool;
use engram_storage::repo::keys as repo;
use rand::RngCore;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::{ApiError, ErrorBody, ErrorEnvelope};

pub use engram_core::auth::{Principal, SCOPES};

fn sha256_hex(input: &str) -> String {
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 登录：校验用户名 + 密码（账号表；空表回退 env 密码）→ 颁发 opaque 会话 token。
/// 返回 (明文 token, token_hash)——token_hash 供「会话管理」标记当前会话。
pub async fn login(
    pool: &PgPool,
    username: &str,
    password: &str,
    env_fallback: Option<&str>,
) -> Result<(String, String), ApiError> {
    let ok = engram_core::auth::verify_login(pool, username, password, env_fallback)
        .await
        .map_err(ApiError::Unavailable)?;
    if !ok {
        return Err(ApiError::Unauthorized("用户名或密码错误".into()));
    }

    let mut raw = [0u8; 32];
    rand::rng().fill_bytes(&mut raw);
    let token = format!("ams_{}", hex(&raw));
    let hash = sha256_hex(&token);
    let expires = Utc::now() + Duration::days(7);

    repo::create_admin_session(pool, &hash, expires)
        .await
        .map_err(ApiError::from)?;

    Ok((token, hash))
}

/// 校验管理员会话 token。
pub async fn validate_admin_session(pool: &PgPool, token: &str) -> Result<bool, ApiError> {
    let hash = sha256_hex(token);
    let expires = repo::find_admin_session_expiry(pool, &hash)
        .await
        .map_err(ApiError::from)?;
    match expires {
        Some(expires) if expires > Utc::now() => {
            // 更新 last_used（失败不阻塞）
            let _ = repo::touch_admin_session(pool, &hash).await;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// 签发 API key（明文只返回一次）。
pub async fn create_api_key(
    pool: &PgPool,
    name: &str,
    scopes: Vec<String>,
) -> Result<(Uuid, String), ApiError> {
    for s in &scopes {
        if !SCOPES.contains(&s.as_str()) {
            return Err(ApiError::BadRequest(format!("未知 scope: {s}")));
        }
    }
    let mut raw = [0u8; 24];
    rand::rng().fill_bytes(&mut raw);
    let key = format!("amk_{}", hex(&raw));
    let id = Uuid::now_v7();
    repo::insert_api_key(pool, id, name, &sha256_hex(&key), &key[..12], &scopes)
        .await
        .map_err(ApiError::from)?;
    Ok((id, key))
}

/// Bearer 中间件：认证并注入 Principal 扩展。
/// 无 Authorization 头 / 无效凭证 → 401；认证成功但后续 scope 检查在各 handler。
pub async fn bearer_auth(
    axum::extract::State(state): axum::extract::State<crate::state::AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    // SPA 导航分流（D-001）：/jobs 与前端路由同路径——浏览器硬刷新（Accept: text/html*）
    // 直接回 SPA 页，不要求 Bearer；API 客户端（JSON Accept）照常走认证。
    let accept_html = req
        .headers()
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|a| a.starts_with("text/html"));
    // SPA 导航分流（D-001）：/jobs、/projects 及子路径与前端路由同路径——
    // 浏览器硬刷新（Accept: text/html*）直接回 SPA 页，不要求 Bearer；
    // API 客户端（JSON Accept）照常走认证。
    if accept_html && is_spa_nav_path(req.uri().path()) {
        return crate::web_assets::static_handler(req.uri().clone()).await;
    }

    let header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let Some(token) = header else {
        return auth_error(
            "缺少 Bearer 凭证——请在请求头加 Authorization: Bearer <token>（管理员登录拿 ams_ 会话，AI 客户端用设置页签发的 amk_ key）",
        );
    };

    let principal = authenticate(&state.pool, token).await;
    let principal = match principal {
        Ok(Some(p)) => p,
        Ok(None) => return auth_error("凭证无效或已过期"),
        Err(e) => return e.into_response(),
    };

    req.extensions_mut().insert(principal);
    next.run(req).await
}

async fn authenticate(pool: &PgPool, token: &str) -> Result<Option<Principal>, ApiError> {
    // 1) 管理员会话（ams_ 前缀）
    if token.starts_with("ams_") {
        return if validate_admin_session(pool, token).await? {
            Ok(Some(Principal::Admin))
        } else {
            Ok(None)
        };
    }

    // 2) API key（amk_ 前缀）
    if token.starts_with("amk_") {
        let hash = sha256_hex(token);
        if let Some((key_id, name, scopes)) = repo::find_api_key_by_hash(pool, &hash)
            .await
            .map_err(ApiError::from)?
        {
            // 更新 last_used（失败不阻塞）
            let _ = repo::touch_api_key(pool, key_id).await;
            return Ok(Some(Principal::ApiKey {
                key_id,
                name,
                scopes,
            }));
        }
    }
    Ok(None)
}

/// scope 检查辅助（handler 用）。
pub fn require_scope(principal: &Principal, scope: &str) -> Result<(), ApiError> {
    if principal.has_scope(scope) {
        Ok(())
    } else {
        Err(ApiError::Forbidden(format!("缺少 scope: {scope}")))
    }
}

/// SPA 页面路由（与 API 端点同路径，浏览器导航时靠 Accept: text/html 区分）。
/// /mcp 与 MCP Streamable HTTP 端点同路径：MCP 客户端发 JSON Accept 照常认证，
/// 浏览器导航（text/html）回 SPA 页——否则硬刷新 /mcp 会 401。
fn is_spa_nav_path(path: &str) -> bool {
    path == "/jobs"
        || path == "/projects"
        || path.starts_with("/projects/")
        || path == "/skills"
        || path.starts_with("/skills/")
        || path == "/todos"
        || path == "/mcp"
}

fn auth_error(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(ErrorEnvelope {
            error: ErrorBody {
                code: "unauthorized",
                message: message.into(),
                retryable: false,
                details: None,
            },
        }),
    )
        .into_response()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

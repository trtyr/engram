//! 鉴权：管理员会话（opaque token, D0011）+ API key（scopes）。
//! Bearer 中间件：admin session → admin 上下文；api key → 携带 scopes 的机器上下文。

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::{Duration, Utc};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ErrorBody, ErrorEnvelope};

/// 资产域 scope。
pub const SCOPES: [&str; 8] = [
    "memory",
    "knowledge",
    "wiki",
    "codegraph",
    "project",
    "llm",
    "erase",
    "cron",
];

/// 已认证主体。
#[derive(Debug, Clone)]
pub enum Principal {
    /// 管理员（Web UI 会话，全权限）
    Admin,
    /// API key 机器主体（限 scopes）
    ApiKey {
        key_id: Uuid,
        name: String,
        scopes: Vec<String>,
    },
}

impl Principal {
    /// 是否拥有某 scope（Admin 恒真）。
    pub fn has_scope(&self, scope: &str) -> bool {
        match self {
            Principal::Admin => true,
            Principal::ApiKey { scopes, .. } => scopes.iter().any(|s| s == scope),
        }
    }
}

fn sha256_hex(input: &str) -> String {
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 登录：校验密码 → 颁发 opaque 会话 token（明文只返回一次）。
pub async fn login(pool: &PgPool, password: &str, expect: &str) -> Result<String, ApiError> {
    // 恒定时间比较（防时序侧信道；单用户场景仍按规范做）
    let a = sha256_hex(password);
    let b = sha256_hex(expect);
    if !constant_time_eq(a.as_bytes(), b.as_bytes()) {
        return Err(ApiError::Unauthorized("密码错误".into()));
    }

    let mut raw = [0u8; 32];
    rand::rng().fill_bytes(&mut raw);
    let token = format!("ams_{}", hex(&raw));
    let hash = sha256_hex(&token);
    let expires = Utc::now() + Duration::days(7);

    sqlx::query("INSERT INTO admin_sessions (token_hash, expires_at) VALUES ($1, $2)")
        .bind(&hash)
        .bind(expires)
        .execute(pool)
        .await
        .map_err(ApiError::from)?;

    Ok(token)
}

/// 校验管理员会话 token。
pub async fn validate_admin_session(pool: &PgPool, token: &str) -> Result<bool, ApiError> {
    let hash = sha256_hex(token);
    let row: Option<(chrono::DateTime<Utc>,)> =
        sqlx::query_as("SELECT expires_at FROM admin_sessions WHERE token_hash = $1")
            .bind(&hash)
            .fetch_optional(pool)
            .await
            .map_err(ApiError::from)?;
    match row {
        Some((expires,)) if expires > Utc::now() => {
            // 更新 last_used（失败不阻塞）
            let _ =
                sqlx::query("UPDATE admin_sessions SET last_used_at = now() WHERE token_hash = $1")
                    .bind(&hash)
                    .execute(pool)
                    .await;
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
    sqlx::query(
        "INSERT INTO api_keys (id, name, key_hash, key_prefix, scopes) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(name)
    .bind(sha256_hex(&key))
    .bind(&key[..12])
    .bind(sqlx::types::Json(&scopes))
    .execute(pool)
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
    if accept_html && req.uri().path() == "/jobs" {
        return crate::web_assets::static_handler(req.uri().clone()).await;
    }

    let header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let Some(token) = header else {
        return auth_error("缺少 Bearer 凭证");
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
        let row: Option<(Uuid, String, sqlx::types::Json<Vec<String>>)> = sqlx::query_as(
            "SELECT id, name, scopes FROM api_keys WHERE key_hash = $1 AND revoked_at IS NULL",
        )
        .bind(&hash)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::from)?;
        if let Some((key_id, name, scopes)) = row {
            // 更新 last_used（失败不阻塞）
            let _ = sqlx::query("UPDATE api_keys SET last_used_at = now() WHERE id = $1")
                .bind(key_id)
                .execute(pool)
                .await;
            return Ok(Some(Principal::ApiKey {
                key_id,
                name,
                scopes: scopes.0,
            }));
        }
        // 区分「已撤销」与「不存在」：只有持完整 key 才能触发此查询，不额外泄露信息；
        // 运维排查时能一眼看出是 key 被撤销而非抄错（2026-08-31 测试方实测痛点）。
        let revoked: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM api_keys WHERE key_hash = $1 AND revoked_at IS NOT NULL",
        )
        .bind(&hash)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::from)?;
        if revoked.is_some() {
            return Err(ApiError::Unauthorized(
                "API key 已被撤销（revoked）——请在 设置→API 密钥 重新签发".into(),
            ));
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

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

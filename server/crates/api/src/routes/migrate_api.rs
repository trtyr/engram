//! 系统迁移端点（admin-only）：全系统导出 / 导入 / 远程拉取。
//!
//! 迁移场景：A、B 两台部署机，B 向 A 发起拉取（pull）——B 的管理员提供
//! A 的地址与 A 的 admin 密码，B 后端登录 A → 拉迁移包 → 落地。
//! A 的密码只在请求期使用，不落库不缓存。

use axum::Json;
use axum::extract::State;
use engram_core::transfer::{self, TransferError};
use serde::Deserialize;

use crate::auth::Principal;
use crate::error::ApiError;
use crate::state::AppState;

fn te(e: TransferError) -> ApiError {
    match e {
        TransferError::BadRequest(m) => ApiError::BadRequest(m),
        TransferError::Storage(m) => ApiError::Unavailable(m),
    }
}

fn require_admin(p: &Principal) -> Result<(), ApiError> {
    match p {
        Principal::Admin => Ok(()),
        Principal::ApiKey { .. } => Err(ApiError::Forbidden("迁移仅限管理员会话".into())),
    }
}

/// 全系统导出：五域迁移包（JSON 下载；派生列 embedding/tsv 不含——导入端重建）。
#[utoipa::path(get, path = "/migrate/export", responses((status = 200, body = Object)))]
pub async fn export_bundle(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&principal)?;
    Ok(Json(
        transfer::export_bundle(&state.pool).await.map_err(te)?,
    ))
}

/// 导入迁移包（冲突跳过，分域报告）。合并语义：已存在的记录不动。
#[utoipa::path(post, path = "/migrate/import", request_body = Object,
    responses((status = 200, body = Object)))]
pub async fn import_bundle(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(data): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&principal)?;
    Ok(Json(
        transfer::import_bundle(&state.pool, &data)
            .await
            .map_err(te)?,
    ))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct PullRequest {
    /// A 机基地址（如 http://a-host:8080）
    pub source_url: String,
    /// A 机的管理员密码（仅请求期使用，不落库）
    pub source_admin_password: String,
}

/// 远程拉取迁移（A → B 一键迁移）：登录 A 拉迁移包，落地到本机（冲突跳过）。
#[utoipa::path(post, path = "/migrate/pull", request_body = PullRequest,
    responses((status = 200, body = Object)))]
pub async fn pull(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<PullRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&principal)?;
    let base = req.source_url.trim().trim_end_matches('/').to_string();
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(ApiError::BadRequest(
            "source_url 必须 http(s):// 开头".into(),
        ));
    }

    let report = transfer::pull_from(&state.pool, &base, &req.source_admin_password)
        .await
        .map_err(te)?;
    Ok(Json(report))
}

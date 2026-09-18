//! 系统迁移端点：全系统导出 / 导入 / 远程拉取。
//!
//! 准入（2026-09-18 数据同步线放宽）：export/import 接受管理员会话或
//! migrate scope 的 API key（公网同步专用钥匙，admin 密码不过公网）；
//! pull 保持 admin-only（远程拉取需向对端提供 admin 密码，是管理员操作）。
//!
//! 迁移场景：A、B 两台部署机，B 向 A 发起拉取（pull）——B 的管理员提供
//! A 的地址与 A 的 admin 密码，B 后端登录 A → 拉迁移包 → 落地。
//! A 的密码只在请求期使用，不落库不缓存。

use axum::Json;
use axum::extract::State;
use engram_core::auth::DomainAccess;
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

/// export/import 准入（2026-09-18 数据同步线放宽）：管理员会话，或携带
/// migrate scope 的 API key（含 :ro 变体——同步钥匙只碰这两个端点）。
fn require_migrate(p: &Principal) -> Result<(), ApiError> {
    match p {
        Principal::Admin => Ok(()),
        Principal::ApiKey { .. } if p.domain_access("migrate") != DomainAccess::None => Ok(()),
        Principal::ApiKey { .. } => Err(ApiError::Forbidden(
            "迁移接口需要管理员会话或 migrate scope 的 API key".into(),
        )),
    }
}

/// 全系统导出：五域迁移包（JSON 下载；派生列 embedding/tsv 不含——导入端重建）。
#[utoipa::path(get, path = "/migrate/export", responses((status = 200, body = Object)))]
pub async fn export_bundle(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_migrate(&principal)?;
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
    require_migrate(&principal)?;
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

/// 双向同步转发请求（CLI sync 的服务端形态）。
#[derive(Deserialize, utoipa::ToSchema)]
pub struct SyncRequest {
    /// 目标 engram 基地址（如 https://cloud.example.com）
    pub target_url: String,
    /// 方向：push（本地→目标）或 pull（目标→本地）
    pub direction: String,
    /// 目标侧 migrate scope 的 API key（Bearer 直连目标）
    pub token: String,
    /// 逃生口：目标非 loopback 时允许 http 明文
    #[serde(default)]
    pub allow_insecure: bool,
    /// 只导出对比不写入
    #[serde(default)]
    pub dry_run: bool,
}

/// 双向同步转发报告：源包分域计数 +（非 dry_run 时）目标导入报告。
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct SyncReport {
    pub direction: String,
    pub source_counts: serde_json::Value,
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_report: Option<serde_json::Value>,
}

/// 双向同步转发：push = 本地 export → POST 目标 import；pull = GET 目标 export → 灌本地。
/// 目标凭证用 migrate scope key（admin 密码不过目标网络）；目标非 loopback 强制 https。
#[utoipa::path(post, path = "/migrate/sync", request_body = SyncRequest,
    responses((status = 200, body = SyncReport)))]
pub async fn migrate_sync(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<SyncRequest>,
) -> Result<Json<SyncReport>, ApiError> {
    require_migrate(&principal)?;
    let outcome = transfer::sync_transfer(
        &state.pool,
        &req.direction,
        &req.target_url,
        &req.token,
        req.allow_insecure,
        req.dry_run,
    )
    .await
    .map_err(te)?;
    Ok(Json(SyncReport {
        direction: outcome.direction.to_string(),
        source_counts: outcome.source_counts,
        dry_run: outcome.dry_run,
        import_report: outcome.import_report,
    }))
}

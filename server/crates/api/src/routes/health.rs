//! 存活与就绪探针。

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Serialize, utoipa::ToSchema)]
pub struct HealthBody {
    pub status: &'static str,
}

/// 存活探针：进程活着即 200，不查依赖。
#[utoipa::path(get, path = "/health", responses((status = 200, body = HealthBody)))]
pub async fn health() -> Json<HealthBody> {
    Json(HealthBody { status: "ok" })
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ReadyBody {
    pub status: &'static str,
    pub migration_version: Option<i64>,
}

/// 就绪探针：DB 连通 + 迁移已应用才算 ready（compose healthcheck 用）。
#[utoipa::path(get, path = "/ready",
    responses(
        (status = 200, body = ReadyBody),
        (status = 503, body = crate::error::ErrorEnvelope),
    ))]
pub async fn ready(State(state): State<AppState>) -> Result<Json<ReadyBody>, ApiError> {
    let version = engram_storage::current_version(&state.pool).await?;
    if version.is_none() {
        return Err(ApiError::Unavailable("迁移尚未应用".into()));
    }
    Ok(Json(ReadyBody {
        status: "ready",
        migration_version: version,
    }))
}

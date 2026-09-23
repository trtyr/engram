//! 节律管理端点（Web 控制台「设置 → 节律」）：配置读写。
//! 节律本体在 engram-distill（jobs 基建自续任务，内置节律线 roadmap v3）；
//! 这里只是 HTTP 壳（管理员鉴权 + 400 语义），与 mcp_admin.rs 同构。

use axum::Json;
use engram_distill::RhythmConfig;
use engram_jobs::JobQueue;

use crate::auth::Principal;
use crate::error::ApiError;
use crate::state::AppState;

/// 读节律配置（settings 缺行 → 缺省：开 / 6 小时增量 / 每日 3 点整理）。
#[utoipa::path(get, path = "/settings/rhythm",
    responses((status = 200, body = serde_json::Value)))]
pub async fn get_rhythm_config(
    principal: axum::Extension<Principal>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<RhythmConfig>, ApiError> {
    if !matches!(principal.0, Principal::Admin) {
        return Err(ApiError::Forbidden("仅限管理员".into()));
    }
    Ok(Json(engram_distill::rhythm::load_config(&state.pool).await))
}

/// 更新节律配置（总开关 / 增量周期 / 每日整理钟点）。
///
/// enabled=true 时即刻确保下一期在队——新周期/钟点产生新幂等键，改完即生效；
/// 停用不动在队任务：下一期跑完自然停（handler 运行时检测 enabled，双保险）。
#[utoipa::path(put, path = "/settings/rhythm",
    request_body = serde_json::Value,
    responses((status = 200, body = serde_json::Value), (status = 400, body = crate::error::ErrorEnvelope)))]
pub async fn put_rhythm_config(
    principal: axum::Extension<Principal>,
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(cfg): Json<RhythmConfig>,
) -> Result<Json<RhythmConfig>, ApiError> {
    if !matches!(principal.0, Principal::Admin) {
        return Err(ApiError::Forbidden("仅限管理员".into()));
    }
    if !(1..=720).contains(&cfg.extract_every_hours) {
        return Err(ApiError::BadRequest(format!(
            "extract_every_hours 需在 1-720 之间（收到 {}）",
            cfg.extract_every_hours
        )));
    }
    if cfg.consolidate_hour_local > 23 {
        return Err(ApiError::BadRequest(format!(
            "consolidate_hour_local 需在 0-23 之间（收到 {}）",
            cfg.consolidate_hour_local
        )));
    }
    engram_distill::rhythm::save_config(&state.pool, &cfg)
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    if cfg.enabled {
        let queue = JobQueue::new(state.pool.clone());
        engram_distill::rhythm::schedule_next(&queue, &cfg).await?;
    }
    Ok(Json(cfg))
}

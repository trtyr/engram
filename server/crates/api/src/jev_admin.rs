//! JEV 决策模型管理端点（Web 控制台「设置 → AI 功能」）：配置读写。
//! 决策 001（2026-10-03）：接口仅支持 OpenRouter（无 provider 选择器）；
//! api_key 经 KeyCipher 加密落 settings，永不在 GET 回显。与 rhythm_admin 同构。

use axum::Json;
use serde::Deserialize;

use crate::auth::Principal;
use crate::error::ApiError;
use crate::state::AppState;

fn require_admin(principal: &Principal) -> Result<(), ApiError> {
    if !matches!(principal, Principal::Admin) {
        return Err(ApiError::Forbidden("仅限管理员".into()));
    }
    Ok(())
}

fn cipher_from(state: &AppState) -> Result<engram_llm::crypto::KeyCipher, ApiError> {
    let hex_master = state
        .master_key
        .as_ref()
        .map(|m| m.0.clone())
        .ok_or_else(|| {
            ApiError::Unavailable("服务端未配置主密钥（AGENT_MEMORY_MASTER_KEY）".into())
        })?;
    engram_llm::crypto::KeyCipher::from_hex_master(&hex_master)
        .map_err(|e| ApiError::Unavailable(e.to_string()))
}

/// 读 JEV 配置（脱敏——key 只给 key_configured 布尔）。
pub async fn get_jev_config(
    axum::Extension(principal): axum::Extension<Principal>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&principal)?;
    let cfg: engram_llm::JevConfig =
        engram_storage::repo::settings::get_json(&state.pool, engram_llm::decisions::SETTINGS_KEY)
            .await
            .unwrap_or_default();
    Ok(Json(engram_llm::decisions::masked(&cfg)))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct PutJevRequest {
    /// 明文 API key（只在请求中出现）。None = 不改动已存值；空串 = 忽略。
    #[serde(default)]
    pub api_key: Option<String>,
    pub enabled: Option<bool>,
    pub model: Option<String>,
    pub reject_threshold: Option<f64>,
    pub review_threshold: Option<f64>,
}

/// 更新 JEV 配置。api_key 提供非空值时加密覆盖；缺省保留旧值。
/// 改动即时生效（每次蒸馏/KV 写入现读 settings，无缓存）。
pub async fn put_jev_config(
    axum::Extension(principal): axum::Extension<Principal>,
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(req): Json<PutJevRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&principal)?;

    let mut cfg: engram_llm::JevConfig =
        engram_storage::repo::settings::get_json(&state.pool, engram_llm::decisions::SETTINGS_KEY)
            .await
            .unwrap_or_default();

    if let Some(enabled) = req.enabled {
        cfg.enabled = enabled;
    }
    if let Some(model) = req.model {
        let model = model.trim().to_string();
        if model.is_empty() {
            return Err(ApiError::BadRequest("model 不能为空".into()));
        }
        cfg.model = model;
    }
    if let Some(t) = req.reject_threshold {
        cfg.reject_threshold = t;
    }
    if let Some(t) = req.review_threshold {
        cfg.review_threshold = t;
    }
    if !(0.0..cfg.review_threshold).contains(&cfg.reject_threshold) || cfg.review_threshold >= 1.0 {
        return Err(ApiError::BadRequest(format!(
            "阈值需满足 0 < reject({}) < review({}) < 1",
            cfg.reject_threshold, cfg.review_threshold
        )));
    }
    if let Some(key) = req.api_key {
        let key = key.trim();
        if !key.is_empty() {
            let cipher = cipher_from(&state)?;
            cfg.api_key_enc = Some(engram_llm::decisions::encrypt_key(key, &cipher)?);
        }
    }
    if cfg.enabled && !cfg.key_configured() {
        return Err(ApiError::BadRequest("启用前需先配置 API key".into()));
    }

    engram_storage::repo::settings::put_json(
        &state.pool,
        engram_llm::decisions::SETTINGS_KEY,
        &cfg,
    )
    .await
    .map_err(ApiError::Database)?;
    Ok(Json(engram_llm::decisions::masked(&cfg)))
}

//! `llm_api` 的实现切片（架构治理 2026-09-21：自 llm_api.rs 纯搬移，零行为变化）。

use super::*;

/// 用量记录（近 500 条）。
#[utoipa::path(get, path = "/llm/usage", responses((status = 200, body = [UsageRecord])))]
pub async fn usage(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<UsageRecord>>, ApiError> {
    require_llm(&principal)?;
    let registry = ProviderRegistry::new(
        state.pool.clone(),
        KeyCipher::from_hex_master(&"00".repeat(32))
            .map_err(|e| ApiError::Unavailable(e.to_string()))?,
    );
    Ok(Json(
        registry
            .usage_summary(chrono::Utc::now() - chrono::Duration::days(30))
            .await?,
    ))
}

#[derive(Deserialize, ToSchema)]
pub struct FetchModelsRequest {
    /// base_url（新建未保存场景直接传）
    pub base_url: String,
    /// 明文 API key（新建场景直接传；编辑已存 provider 时可省略——用库存密钥）
    pub api_key: Option<String>,
    /// 已保存 provider 的 id（api_key 省略时用其库存密钥解密）
    pub provider_id: Option<Uuid>,
}

/// 自动获取供应商的模型 ID 列表（OpenAI 兼容 GET /models）。
/// 两种用法：新建表单传 base_url+api_key；编辑已存 provider 传 base_url+provider_id。
#[utoipa::path(post, path = "/settings/llm/providers/models",
    request_body = FetchModelsRequest,
    responses((status = 200, body = Object)))]
pub async fn fetch_models(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<FetchModelsRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_llm(&principal)?;
    let base_url = req.base_url.trim().to_string();
    if base_url.is_empty() {
        return Err(ApiError::BadRequest("base_url 不能为空".into()));
    }
    // api_key 场景（新建表单）优先；否则 provider_id → 库存密钥解密
    let api_key = match &req.api_key {
        Some(k) if !k.trim().is_empty() => k.clone(),
        _ => {
            let pid = req.provider_id.ok_or_else(|| {
                ApiError::BadRequest(
                    "api_key 与 provider_id 至少传一个（新建场景传明文 key；编辑场景传 provider_id 用库存密钥）".into(),
                )
            })?;
            let cipher = cipher_from(&state)?;
            let enc = engram_storage::repo::llm::get_provider_encrypted(&state.pool, pid)
                .await?
                .ok_or_else(|| ApiError::NotFound(format!("provider {pid} 不存在")))?;
            cipher
                .decrypt(&enc)
                .map_err(|e| ApiError::BadRequest(format!("密钥解密失败: {e}")))?
        }
    };
    let ids = engram_llm::provider::fetch_model_ids(&base_url, &api_key)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(serde_json::json!({ "models": ids })))
}

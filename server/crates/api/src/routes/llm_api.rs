//! LLM provider / 路由 / 用量 / API key 端点（全部仅管理员）。

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use agent_memory_llm::crypto::KeyCipher;
use agent_memory_llm::provider::{LlmProvider, OpenAiCompatProvider, ProviderRegistry};
use agent_memory_llm::router::{PurposeRouter, RoutingTable};
use agent_memory_llm::types::{ChatMessage, ChatRequest, ModelInfo, UsageRecord};

use crate::auth::{Principal, create_api_key};
use crate::error::ApiError;
use crate::state::AppState;

fn require_admin(principal: &Principal) -> Result<(), ApiError> {
    match principal {
        Principal::Admin => Ok(()),
        Principal::ApiKey { .. } => Err(ApiError::Forbidden("该端点仅管理员".into())),
    }
}

fn cipher_from(state: &AppState) -> Result<KeyCipher, ApiError> {
    let hex_master = state
        .master_key
        .as_ref()
        .map(|m| m.0.clone())
        .ok_or_else(|| {
            ApiError::Unavailable("服务端未配置主密钥（AGENT_MEMORY_MASTER_KEY）".into())
        })?;
    KeyCipher::from_hex_master(&hex_master).map_err(|e| ApiError::Unavailable(e.to_string()))
}

// ---------- providers ----------

#[derive(Deserialize, ToSchema)]
pub struct CreateProviderRequest {
    pub name: String,
    pub base_url: String,
    /// 明文 API key（只在请求中出现，落库前加密）
    pub api_key: String,
    #[serde(default)]
    pub models: Vec<ModelInfo>,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Serialize, ToSchema)]
pub struct ProviderDto {
    pub id: Uuid,
    pub name: String,
    pub base_url: String,
    pub models: Vec<ModelInfo>,
    pub is_default: bool,
}

/// 注册 LLM provider（key 加密落库）。
#[utoipa::path(post, path = "/settings/llm/providers",
    request_body = CreateProviderRequest,
    responses((status = 201, body = ProviderDto)))]
pub async fn create_provider(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<CreateProviderRequest>,
) -> Result<(StatusCode, Json<ProviderDto>), ApiError> {
    require_admin(&principal)?;
    let cipher = cipher_from(&state)?;

    let enc = cipher
        .encrypt(&req.api_key)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, models, is_default)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(&req.name)
    .bind(&req.base_url)
    .bind(&enc)
    .bind(sqlx::types::Json(&req.models))
    .bind(req.is_default)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(ProviderDto {
            id,
            name: req.name,
            base_url: req.base_url,
            models: req.models,
            is_default: req.is_default,
        }),
    ))
}

/// provider 列表（永不含密钥）。
#[utoipa::path(get, path = "/settings/llm/providers", responses((status = 200, body = [ProviderDto])))]
pub async fn list_providers(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ProviderDto>>, ApiError> {
    require_admin(&principal)?;
    let rows: Vec<(
        Uuid,
        String,
        String,
        sqlx::types::Json<Vec<ModelInfo>>,
        bool,
    )> = sqlx::query_as(
        "SELECT id, name, base_url, models, is_default FROM llm_providers ORDER BY created_at",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, base_url, models, is_default)| ProviderDto {
                id,
                name,
                base_url,
                models: models.0,
                is_default,
            })
            .collect(),
    ))
}

#[derive(Serialize, ToSchema)]
pub struct TestResult {
    pub ok: bool,
    pub message: String,
}

/// 连通性测试：发 1-token chat 探测。
#[utoipa::path(post, path = "/settings/llm/providers/{id}/test",
    responses((status = 200, body = TestResult)))]
pub async fn test_provider(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<TestResult>, ApiError> {
    require_admin(&principal)?;
    let cipher = cipher_from(&state)?;
    let registry = ProviderRegistry::new(state.pool.clone(), cipher.clone());

    let row: Option<(String, String, Vec<u8>, sqlx::types::Json<Vec<ModelInfo>>)> = sqlx::query_as(
        "SELECT name, base_url, api_key_encrypted, models FROM llm_providers WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((name, base_url, enc, models)) = row else {
        return Err(ApiError::NotFound(format!("provider {id} 不存在")));
    };

    let api_key = registry.get(&name).await.map(|_| ()).err(); // get() 内部已解密；这里只用其错误通道做存在性检查

    let _ = api_key;
    let provider = OpenAiCompatProvider::new(
        name.clone(),
        base_url,
        cipher
            .decrypt(&enc)
            .map_err(|e| ApiError::BadRequest(e.to_string()))?,
    );
    let chat_model = models
        .0
        .iter()
        .find(|m| m.capabilities.iter().any(|c| c == "chat"))
        .map(|m| m.id.clone())
        .unwrap_or_else(|| models.0.first().map(|m| m.id.clone()).unwrap_or_default());
    let embed_model = models
        .0
        .iter()
        .find(|m| m.capabilities.iter().any(|c| c == "embedding"))
        .map(|m| m.id.clone());

    // chat 探测（有 chat 模型时）
    let mut chat_result: Option<Result<_, _>> = None;
    if !chat_model.is_empty() {
        chat_result = Some(
            provider
                .chat(ChatRequest {
                    model: chat_model,
                    messages: vec![ChatMessage::user("ping")],
                    temperature: None,
                    json_mode: false,
                    max_tokens: Some(1),
                })
                .await,
        );
    }

    // embedding 探测（有 embedding 模型时；1 条短语）
    let mut embed_result: Option<Result<_, _>> = None;
    if let Some(em) = embed_model {
        embed_result = Some(
            provider
                .embed(agent_memory_llm::types::EmbedRequest {
                    model: em,
                    inputs: vec!["连通探测".into()],
                    dimensions: Some(1024),
                })
                .await,
        );
    }

    let (ok, message) = match (&chat_result, &embed_result) {
        (None, None) => (false, "该 provider 未配置任何模型".into()),
        (Some(Err(e)), _) | (_, Some(Err(e))) => (false, e.to_string()),
        (c, e) => {
            let mut parts = Vec::new();
            if let Some(Ok(r)) = c {
                registry
                    .record_usage(
                        &name,
                        &r.model,
                        "test",
                        r.input_tokens,
                        r.output_tokens,
                        r.latency_ms,
                        None,
                    )
                    .await;
                parts.push(format!("chat {}ms", r.latency_ms));
            }
            if let Some(Ok(r)) = e {
                registry
                    .record_usage(
                        &name,
                        &r.model,
                        "test",
                        r.input_tokens,
                        0,
                        r.latency_ms,
                        None,
                    )
                    .await;
                parts.push(format!(
                    "embed {}ms×{}维",
                    r.latency_ms,
                    r.embeddings.first().map(|v| v.len()).unwrap_or(0)
                ));
            }
            (true, parts.join("; "))
        }
    };
    Ok(Json(TestResult { ok, message }))
}

// ---------- routing ----------

/// 读取路由表。
#[utoipa::path(get, path = "/settings/llm/routing", responses((status = 200, body = RoutingTable)))]
pub async fn get_routing(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<RoutingTable>, ApiError> {
    require_admin(&principal)?;
    Ok(Json(PurposeRouter::new(state.pool).table().await?))
}

/// 保存路由表。
#[utoipa::path(put, path = "/settings/llm/routing",
    request_body = RoutingTable,
    responses((status = 204)))]
pub async fn put_routing(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(table): Json<RoutingTable>,
) -> Result<StatusCode, ApiError> {
    require_admin(&principal)?;
    PurposeRouter::new(state.pool).save(&table).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- usage ----------

/// 用量记录（近 500 条）。
#[utoipa::path(get, path = "/llm/usage", responses((status = 200, body = [UsageRecord])))]
pub async fn usage(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<UsageRecord>>, ApiError> {
    require_admin(&principal)?;
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

// ---------- api keys ----------

#[derive(Deserialize, ToSchema)]
pub struct CreateApiKeyRequest {
    pub name: String,
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
}
fn default_scopes() -> Vec<String> {
    crate::auth::SCOPES.iter().map(|s| s.to_string()).collect()
}

#[derive(Serialize, ToSchema)]
pub struct ApiKeyCreated {
    pub id: Uuid,
    pub name: String,
    /// 明文 key（amk_ 前缀；只在创建响应出现一次）
    pub key: String,
}

/// 签发 API key。
#[utoipa::path(post, path = "/settings/api-keys",
    request_body = CreateApiKeyRequest,
    responses((status = 201, body = ApiKeyCreated)))]
pub async fn create_api_key_handler(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<ApiKeyCreated>), ApiError> {
    require_admin(&principal)?;
    let (id, key) = create_api_key(&state.pool, &req.name, req.scopes).await?;
    Ok((
        StatusCode::CREATED,
        Json(ApiKeyCreated {
            id,
            name: req.name,
            key,
        }),
    ))
}

#[derive(Serialize, ToSchema)]
pub struct ApiKeyDto {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// API key 列表（永不含完整 key）。
#[utoipa::path(get, path = "/settings/api-keys", responses((status = 200, body = [ApiKeyDto])))]
pub async fn list_api_keys(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ApiKeyDto>>, ApiError> {
    require_admin(&principal)?;
    let rows: Vec<(Uuid, String, String, sqlx::types::Json<Vec<String>>, chrono::DateTime<chrono::Utc>, Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT id, name, key_prefix, scopes, created_at, last_used_at, revoked_at FROM api_keys ORDER BY created_at DESC",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(
                |(id, name, key_prefix, scopes, created_at, last_used_at, revoked_at)| ApiKeyDto {
                    id,
                    name,
                    key_prefix,
                    scopes: scopes.0,
                    created_at,
                    last_used_at,
                    revoked_at,
                },
            )
            .collect(),
    ))
}

/// 吊销 API key。
#[utoipa::path(post, path = "/settings/api-keys/{id}/revoke", responses((status = 204)))]
pub async fn revoke_api_key(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_admin(&principal)?;
    let result =
        sqlx::query("UPDATE api_keys SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL")
            .bind(id)
            .execute(&state.pool)
            .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("API key {id} 不存在或已吊销")));
    }
    Ok(StatusCode::NO_CONTENT)
}

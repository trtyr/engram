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
    /// L10：占位主密钥生效时的告示（不阻断；换真实密钥后需 re-encrypt 迁移）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// L1：UNIQUE 冲突判定（23505）——provider 重名是用户输入错误，应 400 而非 503。
fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(db) if db.code().as_deref() == Some("23505"))
}

/// 注册 LLM provider（key 加密落库）。L1：写入口全量校验；L3：默认唯一性事务降级。
#[utoipa::path(post, path = "/settings/llm/providers",
    request_body = CreateProviderRequest,
    responses((status = 201, body = ProviderDto)))]
pub async fn create_provider(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<CreateProviderRequest>,
) -> Result<(StatusCode, Json<ProviderDto>), ApiError> {
    require_admin(&principal)?;

    // L1：校验（违规 400 带明细——不再让配置错误延迟到运行时爆发）
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name 不能为空".into()));
    }
    let scheme_ok = req.base_url.starts_with("http://") || req.base_url.starts_with("https://");
    let host_part = req
        .base_url
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    if !scheme_ok
        || host_part.is_empty()
        || host_part.contains(char::is_whitespace)
        || host_part.starts_with('/')
    {
        return Err(ApiError::BadRequest(format!(
            "base_url 必须是 http(s):// 开头且含主机名（收到「{}」）",
            req.base_url
        )));
    }
    if req.api_key.trim().is_empty() {
        return Err(ApiError::BadRequest("api_key 不能为空".into()));
    }
    for m in &req.models {
        if m.id.trim().is_empty() {
            return Err(ApiError::BadRequest("models[].id 不能为空".into()));
        }
        let bad: Vec<&str> = m
            .capabilities
            .iter()
            .map(|s| s.as_str())
            .filter(|c| !matches!(*c, "chat" | "embedding"))
            .collect();
        if !bad.is_empty() {
            return Err(ApiError::BadRequest(format!(
                "模型「{}」的 capabilities 仅接受 chat / embedding（非法值：{}）",
                m.id,
                bad.join("、")
            )));
        }
    }

    let cipher = cipher_from(&state)?;

    let enc = cipher
        .encrypt(&req.api_key)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let id = Uuid::now_v7();

    // L3：默认唯一性——is_default 时同事务降级存量默认；
    // L1：UNIQUE 冲突（重名）映射 400（旧路径经 From<sqlx::Error> 变 503 retryable）
    let mut tx = state.pool.begin().await?;
    if req.is_default {
        sqlx::query("UPDATE llm_providers SET is_default = false WHERE is_default")
            .execute(&mut *tx)
            .await?;
    }
    let insert = sqlx::query(
        "INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, models, is_default)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(req.name.trim())
    .bind(&req.base_url)
    .bind(&enc)
    .bind(sqlx::types::Json(&req.models))
    .bind(req.is_default)
    .execute(&mut *tx)
    .await;
    if let Err(e) = insert {
        if is_unique_violation(&e) {
            return Err(ApiError::BadRequest(format!(
                "provider 名「{}」已存在",
                req.name.trim()
            )));
        }
        return Err(e.into());
    }
    tx.commit().await?;

    // L10：占位主密钥生效 → 响应带告示（不阻断但可感知；换真实密钥后需 re-encrypt）
    let warning = state.is_placeholder_master_key().then(|| {
        "当前使用占位主密钥（未设置 AGENT_MEMORY_MASTER_KEY）：此密钥加密的 API key 在换用真实主密钥后将无法解密。请尽早设置环境变量，并通过 POST /settings/llm/providers/re-encrypt 迁移".to_string()
    });

    Ok((
        StatusCode::CREATED,
        Json(ProviderDto {
            id,
            name: req.name.trim().to_string(),
            base_url: req.base_url,
            models: req.models,
            is_default: req.is_default,
            warning,
        }),
    ))
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateProviderRequest {
    /// 新 base_url（可选）
    pub base_url: Option<String>,
    /// 新明文 key（可选；提供则重新加密）
    pub api_key: Option<String>,
    /// 新模型列表（可选）
    pub models: Option<Vec<ModelInfo>>,
    /// 默认切换（可选；true 时事务降级存量默认）
    pub is_default: Option<bool>,
}

/// L2：更新 provider（name 不可改；key 提供则重新加密；is_default 切换保持唯一性）。
#[utoipa::path(put, path = "/settings/llm/providers/{id}",
    request_body = UpdateProviderRequest,
    responses((status = 200, body = ProviderDto)))]
pub async fn update_provider(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProviderRequest>,
) -> Result<Json<ProviderDto>, ApiError> {
    require_admin(&principal)?;

    // 校验提供的字段（与 create 同规则）
    if let Some(u) = &req.base_url {
        let scheme_ok = u.starts_with("http://") || u.starts_with("https://");
        let host_part = u
            .trim_start_matches("http://")
            .trim_start_matches("https://");
        if !scheme_ok || host_part.is_empty() || host_part.contains(char::is_whitespace) {
            return Err(ApiError::BadRequest(format!(
                "base_url 非法（收到「{u}」）"
            )));
        }
    }
    if let Some(k) = &req.api_key
        && k.trim().is_empty()
    {
        return Err(ApiError::BadRequest("api_key 不能为空".into()));
    }
    if let Some(models) = &req.models {
        for m in models {
            if m.id.trim().is_empty() {
                return Err(ApiError::BadRequest("models[].id 不能为空".into()));
            }
            if m.capabilities
                .iter()
                .any(|c| !matches!(c.as_str(), "chat" | "embedding"))
            {
                return Err(ApiError::BadRequest(format!(
                    "模型「{}」的 capabilities 仅接受 chat / embedding",
                    m.id
                )));
            }
        }
    }

    let cipher = cipher_from(&state)?;
    let enc_new = match &req.api_key {
        Some(k) => Some(
            cipher
                .encrypt(k)
                .map_err(|e| ApiError::BadRequest(e.to_string()))?,
        ),
        None => None,
    };

    let mut tx = state.pool.begin().await?;
    if req.is_default == Some(true) {
        sqlx::query("UPDATE llm_providers SET is_default = false WHERE is_default AND id <> $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    // COALESCE 逐字段更新；未提供的字段保持原值
    let row = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            sqlx::types::Json<Vec<ModelInfo>>,
            bool,
        ),
    >(
        "UPDATE llm_providers SET \
            base_url = COALESCE($2, base_url), \
            api_key_encrypted = COALESCE($3, api_key_encrypted), \
            models = COALESCE($4, models), \
            is_default = COALESCE($5, is_default), \
            updated_at = now() \
         WHERE id = $1 \
         RETURNING id, name, base_url, models, is_default",
    )
    .bind(id)
    .bind(&req.base_url)
    .bind(&enc_new)
    .bind(req.models.as_ref().map(sqlx::types::Json))
    .bind(req.is_default)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| {
        if is_unique_violation(&e) {
            ApiError::BadRequest("provider 名冲突".into())
        } else {
            e.into()
        }
    })?;
    tx.commit().await?;

    let Some((id, name, base_url, models, is_default)) = row else {
        return Err(ApiError::NotFound(format!("provider {id} 不存在")));
    };
    let warning = if req.api_key.is_some() && state.is_placeholder_master_key() {
        Some("本次更新的 API key 以占位主密钥加密——换用真实主密钥后需 re-encrypt 迁移".to_string())
    } else {
        None
    };
    Ok(Json(ProviderDto {
        id,
        name,
        base_url,
        models: models.0,
        is_default,
        warning,
    }))
}

/// L2：删除 provider（默认拒删；routing 引用拒删带明细）。
#[utoipa::path(delete, path = "/settings/llm/providers/{id}", responses((status = 204)))]
pub async fn delete_provider(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_admin(&principal)?;

    let row: Option<(String, bool)> =
        sqlx::query_as("SELECT name, is_default FROM llm_providers WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((name, is_default)) = row else {
        return Err(ApiError::NotFound(format!("provider {id} 不存在")));
    };
    if is_default {
        return Err(ApiError::BadRequest(
            "默认 provider 不可删除：请先将其他 provider 设为默认（PUT is_default=true）".into(),
        ));
    }
    // routing 表引用检查（L4 校验保证新写入不引用幽灵；存量引用在此拦截）
    let router = PurposeRouter::new(state.pool.clone());
    let table = router.table().await?;
    let referencing: Vec<String> = table
        .routes
        .iter()
        .filter(|(_, chain)| chain.iter().any(|r| r.provider == name))
        .map(|(p, _)| p.clone())
        .collect();
    if !referencing.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "路由表仍引用「{name}」的 purpose：{}——请先更新 /settings/llm/routing",
            referencing.join("、")
        )));
    }

    sqlx::query("DELETE FROM llm_providers WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct ReEncryptRequest {
    /// 轮换前的旧主密钥（64 hex；当前密钥来自 env，新密钥下加密）
    pub old_master_key: String,
}

#[derive(Serialize, ToSchema)]
pub struct ReEncryptResult {
    pub re_encrypted: usize,
}

/// L2：master key 轮换——全部 provider 密钥用旧密钥解密、当前密钥重新加密（单事务）。
/// 场景：先改 AGENT_MEMORY_MASTER_KEY 重启，再带旧密钥调用本端点完成迁移。
#[utoipa::path(post, path = "/settings/llm/providers/re-encrypt",
    request_body = ReEncryptRequest,
    responses((status = 200, body = ReEncryptResult)))]
pub async fn reencrypt_providers(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<ReEncryptRequest>,
) -> Result<Json<ReEncryptResult>, ApiError> {
    require_admin(&principal)?;

    let old_cipher = KeyCipher::from_hex_master(&req.old_master_key)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let new_cipher = cipher_from(&state)?;

    let rows: Vec<(Uuid, Vec<u8>)> =
        sqlx::query_as("SELECT id, api_key_encrypted FROM llm_providers ORDER BY created_at")
            .fetch_all(&state.pool)
            .await?;

    // 先全部解密成功再落库（任一失败 = 旧密钥不对，整体 400 不做半截迁移）
    let mut reencoded: Vec<(Uuid, Vec<u8>)> = Vec::with_capacity(rows.len());
    for (id, enc) in &rows {
        let plain = old_cipher.decrypt(enc).map_err(|e| {
            ApiError::BadRequest(format!(
                "旧主密钥无法解密 provider {id}（{}）——确认 old_master_key 正确后重试",
                e
            ))
        })?;
        let new_enc = new_cipher
            .encrypt(&plain)
            .map_err(|e| ApiError::BadRequest(e.to_string()))?;
        reencoded.push((*id, new_enc));
    }

    let mut tx = state.pool.begin().await?;
    for (id, enc) in &reencoded {
        sqlx::query(
            "UPDATE llm_providers SET api_key_encrypted = $2, updated_at = now() WHERE id = $1",
        )
        .bind(id)
        .bind(enc)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    Ok(Json(ReEncryptResult {
        re_encrypted: reencoded.len(),
    }))
}

/// provider 列表（永不含密钥）。
#[utoipa::path(get, path = "/settings/llm/providers", responses((status = 200, body = [ProviderDto])))]
pub async fn list_providers(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ProviderDto>>, ApiError> {
    require_admin(&principal)?;
    type ProvRow = (
        Uuid,
        String,
        String,
        sqlx::types::Json<Vec<ModelInfo>>,
        bool,
    );
    let rows: Vec<ProvRow> = sqlx::query_as(
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
                warning: None,
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

    type Row = (String, String, Vec<u8>, sqlx::types::Json<Vec<ModelInfo>>);
    let row: Option<Row> = sqlx::query_as(
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
                    .record_usage(&agent_memory_llm::types::UsageMeta {
                        provider: name.clone(),
                        model: r.model.clone(),
                        purpose: "test".into(),
                        input_tokens: r.input_tokens,
                        output_tokens: r.output_tokens,
                        latency_ms: r.latency_ms,
                        job_id: None,
                    })
                    .await;
                parts.push(format!("chat {}ms", r.latency_ms));
            }
            if let Some(Ok(r)) = e {
                registry
                    .record_usage(&agent_memory_llm::types::UsageMeta {
                        provider: name.clone(),
                        model: r.model.clone(),
                        purpose: "test".into(),
                        input_tokens: r.input_tokens,
                        output_tokens: 0,
                        latency_ms: r.latency_ms,
                        job_id: None,
                    })
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

/// 保存路由表。L4：落库前全量校验（purpose 枚举 / provider 存在 / model 在册）。
#[utoipa::path(put, path = "/settings/llm/routing",
    request_body = RoutingTable,
    responses((status = 204)))]
pub async fn put_routing(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(table): Json<RoutingTable>,
) -> Result<StatusCode, ApiError> {
    require_admin(&principal)?;

    const VALID_PURPOSES: [&str; 8] = [
        "extract",
        "arbitrate",
        "embed",
        "organize",
        "consolidate",
        "wiki_analysis",
        "persona",
        "wiki_generation",
    ];

    // 一次取全部 provider（name → models）
    let providers: Vec<(String, sqlx::types::Json<Vec<ModelInfo>>)> =
        sqlx::query_as("SELECT name, models FROM llm_providers")
            .fetch_all(&state.pool)
            .await?;
    let provider_models: std::collections::HashMap<String, Vec<String>> = providers
        .into_iter()
        .map(|(n, m)| (n, m.0.into_iter().map(|x| x.id).collect()))
        .collect();

    // L4：逐条校验，违规收集明细一次性返回（typo purpose/幽灵 provider/不在册 model
    // 不再静默入库——旧路径落库后 resolve 静默跳过，用户以为在用路由实际全走默认）
    let mut errors: Vec<String> = Vec::new();
    for (purpose, chain) in &table.routes {
        if !VALID_PURPOSES.contains(&purpose.as_str()) {
            errors.push(format!(
                "未知 purpose「{purpose}」（合法值：{}）",
                VALID_PURPOSES.join(" / ")
            ));
            continue;
        }
        for (i, rule) in chain.iter().enumerate() {
            match provider_models.get(&rule.provider) {
                None => errors.push(format!(
                    "{purpose} 第{}条：provider「{}」不存在",
                    i + 1,
                    rule.provider
                )),
                Some(models) => {
                    if !models.contains(&rule.model) {
                        errors.push(format!(
                            "{purpose} 第{}条：模型「{}」不在 provider「{}」的 models 列表中（{}）",
                            i + 1,
                            rule.model,
                            rule.provider,
                            models.join("、")
                        ));
                    }
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "路由表校验失败（{} 处）：{}",
            errors.len(),
            errors.join("；")
        )));
    }

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
    type KeyRow = (
        Uuid,
        String,
        String,
        sqlx::types::Json<Vec<String>>,
        chrono::DateTime<chrono::Utc>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    );
    let rows: Vec<KeyRow> = sqlx::query_as(
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

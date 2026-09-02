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
use agent_memory_llm::types::{ChatMessage, ChatRequest, Purpose, UsageRecord};

use crate::auth::{Principal, create_api_key};
use crate::error::ApiError;
use crate::state::AppState;

fn require_admin(principal: &Principal) -> Result<(), ApiError> {
    match principal {
        Principal::Admin => Ok(()),
        Principal::ApiKey { .. } => Err(ApiError::Forbidden("该端点仅管理员".into())),
    }
}

/// LLM 配置面：管理员全权，或持 llm scope 的 API key（2026-08-31 方向：
/// 除 amk_ 管理外平台能力全部暴露给 AI——provider/路由/连通测试归 llm scope；
/// api-keys 管理与主密钥 re-encrypt 仍仅管理员）。
fn require_llm(principal: &Principal) -> Result<(), ApiError> {
    match principal {
        Principal::Admin => Ok(()),
        Principal::ApiKey { scopes, .. } if scopes.iter().any(|s| s == "llm") => Ok(()),
        Principal::ApiKey { .. } => Err(ApiError::Forbidden(
            "该端点需要管理员或 llm scope 的 API key".into(),
        )),
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
    /// 单一模型 id（一个供应商一个模型一个 key）
    pub model_id: String,
    /// 能力：chat | embedding（默认 chat）
    #[serde(default = "default_capability")]
    pub capability: String,
    #[serde(default)]
    pub is_default: bool,
}

fn default_capability() -> String {
    "chat".to_string()
}

#[derive(Serialize, ToSchema)]
pub struct ProviderDto {
    pub id: Uuid,
    pub name: String,
    pub base_url: String,
    pub model_id: String,
    pub capability: String,
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
    require_llm(&principal)?;

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
    if req.model_id.trim().is_empty() {
        return Err(ApiError::BadRequest("model_id 不能为空".into()));
    }
    if !matches!(req.capability.as_str(), "chat" | "embedding") {
        return Err(ApiError::BadRequest(format!(
            "capability 仅接受 chat / embedding（非法值：{}）",
            req.capability
        )));
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
        // 同 capability 的唯一默认（chat 与 embedding 各有一个默认）
        sqlx::query(
            "UPDATE llm_providers SET is_default = false WHERE is_default AND capability = $1",
        )
        .bind(&req.capability)
        .execute(&mut *tx)
        .await?;
    }
    let insert = sqlx::query(
        "INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, model_id, capability, is_default)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(req.name.trim())
    .bind(&req.base_url)
    .bind(&enc)
    .bind(req.model_id.trim())
    .bind(&req.capability)
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
            model_id: req.model_id.trim().to_string(),
            capability: req.capability,
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
    /// 新模型 id（可选）
    pub model_id: Option<String>,
    /// 新能力（可选）
    pub capability: Option<String>,
    /// 默认切换（可选；true 时事务降级同能力存量默认）
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
    require_llm(&principal)?;

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
    if let Some(mid) = &req.model_id
        && mid.trim().is_empty()
    {
        return Err(ApiError::BadRequest("model_id 不能为空".into()));
    }
    if let Some(cap) = &req.capability
        && !matches!(cap.as_str(), "chat" | "embedding")
    {
        return Err(ApiError::BadRequest(format!(
            "capability 仅接受 chat / embedding（非法值：{cap}）"
        )));
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
        // 同 capability 的唯一默认（capability 变更时用新值降级存量）
        let cur_cap: Option<String> =
            sqlx::query_scalar("SELECT capability FROM llm_providers WHERE id = $1")
                .bind(id)
                .fetch_optional(&state.pool)
                .await?;
        let new_cap = req
            .capability
            .clone()
            .or(cur_cap)
            .unwrap_or_else(|| "chat".to_string());
        sqlx::query(
            "UPDATE llm_providers SET is_default = false WHERE is_default AND capability = $2 AND id <> $1",
        )
        .bind(id)
        .bind(&new_cap)
        .execute(&mut *tx)
        .await?;
    }

    // COALESCE 逐字段更新；未提供的字段保持原值
    let row = sqlx::query_as::<_, (Uuid, String, String, String, String, bool)>(
        "UPDATE llm_providers SET \
            base_url = COALESCE($2, base_url), \
            api_key_encrypted = COALESCE($3, api_key_encrypted), \
            model_id = COALESCE($4, model_id), \
            capability = COALESCE($5, capability), \
            is_default = COALESCE($6, is_default), \
            updated_at = now() \
         WHERE id = $1 \
         RETURNING id, name, base_url, model_id, capability, is_default",
    )
    .bind(id)
    .bind(&req.base_url)
    .bind(&enc_new)
    .bind(&req.model_id)
    .bind(&req.capability)
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

    let Some((id, name, base_url, model_id, capability, is_default)) = row else {
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
        model_id,
        capability,
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
    require_llm(&principal)?;

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
    require_llm(&principal)?;
    type ProvRow = (Uuid, String, String, String, String, bool);
    let rows: Vec<ProvRow> = sqlx::query_as(
        "SELECT id, name, base_url, model_id, capability, is_default FROM llm_providers ORDER BY created_at",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(
                |(id, name, base_url, model_id, capability, is_default)| ProviderDto {
                    id,
                    name,
                    base_url,
                    model_id,
                    capability,
                    is_default,
                    warning: None,
                },
            )
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
    require_llm(&principal)?;
    let cipher = cipher_from(&state)?;
    let registry = ProviderRegistry::new(state.pool.clone(), cipher.clone());

    type Row = (String, String, Vec<u8>, String, String);
    let row: Option<Row> = sqlx::query_as(
        "SELECT name, base_url, api_key_encrypted, model_id, capability FROM llm_providers WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((name, base_url, enc, model_id, capability)) = row else {
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
    // 一个 provider 一个模型：按 capability 只探测对应方向
    let chat_model = if capability == "chat" {
        model_id.clone()
    } else {
        String::new()
    };
    let embed_model = if capability == "embedding" {
        Some(model_id)
    } else {
        None
    };

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
    require_llm(&principal)?;
    Ok(Json(PurposeRouter::new(state.pool).table().await?))
}

#[derive(Deserialize, ToSchema)]
pub struct RoutingSuggestRequest {
    /// 指定用哪个 provider 生成建议（可选；默认用 chat 能力的默认 provider）
    pub provider: Option<String>,
}

/// AI 路由建议：读现有供应商 + 8 用途，调 LLM 生成建议路由表（不落库，返回给前端确认）。
#[utoipa::path(post, path = "/settings/llm/routing/suggest",
    request_body = RoutingSuggestRequest,
    responses((status = 200, body = RoutingTable)))]
pub async fn suggest_routing(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<RoutingSuggestRequest>,
) -> Result<Json<RoutingTable>, ApiError> {
    require_llm(&principal)?;

    // 读现有供应商
    let providers: Vec<(String, String, String)> =
        sqlx::query_as("SELECT name, model_id, capability FROM llm_providers ORDER BY name")
            .fetch_all(&state.pool)
            .await?;
    if providers.is_empty() {
        return Err(ApiError::BadRequest(
            "没有可用的 LLM 供应商——请先在「供应商」注册至少一个 chat 供应商".into(),
        ));
    }
    if !providers.iter().any(|(_, _, c)| c == "chat") {
        return Err(ApiError::BadRequest(
            "没有 chat 能力的供应商——AI 建议需要至少一个 chat 供应商".into(),
        ));
    }

    let provider_desc = providers
        .iter()
        .map(|(n, m, c)| format!("- {n}（{c}，模型 {m}）"))
        .collect::<Vec<_>>()
        .join("\n");
    let system = "你是 AI 配置助手，为单用户 AI 长期记忆系统生成 LLM 路由表。\
        \n系统有 8 个用途：extract（抽取）、arbitrate（仲裁）、embed（嵌入）、organize（组织）、consolidate（整理）、wiki_analysis（Wiki 分析）、persona（画像）、wiki_generation（Wiki 生成）。\
        \n规则：1) embed 用途必须用 embedding 能力的供应商；2) 其余用途用 chat 能力的供应商；3) 高频低成本的用途（extract / arbitrate / embed）优先便宜的模型，低频高价值的（persona / wiki_generation）优先强的模型；4) provider 与 model 必须来自下面给定的列表，不得臆造。";
    let user = format!(
        "可用的供应商：\n{provider_desc}\n\n请为 8 个用途生成路由建议，每个用途一条回退链（至少一条）。只输出严格 JSON，形如 {{\"extract\":[{{\"provider\":\"...\",\"model\":\"...\"}}],\"embed\":[...]}}。"
    );

    let cipher = cipher_from(&state)?;
    let registry = ProviderRegistry::new(state.pool.clone(), cipher);
    let (provider, model) = match &req.provider {
        Some(name) => registry
            .get(name)
            .await
            .map_err(|e| ApiError::BadRequest(e.to_string()))?,
        None => registry
            .resolve(Purpose::Extract)
            .await
            .map_err(|e| ApiError::Unavailable(e.to_string()))?,
    };
    let resp = provider
        .chat(ChatRequest {
            model,
            messages: vec![ChatMessage::system(system), ChatMessage::user(&user)],
            temperature: Some(0.2),
            json_mode: true,
            max_tokens: Some(2000),
        })
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;

    let content = resp.content.trim();
    let table: RoutingTable = serde_json::from_str(content)
        .map_err(|e| ApiError::BadRequest(format!("LLM 生成的路由建议无法解析（{e}）")))?;

    Ok(Json(table))
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
    require_llm(&principal)?;

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

    // 一次取全部 provider（name → model_id，一个 provider 一个模型）
    let providers: Vec<(String, String)> =
        sqlx::query_as("SELECT name, model_id FROM llm_providers")
            .fetch_all(&state.pool)
            .await?;
    let provider_models: std::collections::HashMap<String, String> =
        providers.into_iter().collect();

    // L4：逐条校验，违规收集明细一次性返回（typo purpose/幽灵 provider/model 与 provider 不一致
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
                Some(model_id) => {
                    if &rule.model != model_id {
                        errors.push(format!(
                            "{purpose} 第{}条：模型「{}」与 provider「{}」的 model_id（{}）不一致",
                            i + 1,
                            rule.model,
                            rule.provider,
                            model_id
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

#[derive(Deserialize, ToSchema)]
pub struct BatchRevokeRequest {
    pub ids: Vec<Uuid>,
}

#[derive(Serialize, ToSchema)]
pub struct BatchRevokeResult {
    pub revoked: usize,
}

/// 批量吊销 API key（幂等：已吊销的忽略，返回实际吊销数）。
#[utoipa::path(post, path = "/settings/api-keys/batch-revoke",
    request_body = BatchRevokeRequest,
    responses((status = 200, body = BatchRevokeResult)))]
pub async fn batch_revoke_api_keys(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<BatchRevokeRequest>,
) -> Result<Json<BatchRevokeResult>, ApiError> {
    require_admin(&principal)?;
    if req.ids.is_empty() {
        return Err(ApiError::BadRequest("ids 不能为空".into()));
    }
    let result = sqlx::query(
        "UPDATE api_keys SET revoked_at = now() WHERE id = ANY($1) AND revoked_at IS NULL",
    )
    .bind(&req.ids)
    .execute(&state.pool)
    .await?;
    Ok(Json(BatchRevokeResult {
        revoked: result.rows_affected() as usize,
    }))
}

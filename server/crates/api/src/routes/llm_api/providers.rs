//! `llm_api` 的实现切片（架构治理 2026-09-21：自 llm_api.rs 纯搬移，零行为变化）。

use super::*;

/// LLM 配置面：管理员全权，或持 llm scope 的 API key（2026-08-31 方向：
/// 除 amk_ 管理外平台能力全部暴露给 AI——provider/路由/连通测试归 llm scope；
/// api-keys 管理与主密钥 re-encrypt 仍仅管理员）。
pub(crate) fn require_llm(principal: &Principal) -> Result<(), ApiError> {
    match principal {
        Principal::Admin => Ok(()),
        Principal::ApiKey { scopes, .. } if scopes.iter().any(|s| s == "llm") => Ok(()),
        Principal::ApiKey { .. } => Err(ApiError::Forbidden(
            "该端点需要管理员或 llm scope 的 API key".into(),
        )),
    }
}

pub(crate) fn cipher_from(state: &AppState) -> Result<KeyCipher, ApiError> {
    let hex_master = state
        .master_key
        .as_ref()
        .map(|m| m.0.clone())
        .ok_or_else(|| {
            ApiError::Unavailable("服务端未配置主密钥（AGENT_MEMORY_MASTER_KEY）".into())
        })?;
    KeyCipher::from_hex_master(&hex_master).map_err(|e| ApiError::Unavailable(e.to_string()))
}

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

pub(crate) fn default_capability() -> String {
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

    // L1：校验（违规 400 带明细——不再让配置错误延迟到运行时爆发）。
    // base_url 先 trim：粘贴尾随空格是高频输入失误，自动纠正而非拒绝。
    let base_url = req.base_url.trim().to_string();
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name 不能为空".into()));
    }
    let scheme_ok = base_url.starts_with("http://") || base_url.starts_with("https://");
    let host_part = base_url
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    if !scheme_ok
        || host_part.is_empty()
        || host_part.contains(char::is_whitespace)
        || host_part.starts_with('/')
    {
        return Err(ApiError::BadRequest(format!(
            "base_url 必须是 http(s):// 开头且含主机名（收到「{}」）",
            base_url
        )));
    }
    if req.api_key.trim().is_empty() {
        return Err(ApiError::BadRequest("api_key 不能为空".into()));
    }
    // SEC-C（2026-09-03）：最小长度校验——挡手滑的占位串；真实性由 provider-test 连通探针判定
    if req.api_key.trim().len() < 8 {
        return Err(ApiError::BadRequest(
            "api_key 看起来太短（<8 字符）——请填真实 key，连通性可用 provider-test 验证".into(),
        ));
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
    // L1：UNIQUE 冲突（重名）映射 400（仓储层以 Conflict 标注）
    if let Err(e) = repo::insert_provider_tx(
        &state.pool,
        id,
        req.name.trim(),
        &base_url,
        &enc,
        req.model_id.trim(),
        &req.capability,
        req.is_default,
    )
    .await
    {
        return Err(match e {
            StoreError::Conflict(_) => {
                ApiError::BadRequest(format!("provider 名「{}」已存在", req.name.trim()))
            }
            other => other.into(),
        });
    }

    // L10：占位主密钥生效 → 响应带告示（不阻断但可感知；换真实密钥后需 re-encrypt）
    let warning = state.is_placeholder_master_key().then(|| {
        "当前使用占位主密钥（未设置 AGENT_MEMORY_MASTER_KEY）：此密钥加密的 API key 在换用真实主密钥后将无法解密。请尽早设置环境变量，并通过 POST /settings/llm/providers/re-encrypt 迁移".to_string()
    });

    Ok((
        StatusCode::CREATED,
        Json(ProviderDto {
            id,
            name: req.name.trim().to_string(),
            base_url,
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

    // 校验提供的字段（与 create 同规则；trim 粘贴空白）
    let base_url = req.base_url.as_ref().map(|u| u.trim().to_string());
    if let Some(u) = base_url.as_deref() {
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

    // COALESCE 逐字段更新；未提供的字段保持原值（默认唯一性事务在仓储内）
    let row = repo::update_provider_tx(
        &state.pool,
        id,
        base_url.as_deref(),
        enc_new.as_deref(),
        req.model_id.as_deref(),
        req.capability.as_deref(),
        req.is_default,
    )
    .await
    .map_err(|e| match e {
        StoreError::Conflict(_) => ApiError::BadRequest("provider 名冲突".into()),
        other => other.into(),
    })?;

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

    let Some((name, is_default)) = repo::get_provider_name_default(&state.pool, id).await? else {
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

    repo::delete_provider(&state.pool, id).await?;
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

    let rows = repo::all_provider_keys(&state.pool).await?;

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

    repo::update_provider_keys_tx(&state.pool, &reencoded).await?;

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
    let rows = repo::list_providers(&state.pool).await?;
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

    let Some((name, base_url, enc, model_id, capability)) =
        repo::get_provider_full(&state.pool, id).await?
    else {
        return Err(ApiError::NotFound(format!("provider {id} 不存在")));
    };

    // get() 内部已解密；这里只借用它的错误通道做存在性探针。原实现把探针结果直接丢弃
    // （错误静默吞掉、探针形同虚设）——改为显式告警，控制流保持不变。
    if let Err(e) = registry.get(&name).await {
        tracing::warn!(provider = %name, error = %e, "provider 无法读取（存在性探针失败）");
    }
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
                .embed(engram_llm::types::EmbedRequest {
                    model: em,
                    inputs: vec!["连通探测".into()],
                    dimensions: Some(engram_distill::llm_port::embedding_dimensions()),
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
                    .record_usage(&engram_llm::types::UsageMeta {
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
                    .record_usage(&engram_llm::types::UsageMeta {
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

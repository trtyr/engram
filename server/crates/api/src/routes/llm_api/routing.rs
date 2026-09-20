//! `llm_api` 的实现切片（架构治理 2026-09-21：自 llm_api.rs 纯搬移，零行为变化）。

use super::*;

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
    let providers = repo::list_provider_name_model_cap(&state.pool).await?;
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
        \n系统有 9 个用途：extract（抽取）、arbitrate（仲裁）、embed（嵌入）、organize（组织）、consolidate（整理）、wiki_analysis（Wiki 分析）、persona（画像）、wiki_generation（Wiki 生成）、wiki_lint（Wiki 语义检查）。\
        \n规则：1) embed 用途必须用 embedding 能力的供应商；2) 其余用途用 chat 能力的供应商；3) 高频低成本的用途（extract / arbitrate / embed）优先便宜的模型，低频高价值的（persona / wiki_generation）优先强的模型；4) provider 与 model 必须来自下面给定的列表，不得臆造。";
    let user = format!(
        "可用的供应商：\n{provider_desc}\n\n请为 9 个用途生成路由建议，每个用途一条回退链（至少一条）。只输出严格 JSON，形如 {{\"extract\":[{{\"provider\":\"...\",\"model\":\"...\"}}],\"embed\":[...]}}。"
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

    let table: RoutingTable = parse_llm_json(&resp.content).map_err(ApiError::BadRequest)?;

    Ok(Json(table))
}

/// LLM 返回体解析（SEC-A 修复）：容忍三种现实形态——纯 JSON / markdown fence 包裹
/// （```json … ```）/ 前置说明文字 + JSON。失败时报返回片段（可诊断），不裸 serde 错误。
pub(crate) fn parse_llm_json<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, String> {
    let mut s = raw.trim();
    // 剥 markdown fence：```json\n…\n``` 或 ```\n…\n```
    if let Some(rest) = s.strip_prefix("```") {
        let body = rest.split_once('\n').map(|(_, r)| r).unwrap_or(rest);
        s = body.trim().trim_end_matches("```").trim();
    }
    if let Ok(v) = serde_json::from_str(s) {
        return Ok(v);
    }
    // 兜底：截取首个 '{' 到最后一个 '}'（容忍「以下是建议：{…}」之类包裹文字）
    if let (Some(a), Some(b)) = (s.find('{'), s.rfind('}'))
        && a < b
        && let Ok(v) = serde_json::from_str(&s[a..=b])
    {
        return Ok(v);
    }
    let head: String = s.chars().take(200).collect();
    Err(format!(
        "LLM 返回内容不是合法 JSON（返回片段：{head}）——请重试或换 chat 供应商"
    ))
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

    const VALID_PURPOSES: [&str; 9] = [
        "extract",
        "arbitrate",
        "embed",
        "organize",
        "consolidate",
        "wiki_analysis",
        "persona",
        "wiki_generation",
        "wiki_lint",
    ];

    // 一次取全部 provider（name → model_id，一个 provider 一个模型）
    let provider_models: std::collections::HashMap<String, String> =
        repo::list_provider_models(&state.pool)
            .await?
            .into_iter()
            .collect();

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

//! `memory_api` 的实现切片（架构治理 2026-09-21：自 memory_api.rs 纯搬移；P015 蒸馏手动触发统一走 /memory/maintain）。

use super::*;

#[derive(Deserialize, IntoParams)]
pub struct ListAtomsParams {
    pub kind: Option<String>,
    pub status: Option<String>,
    pub cursor: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<i64>,
}

#[utoipa::path(get, path = "/memory/atoms", params(ListAtomsParams),
    responses((status = 200, body = [AtomDto])))]
pub async fn list_atoms(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ListAtomsParams>,
) -> Result<Json<Vec<AtomDto>>, ApiError> {
    require_memory_read(&principal)?;
    Ok(Json(
        svc(&state)
            .list_atoms(
                p.kind.as_deref(),
                p.status.as_deref(),
                p.cursor,
                p.limit.unwrap_or(100),
            )
            .await
            .map_err(me)?,
    ))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CreateAtomRequest {
    pub kind: String,
    pub content: String,
    #[serde(default = "default_conf")]
    pub confidence: f32,
    /// 事件时间（ISO8601 或 date-only；"下周三"这类相对时间解析后的绝对值）
    #[serde(default, deserialize_with = "opt_flex_dt")]
    pub occurred_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 有效期（ISO8601；到期事件可过滤/降权）
    #[serde(default, deserialize_with = "opt_flex_dt")]
    pub valid_until: Option<chrono::DateTime<chrono::Utc>>,
    /// P3 隐私标记：默认不进检索与 context_pack（reveal 才可见）
    #[serde(default)]
    pub sensitive: bool,
    /// 断言强度：fact=用户明示/机器验证, inference=agent 推断, assumption=假设（缺省 fact）
    #[serde(default)]
    pub strength: Option<String>,
    /// 断言来源：user_stated/verified_probe/agent_inferred/doc（缺省 user_stated）
    #[serde(default)]
    pub source: Option<String>,
}

/// 手工新增原子（人审补充，直接 active）。
#[utoipa::path(post, path = "/memory/atoms",
    request_body = CreateAtomRequest,
    responses((status = 201, body = AtomDto)))]
pub async fn create_atom(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<CreateAtomRequest>,
) -> Result<(StatusCode, Json<AtomDto>), ApiError> {
    require_memory(&principal)?;
    // 权限收窄：AI 只写会话（原料），直写原子是蒸馏的活——收回直写加工权
    if !matches!(&*principal, Principal::Admin) {
        return Err(ApiError::Forbidden(
            "原子由会话蒸馏产生——AI 记录对话（session-write）即可，蒸馏自动抽取；人工直写断溯源且绕过判重".into(),
        ));
    }
    let a = svc(&state)
        .create_atom(
            &req.kind,
            &req.content,
            req.confidence,
            req.occurred_at,
            req.valid_until,
            req.sensitive,
            req.strength.as_deref(),
            req.source.as_deref(),
        )
        .await
        .map_err(me)?;
    Ok((StatusCode::CREATED, Json(a)))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateAtomRequest {
    pub content: Option<String>,
    /// 分面类型修正（仅用户会话；AI 禁改改写语义）
    pub kind: Option<String>,
    pub confidence: Option<f32>,
    /// 只允许 "archived" / "active"
    pub status: Option<String>,
    /// correction 取代链：本原子被哪条新原子取代（蒸馏链自动维护，手动 correction 补链）
    /// 三态：缺省=不动 / null=清空 / 值=设置（T004）
    pub superseded_by: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "opt_flex_dt3")]
    /// 三态：缺省=不动 / null=清空 / 值=设置（T004）
    pub occurred_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    #[serde(default, deserialize_with = "opt_flex_dt3")]
    /// 三态：缺省=不动 / null=清空 / 值=设置（T004）
    pub valid_until: Option<Option<chrono::DateTime<chrono::Utc>>>,
    /// P3 隐私标记切换
    /// 三态：缺省=不动 / null=清标记(false) / 值=设置（T004）
    pub sensitive: Option<Option<bool>>,
}

#[utoipa::path(patch, path = "/memory/atoms/{id}",
    request_body = UpdateAtomRequest,
    responses((status = 200, body = AtomDto)))]
pub async fn update_atom(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateAtomRequest>,
) -> Result<Json<AtomDto>, ApiError> {
    require_memory(&principal)?;
    // 编辑分权：改写语义（content/kind/confidence）仅限用户（Web 登录态）；
    // AI 走 correction：atom-add 新原子 + PATCH 旧原子 superseded_by/status。
    // sensitive/status/superseded_by/时间字段对 AI 开放（保护与追加语义）。
    let actor = actor_of(&principal);
    let rewriting = req.content.is_some() || req.kind.is_some() || req.confidence.is_some();
    if rewriting && !matches!(&*principal, Principal::Admin) {
        return Err(ApiError::Forbidden(
            "content/kind/confidence 编辑仅限用户（Web 登录态）；AI 纠错走 correction：写纠正对话（session-write），蒸馏自动取代".into(),
        ));
    }
    // 权限收窄：superseded_by 收回——correction 走会话，取代链由蒸馏自动维护
    if req.superseded_by.is_some() && !matches!(&*principal, Principal::Admin) {
        return Err(ApiError::Forbidden(
            "取代链由蒸馏自动维护——AI 写纠正对话（session-write）即可，correction 走会话".into(),
        ));
    }
    Ok(Json(
        svc(&state)
            .update_atom(
                id,
                req.content.as_deref(),
                req.kind.as_deref(),
                req.confidence,
                req.status.as_deref(),
                req.superseded_by,
                req.occurred_at,
                req.valid_until,
                req.sensitive,
                &actor,
            )
            .await
            .map_err(me)?,
    ))
}

/// 原子改写历史（新→旧；编辑留痕）。
#[utoipa::path(get, path = "/memory/atoms/{id}/revisions",
    responses((status = 200, body = [engram_core::AtomRevision])))]
pub async fn atom_revisions(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<engram_core::AtomRevision>>, ApiError> {
    require_memory_read(&principal)?;
    Ok(Json(svc(&state).atom_revisions(id).await.map_err(me)?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct SearchRequest {
    pub query: String,
    /// 层过滤：["l1","entities"]，空 = 全部（P015 场景层退役：l2/l3 已不存在）
    #[serde(default)]
    pub layers: Vec<String>,
    pub max_items: Option<i64>,
    /// true = 命中不回写热度（harness 注入/测试用，防 B6 污染）
    #[serde(default)]
    pub no_feedback: bool,
    /// true = 结果包含 sensitive 原子（隐私项默认排除；P3）
    #[serde(default)]
    pub reveal: bool,
    /// 时间范围过滤起点（occurred_at 优先，NULL fallback created_at；UTC）
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    /// 时间范围过滤终点
    pub to: Option<chrono::DateTime<chrono::Utc>>,
}

#[utoipa::path(post, path = "/memory/search", operation_id = "memory_search",
    request_body = SearchRequest,
    responses((status = 200, body = SearchResponse)))]
pub async fn search(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, ApiError> {
    require_memory(&principal)?;
    let layers: Vec<&str> = req.layers.iter().map(|s| s.as_str()).collect();
    Ok(Json(
        svc(&state)
            .search(
                &req.query,
                &layers,
                req.max_items.unwrap_or(20),
                req.no_feedback,
                req.from,
                req.to,
                None,
            )
            .await
            .map_err(me)?,
    ))
}

#[derive(Deserialize, IntoParams)]
pub struct ContextParams {
    /// 可选相关性查询；缺省按最近
    pub query: Option<String>,
    pub budget_items: Option<usize>,
    pub budget_chars: Option<usize>,
    /// true = 命中不回写热度（harness 注入/测试用）
    #[serde(default)]
    pub no_feedback: bool,
}

/// AI 冷启动首选：一站式上下文包（L3+L2+L1 按预算裁剪）。
#[utoipa::path(get, path = "/memory/context", params(ContextParams),
    responses((status = 200, body = ContextPack)))]
pub async fn context(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ContextParams>,
) -> Result<Json<ContextPack>, ApiError> {
    require_memory_read(&principal)?;
    Ok(Json(
        svc(&state)
            .context_pack(
                p.query.as_deref(),
                p.budget_items.unwrap_or(20),
                p.budget_chars.unwrap_or(8000),
                p.no_feedback,
                None,
            )
            .await
            .map_err(me)?,
    ))
}

/// 记忆域缺失向量统计（原子/场景）。
#[utoipa::path(get, path = "/memory/embeddings/status",
    responses((status = 200, body = EmbeddingStatus)))]
pub async fn embedding_status(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<EmbeddingStatus>, ApiError> {
    require_memory_read(&principal)?;
    Ok(Json(svc(&state).embedding_status().await.map_err(me)?))
}

/// 入队重嵌（换 embedding 供应商后的修复路径）。
#[utoipa::path(post, path = "/memory/reembed",
    responses((status = 202, description = "重嵌 job 已入队")))]
pub async fn reembed_memory(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    require_memory(&principal)?;
    svc(&state).reembed().await.map_err(me)?;
    Ok(StatusCode::ACCEPTED)
}

/// 画像活文档（P015）：当前版 + 历史。
#[utoipa::path(get, path = "/memory/persona-doc",
    responses((status = 200, body = serde_json::Value)))]
pub async fn persona_doc(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<PersonaDocParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_memory_read(&principal)?;
    Ok(Json(
        svc(&state)
            .persona_doc(p.history_limit.unwrap_or(10))
            .await
            .map_err(me)?,
    ))
}

#[derive(Deserialize, IntoParams)]
pub struct PersonaDocParams {
    pub history_limit: Option<i64>,
}

/// 手动触发离线整理 Agent（判重/归档/画像维护）。cron 通道专用。
#[utoipa::path(post, path = "/memory/maintain",
    responses((status = 200, body = serde_json::Value)))]
pub async fn trigger_maintain(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_cron(&principal)?;
    let by = actor_of(&principal);
    Ok(Json(
        svc(&state)
            .trigger_maintain(by.as_str())
            .await
            .map_err(me)?,
    ))
}

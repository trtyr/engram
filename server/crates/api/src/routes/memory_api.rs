//! 记忆域端点（memory scope）。

use agent_memory_core::memory::{
    AtomDto, ContextPack, MemoryError, MemoryService, PersonaVersion, ScenarioDto, SearchResponse,
    SessionDto,
};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::auth::{Principal, require_scope};
use crate::error::ApiError;
use crate::state::AppState;

fn require_memory(p: &Principal) -> Result<(), ApiError> {
    require_scope(p, "memory").map_err(ApiError::from)
}

fn me(e: MemoryError) -> ApiError {
    match e {
        MemoryError::NotFound(m) => ApiError::NotFound(m),
        MemoryError::BadRequest(m) => ApiError::BadRequest(m),
        MemoryError::Storage(m) => ApiError::Unavailable(m),
        MemoryError::LlmNotConfigured(m) => ApiError::Unavailable(m),
    }
}

fn svc(state: &AppState) -> MemoryService {
    MemoryService::new(state.pool.clone(), state.registry())
}

// ---------- L0 会话 ----------

#[derive(Deserialize, utoipa::ToSchema)]
pub struct WriteSessionRequest {
    pub agent: Option<String>,
    /// 轮次数组：[{speaker, text, ts?}]
    #[schema(value_type = Object)]
    pub turns: serde_json::Value,
    /// auto（默认，防抖触发蒸馏）| manual（立即）| off
    #[serde(default = "default_distill")]
    pub distill: String,
}
fn default_distill() -> String {
    "auto".into()
}

/// 写入 L0 会话（AI 客户端的主要写入口）。
#[utoipa::path(post, path = "/memory/sessions",
    request_body = WriteSessionRequest,
    responses((status = 201, body = SessionDto)))]
pub async fn write_session(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<WriteSessionRequest>,
) -> Result<(StatusCode, Json<SessionDto>), ApiError> {
    require_memory(&principal)?;
    let agent = req.agent.unwrap_or_else(|| "default".into());
    let s = svc(&state)
        .write_session(&agent, req.turns, &req.distill)
        .await
        .map_err(me)?;
    Ok((StatusCode::CREATED, Json(s)))
}

#[derive(Deserialize, IntoParams)]
pub struct ListSessionsParams {
    pub agent: Option<String>,
    pub cursor: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<i64>,
}

#[utoipa::path(get, path = "/memory/sessions", params(ListSessionsParams),
    responses((status = 200, body = [SessionDto])))]
pub async fn list_sessions(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ListSessionsParams>,
) -> Result<Json<Vec<SessionDto>>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(
        svc(&state)
            .list_sessions(p.agent.as_deref(), p.cursor, p.limit.unwrap_or(50))
            .await
            .map_err(me)?,
    ))
}

#[utoipa::path(get, path = "/memory/sessions/{id}",
    responses((status = 200, body = SessionDto), (status = 404, body = crate::error::ErrorEnvelope)))]
pub async fn get_session(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<SessionDto>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(svc(&state).get_session(id).await.map_err(me)?))
}

/// 擦除会话（引用它的原子来源标记失效）。
#[utoipa::path(delete, path = "/memory/sessions/{id}", responses((status = 204)))]
pub async fn erase_session(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_memory(&principal)?;
    svc(&state).erase_session(id).await.map_err(me)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct DistillRequest {
    /// true 时附带 consolidate
    #[serde(default)]
    pub full: bool,
}

/// 手动触发蒸馏链。
#[utoipa::path(post, path = "/memory/distill",
    request_body = DistillRequest,
    responses((status = 202, body = [agent_memory_jobs::Job])))]
pub async fn trigger_distill(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<DistillRequest>,
) -> Result<(StatusCode, Json<Vec<agent_memory_jobs::Job>>), ApiError> {
    require_memory(&principal)?;
    let jobs = svc(&state).trigger_distill(req.full).await.map_err(me)?;
    Ok((StatusCode::ACCEPTED, Json(jobs)))
}

// ---------- L1 原子 ----------

#[derive(Deserialize, IntoParams)]
pub struct ListAtomsParams {
    pub kind: Option<String>,
    pub status: Option<String>,
    pub needs_review: Option<bool>,
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
    require_memory(&principal)?;
    Ok(Json(
        svc(&state)
            .list_atoms(
                p.kind.as_deref(),
                p.status.as_deref(),
                p.needs_review,
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
}
fn default_conf() -> f32 {
    0.9
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
    let a = svc(&state)
        .create_atom(&req.kind, &req.content, req.confidence)
        .await
        .map_err(me)?;
    Ok((StatusCode::CREATED, Json(a)))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateAtomRequest {
    pub content: Option<String>,
    pub confidence: Option<f32>,
    /// 只允许 "archived" / "active"
    pub status: Option<String>,
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
    Ok(Json(
        svc(&state)
            .update_atom(
                id,
                req.content.as_deref(),
                req.confidence,
                req.status.as_deref(),
            )
            .await
            .map_err(me)?,
    ))
}

// ---------- L2 场景 ----------

#[utoipa::path(get, path = "/memory/scenarios", responses((status = 200, body = [ScenarioDto])))]
pub async fn list_scenarios(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ScenarioDto>>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(svc(&state).list_scenarios(100).await.map_err(me)?))
}

#[utoipa::path(get, path = "/memory/scenarios/{id}",
    responses((status = 200, body = ScenarioDto)))]
pub async fn get_scenario(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ScenarioDto>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(svc(&state).get_scenario(id).await.map_err(me)?))
}

// ---------- L3 画像 ----------

#[utoipa::path(get, path = "/memory/persona", responses((status = 200, body = [PersonaVersion])))]
pub async fn get_persona(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<PersonaVersion>>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(svc(&state).persona().await.map_err(me)?))
}

#[derive(Deserialize, IntoParams)]
pub struct HistoryParams {
    pub aspect: String,
}

#[utoipa::path(get, path = "/memory/persona/history", params(HistoryParams),
    responses((status = 200, body = [PersonaVersion])))]
pub async fn persona_history(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<HistoryParams>,
) -> Result<Json<Vec<PersonaVersion>>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(
        svc(&state).persona_history(&p.aspect).await.map_err(me)?,
    ))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct RollbackRequest {
    pub aspect: String,
    pub to_version: i32,
}

/// 回滚分面到历史版本（以新版本落地，历史不可变）。
#[utoipa::path(post, path = "/memory/persona/rollback",
    request_body = RollbackRequest,
    responses((status = 200, body = PersonaVersion)))]
pub async fn persona_rollback(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<RollbackRequest>,
) -> Result<Json<PersonaVersion>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(
        svc(&state)
            .persona_rollback(&req.aspect, req.to_version)
            .await
            .map_err(me)?,
    ))
}

// ---------- 检索 ----------

#[derive(Deserialize, utoipa::ToSchema)]
pub struct SearchRequest {
    pub query: String,
    /// 层过滤：["l1","l2","l3"]，空 = 全部
    #[serde(default)]
    pub layers: Vec<String>,
    pub max_items: Option<i64>,
}

#[utoipa::path(post, path = "/memory/search",
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
            .search(&req.query, &layers, req.max_items.unwrap_or(20))
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
}

/// AI 冷启动首选：一站式上下文包（L3+L2+L1 按预算裁剪）。
#[utoipa::path(get, path = "/memory/context", params(ContextParams),
    responses((status = 200, body = ContextPack)))]
pub async fn context(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ContextParams>,
) -> Result<Json<ContextPack>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(
        svc(&state)
            .context_pack(
                p.query.as_deref(),
                p.budget_items.unwrap_or(20),
                p.budget_chars.unwrap_or(8000),
            )
            .await
            .map_err(me)?,
    ))
}

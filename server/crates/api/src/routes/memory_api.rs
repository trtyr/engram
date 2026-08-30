//! 记忆域端点（memory scope）。

use agent_memory_core::memory::{
    AtomDto, ContextPack, EntityDetail, EntityDto, EntityGraph, MemoryError, MemoryService,
    PersonaVersion, ScenarioDto, SearchResponse, SessionDto,
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
    require_scope(p, "memory")
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

// ---------- 实体（记忆星系） ----------

#[derive(Deserialize, IntoParams)]
pub struct ListEntitiesParams {
    /// person / project / topic / group
    pub kind: Option<String>,
}

/// 实体列表（按记忆密度降序）。
#[utoipa::path(get, path = "/memory/entities", params(ListEntitiesParams),
    responses((status = 200, body = [EntityDto])))]
pub async fn list_entities(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ListEntitiesParams>,
) -> Result<Json<Vec<EntityDto>>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(
        svc(&state)
            .list_entities(p.kind.as_deref())
            .await
            .map_err(me)?,
    ))
}

/// 星系图：节点 + 共现边。
#[utoipa::path(get, path = "/memory/entities/graph",
    responses((status = 200, body = EntityGraph)))]
pub async fn entity_graph(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<EntityGraph>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(svc(&state).entity_graph().await.map_err(me)?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CreateEntityRequest {
    pub name: String,
    /// person / project / topic / group
    pub kind: String,
    /// 画像摘要（关系行文，可后补）
    #[serde(default)]
    pub summary: String,
}

/// 手动建实体。
#[utoipa::path(post, path = "/memory/entities", request_body = CreateEntityRequest,
    responses((status = 201, body = EntityDto)))]
pub async fn create_entity(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<CreateEntityRequest>,
) -> Result<(StatusCode, Json<EntityDto>), ApiError> {
    require_memory(&principal)?;
    let e = svc(&state)
        .create_entity(&req.name, &req.kind, &req.summary)
        .await
        .map_err(me)?;
    Ok((StatusCode::CREATED, Json(e)))
}

/// 实体详情：画像摘要 + 相关原子时间线 + 相关场景。
#[utoipa::path(get, path = "/memory/entities/{id}",
    responses((status = 200, body = EntityDetail)))]
pub async fn get_entity(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<EntityDetail>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(svc(&state).get_entity(id).await.map_err(me)?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateEntityRequest {
    pub name: Option<String>,
    pub summary: Option<String>,
}

/// 改名/改画像摘要。
#[utoipa::path(patch, path = "/memory/entities/{id}", request_body = UpdateEntityRequest,
    responses((status = 200, body = EntityDto)))]
pub async fn update_entity(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateEntityRequest>,
) -> Result<Json<EntityDto>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(
        svc(&state)
            .update_entity(id, req.name.as_deref(), req.summary.as_deref())
            .await
            .map_err(me)?,
    ))
}

/// 删实体（关联原子保留，仅解除关联）。
#[utoipa::path(delete, path = "/memory/entities/{id}",
    responses((status = 204)))]
pub async fn delete_entity(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_memory(&principal)?;
    svc(&state).delete_entity(id).await.map_err(me)?;
    Ok(StatusCode::NO_CONTENT)
}

/// 挂原子到实体（幂等）。
#[utoipa::path(post, path = "/memory/entities/{id}/atoms/{atom_id}",
    responses((status = 204)))]
pub async fn attach_atom(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path((id, atom_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    require_memory(&principal)?;
    svc(&state).attach_atom(id, atom_id).await.map_err(me)?;
    Ok(StatusCode::NO_CONTENT)
}

/// 摘除原子关联。
#[utoipa::path(delete, path = "/memory/entities/{id}/atoms/{atom_id}",
    responses((status = 204)))]
pub async fn detach_atom(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path((id, atom_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    require_memory(&principal)?;
    svc(&state).detach_atom(id, atom_id).await.map_err(me)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct MergeEntityRequest {
    /// 合并目标（幸存实体）
    pub into: Uuid,
}

/// 合并实体：原子关联全部改挂目标，from 置 merged_into 让出唯一名。
#[utoipa::path(post, path = "/memory/entities/{id}/merge", request_body = MergeEntityRequest,
    responses((status = 200, body = Object)))]
pub async fn merge_entity(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<MergeEntityRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_memory(&principal)?;
    let moved = svc(&state).merge_entities(id, req.into).await.map_err(me)?;
    Ok(Json(serde_json::json!({ "moved": moved })))
}

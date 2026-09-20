//! `memory_api` 的实现切片（架构治理 2026-09-21：自 memory_api.rs 纯搬移，零行为变化）。

use super::*;

#[derive(Deserialize, utoipa::ToSchema)]
pub struct WriteSessionRequest {
    pub agent: Option<String>,
    /// 轮次数组：[{speaker, text, ts?}]
    #[schema(value_type = Object)]
    pub turns: serde_json::Value,
    /// auto（默认，防抖触发蒸馏）| manual（立即）| off
    #[serde(default = "default_distill")]
    pub distill: String,
    /// 会话级敏感标记：整段对话含隐私（医疗/感情/财务），蒸馏产物自动继承 sensitive
    #[serde(default)]
    pub sensitive: bool,
    /// 客户端幂等键（可选）：同一 ref 重复调用返回原会话不新建——网络重试防重
    pub client_ref: Option<String>,
}

pub(crate) fn default_distill() -> String {
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
    // P001 身份归因：key 写入自动带 key_id + key 名快照（key 删除不丢归因）
    let (api_key_id, key_name_snapshot) = match &principal.0 {
        Principal::ApiKey { key_id, name, .. } => (Some(*key_id), Some(name.as_str())),
        Principal::Admin => (None, None),
    };
    let s = svc(&state)
        .write_session_identity(
            &agent,
            req.turns,
            &req.distill,
            req.sensitive,
            api_key_id,
            key_name_snapshot,
            req.client_ref.as_deref(),
        )
        .await
        .map_err(me)?;
    Ok((StatusCode::CREATED, Json(s)))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ImportSessionRequest {
    pub agent: Option<String>,
    /// 导入内容全文：JSONL（每行 {role, content}）或纯文本（空行分段）
    pub content: String,
    /// jsonl | text
    pub format: String,
    /// auto（默认，防抖触发蒸馏）| manual（立即）| off
    #[serde(default = "default_distill")]
    pub distill: String,
}

/// 批量导入历史对话为会话（phase-2）：JSONL/纯文本 → turns → 落 session（source=import）。
#[utoipa::path(post, path = "/memory/sessions/import",
    request_body = ImportSessionRequest,
    responses((status = 201, body = SessionDto)))]
pub async fn import_session(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<ImportSessionRequest>,
) -> Result<(StatusCode, Json<SessionDto>), ApiError> {
    require_memory(&principal)?;
    let agent = req.agent.unwrap_or_else(|| "default".into());
    let s = svc(&state)
        .import_session(&agent, &req.content, &req.format, &req.distill)
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
    operation_id = "list_memory_sessions",
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
    // erase 不可逆且污染溯源——比读写高一级：memory + erase 双 scope（admin 全权）。
    // 2026-08-31 用户批准：多 AI 共享 key 时任何一个不能单独毁库。
    require_memory(&principal)?;
    match &*principal {
        Principal::Admin => {}
        Principal::ApiKey { scopes, .. } if scopes.iter().any(|s| s == "erase") => {}
        _ => {
            return Err(ApiError::Forbidden(
                "擦除需要 erase scope（不可逆操作，与读写分权）".into(),
            ));
        }
    }
    svc(&state).erase_session(id).await.map_err(me)?;
    Ok(StatusCode::NO_CONTENT)
}

/// P5 会话作废（v2 扩大语义）：「这段白记了」——任何会话可作废：pending/off 蒸馏跳过；
/// done 会话作废时其蒸馏产出的 active 原子级联归档（检索/context 立即失效），原文保留可审计。
#[utoipa::path(post, path = "/memory/sessions/{id}/void",
    responses((status = 200, body = SessionDto), (status = 400, description = "不存在或已蒸馏")))]
pub async fn void_session(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<SessionDto>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(svc(&state).void_session(id).await.map_err(me)?))
}

/// 撤销作废（Web 前端「恢复」按钮的通道，与 MCP forget mode=restore 同一 core 服务）：
/// 会话状态照作废时存档还原，被级联归档的原子一并恢复。非破坏性——不需要 erase scope。
#[utoipa::path(post, path = "/memory/sessions/{id}/restore",
    responses((status = 200, body = Object), (status = 400, description = "不存在或非 void 状态")))]
pub async fn restore_session(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_memory(&principal)?;
    let (session, restored) = svc(&state).unvoid_session(id).await.map_err(me)?;
    Ok(Json(serde_json::json!({
        "session": session,
        "restored_atoms": restored,
    })))
}

/// 批量遗忘请求：会话 id 列表。
#[derive(Deserialize, utoipa::ToSchema)]
pub struct BatchIdsRequest {
    /// 要操作的会话 id 列表（1~200 条）
    pub ids: Vec<Uuid>,
}

/// 批量撤销作废（Web「恢复所选」）：逐条恢复，单条失败不影响其余。
/// 混合选择（含非 void）时 failed 逐条带原因。
#[utoipa::path(post, path = "/memory/sessions/batch-restore",
    request_body = BatchIdsRequest,
    responses((status = 200, body = Object), (status = 400)))]
pub async fn batch_restore_sessions(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<BatchIdsRequest>,
) -> Result<Json<engram_core::memory::BatchOutcome>, ApiError> {
    require_memory(&principal)?;
    validate_batch(&req.ids)?;
    Ok(Json(svc(&state).unvoid_sessions(&req.ids).await))
}

/// 批量擦除（Web「擦除所选」）：物理删除，逐条含原子级联；与单条擦除同级——需 erase scope。
#[utoipa::path(post, path = "/memory/sessions/batch-erase",
    request_body = BatchIdsRequest,
    responses((status = 200, body = Object), (status = 400), (status = 403)))]
pub async fn batch_erase_sessions(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<BatchIdsRequest>,
) -> Result<Json<engram_core::memory::BatchOutcome>, ApiError> {
    require_memory(&principal)?;
    match &*principal {
        Principal::Admin => {}
        Principal::ApiKey { scopes, .. } if scopes.iter().any(|s| s == "erase") => {}
        _ => {
            return Err(ApiError::Forbidden(
                "擦除需要 erase scope（不可逆操作，与读写分权）".into(),
            ));
        }
    }
    validate_batch(&req.ids)?;
    Ok(Json(svc(&state).erase_sessions(&req.ids).await))
}

/// 批量清单校验：非空、单批上限 200（防一次勾几百条把请求拖成无超时慢查询）。
pub(crate) fn validate_batch(ids: &[Uuid]) -> Result<(), ApiError> {
    if ids.is_empty() {
        return Err(ApiError::BadRequest(
            "ids 不能为空——先勾选要操作的会话".into(),
        ));
    }
    if ids.len() > 200 {
        return Err(ApiError::BadRequest(format!(
            "单批最多 200 条（收到 {}）——分批操作",
            ids.len()
        )));
    }
    Ok(())
}

/// 追加轮次到既有会话（自动节律 b 配套：长对话分片落库，不等收尾）。
#[derive(Deserialize, utoipa::ToSchema)]
pub struct AppendSessionRequest {
    #[schema(value_type = Object)]
    pub turns: serde_json::Value,
    /// 补记会话归属（pi extension 传 session/model 名；不传保持原值）
    pub agent: Option<String>,
    /// auto（默认，防抖）| off
    #[serde(default = "default_distill")]
    pub distill: String,
}

#[utoipa::path(post, path = "/memory/sessions/{id}/append",
    request_body = AppendSessionRequest,
    responses((status = 200, body = SessionDto), (status = 400, description = "已蒸馏会话不可追加")))]
pub async fn append_session(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<AppendSessionRequest>,
) -> Result<Json<SessionDto>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(
        svc(&state)
            .append_session(id, req.turns, req.agent.as_deref(), &req.distill)
            .await
            .map_err(me)?,
    ))
}

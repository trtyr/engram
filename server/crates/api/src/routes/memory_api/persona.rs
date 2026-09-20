//! `memory_api` 的实现切片（架构治理 2026-09-21：自 memory_api.rs 纯搬移，零行为变化）。

use super::*;

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
    // 回滚 = 改写语义（重写当前版本 + 钉住），仅限用户
    if !matches!(&*principal, Principal::Admin) {
        return Err(ApiError::Forbidden(
            "画像回滚仅限用户（Web 登录态）；AI 纠错走 correction 流程".into(),
        ));
    }
    let actor = actor_of(&principal);
    Ok(Json(
        svc(&state)
            .persona_rollback(&req.aspect, req.to_version, &actor)
            .await
            .map_err(me)?,
    ))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct PersonaEditRequest {
    /// 分面（identity/preferences/skills/constraints/communication_style/goals/routines）
    pub aspect: String,
    /// 新内容；缺省时不改内容
    pub content: Option<String>,
    /// false = 解除钉住，回归蒸馏管辖（手编保护关闭）
    pub pinned: Option<bool>,
}

/// 用户编辑画像分面（留痕：新版本 manually_edited=true + 审计行）。
#[utoipa::path(patch, path = "/memory/persona",
    request_body = PersonaEditRequest,
    responses(
        (status = 200, body = PersonaVersion, description = "编辑/解钉后的分面最新版"),
        (status = 403, body = crate::error::ErrorEnvelope),
    ))]
pub async fn persona_edit(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<PersonaEditRequest>,
) -> Result<Json<PersonaVersion>, ApiError> {
    require_memory(&principal)?;
    // 编辑画像是"用户直改"语义——AI 禁入（它有自己的蒸馏/correction 通道）
    if !matches!(&*principal, Principal::Admin) {
        return Err(ApiError::Forbidden(
            "画像编辑仅限用户（Web 登录态）——AI 的画像认知由蒸馏与 correction 维护".into(),
        ));
    }
    let actor = actor_of(&principal);
    let svc = svc(&state);
    if let Some(content) = req.content.as_deref() {
        let v = svc
            .persona_edit(&req.aspect, content, &actor)
            .await
            .map_err(me)?;
        return Ok(Json(v));
    }
    match req.pinned {
        Some(false) => svc.persona_unpin(&req.aspect, &actor).await.map_err(me)?,
        Some(true) => {
            svc.persona_repin(&req.aspect, &actor).await.map_err(me)?;
        }
        None => {}
    }
    let latest = svc
        .persona_history(&req.aspect)
        .await
        .map_err(me)?
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::NotFound(format!("分面 {} 不存在", req.aspect)))?;
    Ok(Json(latest))
}

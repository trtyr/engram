//! 工单域端点（0074 拆表）：项目绑定制——工单必须绑定已有项目。
//! scope 沿用 "todos"（工单与待办本属同一治理面拆分，存量 key 不因拆表失效）。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use engram_core::tickets::{TicketDto, TicketError, TicketService};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::{Principal, require_scope, require_scope_read};
use crate::error::ApiError;
use crate::state::AppState;

fn require_tickets(p: &Principal) -> Result<(), ApiError> {
    require_scope(p, "todos")
}
fn require_tickets_read(p: &Principal) -> Result<(), ApiError> {
    require_scope_read(p, "todos")
}

fn te(e: TicketError) -> ApiError {
    match e {
        TicketError::NotFound(m) => ApiError::NotFound(m),
        TicketError::BadRequest(m) => ApiError::BadRequest(m),
        TicketError::Storage(m) => ApiError::Unavailable(m),
    }
}

fn svc(state: &AppState) -> TicketService {
    TicketService::new(state.pool.clone())
}

#[derive(Deserialize, utoipa::IntoParams)]
pub struct ListTicketsParams {
    pub status: Option<String>,
    /// 可选：工单严重度 P0-P3
    pub severity: Option<String>,
    /// 可选：按项目过滤（项目绑定的查询面）
    pub project_id: Option<Uuid>,
    pub q: Option<String>,
    /// keyset 分页游标：{1|0}|{updated_at ISO8601}|{id}（1=该条 status=open）
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CreateTicketRequest {
    /// 项目 id（必填——绑定不了的工单不存在）
    pub project_id: Uuid,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// 可选：工单严重度 P0-P3
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub symptom: String,
    #[serde(default)]
    pub reproduce: String,
    #[serde(default)]
    pub acceptance: String,
}

/// serde 双层 Option：字段缺失→None（不动）；字段=null→Some(None)（显式清除）；字段=值→Some(Some(v))。
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<T>::deserialize(de)?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateTicketRequest {
    pub title: Option<String>,
    pub body: Option<String>,
    /// open | confirmed | in_progress | resolved | verified | archived
    pub status: Option<String>,
    /// null=显式清除（回到未定级）
    #[serde(default, deserialize_with = "double_option")]
    pub severity: Option<Option<String>>,
    pub symptom: Option<String>,
    pub reproduce: Option<String>,
    pub acceptance: Option<String>,
    /// 转 resolved/verified 必填（做了什么/怎么修的）
    pub resolution: Option<String>,
}

/// 工单列表（open 优先；status/severity/project/q 过滤）。
#[utoipa::path(get, path = "/tickets", params(ListTicketsParams),
    responses((status = 200, body = [TicketDto])))]
pub async fn list_tickets(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ListTicketsParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_tickets_read(&principal)?;
    let (items, total) = svc(&state)
        .list(
            p.status.as_deref(),
            p.severity.as_deref(),
            p.project_id,
            p.q.as_deref(),
            p.cursor.as_deref(),
            p.limit.unwrap_or(200),
        )
        .await
        .map_err(te)?;
    Ok(Json(serde_json::json!({ "items": items, "total": total })))
}

/// 新建工单（project_id 必填且必须存在——绑定不了直接 400）。
#[utoipa::path(post, path = "/tickets", request_body = CreateTicketRequest,
    responses((status = 201, body = TicketDto)))]
pub async fn create_ticket(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<CreateTicketRequest>,
) -> Result<(StatusCode, Json<TicketDto>), ApiError> {
    require_tickets(&principal)?;
    let dto = svc(&state)
        .create(
            req.project_id,
            &req.title,
            &req.body,
            req.severity.as_deref(),
            &req.symptom,
            &req.reproduce,
            &req.acceptance,
        )
        .await
        .map_err(te)?;
    Ok((StatusCode::CREATED, Json(dto)))
}

/// 工单详情。
#[utoipa::path(get, path = "/tickets/{id}", responses((status = 200, body = TicketDto), (status = 404, body = crate::error::ErrorEnvelope)))]
pub async fn get_ticket(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<TicketDto>, ApiError> {
    require_tickets_read(&principal)?;
    let svc = svc(&state);
    let dto = svc
        .find_by_ref(&id.to_string())
        .await
        .map_err(te)?
        .ok_or_else(|| ApiError::NotFound(format!("工单 {id} 不存在")))?;
    Ok(Json(dto))
}

/// 更新工单（部分字段；转 resolved/verified 必须带 resolution）。
#[utoipa::path(put, path = "/tickets/{id}", request_body = UpdateTicketRequest,
    responses((status = 200, body = TicketDto), (status = 404, body = crate::error::ErrorEnvelope)))]
pub async fn update_ticket(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTicketRequest>,
) -> Result<Json<TicketDto>, ApiError> {
    require_tickets(&principal)?;
    Ok(Json(
        svc(&state)
            .update(
                id,
                req.title.as_deref(),
                req.body.as_deref(),
                req.status.as_deref(),
                req.severity.as_ref().map(|o| o.as_deref()),
                req.symptom.as_deref(),
                req.reproduce.as_deref(),
                req.acceptance.as_deref(),
                req.resolution.as_deref(),
                "console",
            )
            .await
            .map_err(te)?,
    ))
}

/// 删除工单（物理删除；归档语义走 status=archived）。
#[utoipa::path(delete, path = "/tickets/{id}", responses((status = 204), (status = 404, body = crate::error::ErrorEnvelope)))]
pub async fn delete_ticket(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_tickets(&principal)?;
    svc(&state).delete(id).await.map_err(te)?;
    Ok(StatusCode::NO_CONTENT)
}

/// 活动时间线（状态流转 event + 评论 comment，升序）。
#[utoipa::path(get, path = "/tickets/{id}/events", responses((status = 200)))]
pub async fn ticket_events(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_tickets_read(&principal)?;
    let rows = svc(&state).events(id).await.map_err(te)?;
    Ok(Json(serde_json::json!({ "events": rows })))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct TicketCommentRequest {
    pub text: String,
}

/// 工单评论（入活动时间线）。
#[utoipa::path(post, path = "/tickets/{id}/events", request_body = TicketCommentRequest,
    responses((status = 201)))]
pub async fn ticket_comment(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::Json(req): axum::Json<TicketCommentRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    require_tickets(&principal)?;
    svc(&state)
        .event_add(
            id,
            "comment",
            &serde_json::json!({ "text": req.text.trim() }),
            "console",
        )
        .await
        .map_err(te)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "commented": true })),
    ))
}

/// 工单全量导出（P4 数据主权）。
#[utoipa::path(get, path = "/tickets/export", responses((status = 200, body = [TicketDto])))]
pub async fn export_tickets(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_tickets_read(&principal)?;
    let (items, total) = svc(&state)
        .list(None, None, None, None, None, 10_000)
        .await
        .map_err(te)?;
    Ok(Json(serde_json::json!({ "items": items, "total": total })))
}

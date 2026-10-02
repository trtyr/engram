//! study 学习路线图 HTTP 端点（P007 二期 T007）：与 MCP study 域同源（StudyService）。
//! 分工：study 只存「学没学/学到哪/下一步学啥」的过程状态；
//! 知识内容归 wiki，原文归 documents，感悟叙事归 memory。

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::{Principal, require_scope, require_scope_read};
use crate::error::ApiError;
use crate::state::AppState;

use engram_core::study::{StudyError, StudyService};

fn svc(state: &AppState) -> StudyService {
    StudyService::new(state.pool.clone())
}

fn se(e: StudyError) -> ApiError {
    match e {
        StudyError::NotFound(m) => ApiError::NotFound(m),
        StudyError::BadRequest(m) => ApiError::BadRequest(m),
        StudyError::Storage(m) => ApiError::Unavailable(m),
    }
}

#[derive(Deserialize)]
pub struct StudyTopicCreateRequest {
    pub name: String,
    pub goal: Option<String>,
}

#[derive(Deserialize)]
pub struct StudyTopicUpdateRequest {
    pub name: Option<String>,
    pub goal: Option<String>,
    pub status: Option<String>,
}

#[derive(Deserialize)]
pub struct StudyItemAddRequest {
    pub name: String,
    pub position: Option<i32>,
}

#[derive(Deserialize)]
pub struct StudyItemPatchRequest {
    pub status: Option<String>,
    pub wiki_slugs: Option<Vec<String>>,
    pub doc_ids: Option<Vec<String>>,
    pub needs_review: Option<bool>,
    /// RFC3339；needs_review=true 且缺省时立即到期
    pub review_due_at: Option<String>,
}

/// 全部学习领域（简报）。
#[utoipa::path(get, path = "/study/topics",
    responses((status = 200, body = serde_json::Value)))]
pub async fn list_topics(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_scope_read(&principal, "study")?;
    let rows = svc(&state).topic_list().await.map_err(se)?;
    Ok(Json(serde_json::json!({ "topics": rows, "count": rows.len() })))
}

/// 开题。
#[utoipa::path(post, path = "/study/topics",
    request_body = serde_json::Value,
    responses((status = 201, body = serde_json::Value)))]
pub async fn create_topic(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<StudyTopicCreateRequest>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), ApiError> {
    require_scope(&principal, "study")?;
    let id = svc(&state)
        .topic_create(&req.name, req.goal.as_deref().unwrap_or(""))
        .await
        .map_err(se)?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(serde_json::json!({ "id": id, "name": req.name.trim() })),
    ))
}

/// topic 全量（进度+下一步队列+进行中+资料清单）。
#[utoipa::path(get, path = "/study/topics/{id}",
    responses((status = 200, body = serde_json::Value)))]
pub async fn get_topic(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_scope_read(&principal, "study")?;
    let full = svc(&state)
        .topic_get(id)
        .await
        .map_err(se)?
        .ok_or_else(|| ApiError::NotFound(format!("topic {id} 不存在")))?;
    Ok(Json(serde_json::to_value(&full).unwrap_or(serde_json::json!({}))))
}

/// 补丁式更新 topic。
#[utoipa::path(patch, path = "/study/topics/{id}",
    request_body = serde_json::Value,
    responses((status = 200, body = serde_json::Value)))]
pub async fn update_topic(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<StudyTopicUpdateRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_scope(&principal, "study")?;
    svc(&state)
        .topic_update(id, req.name.as_deref(), req.goal.as_deref(), req.status.as_deref())
        .await
        .map_err(se)?;
    Ok(Json(serde_json::json!({ "id": id, "ok": true })))
}

/// 删 topic（级联节点）。
#[utoipa::path(delete, path = "/study/topics/{id}",
    responses((status = 204)))]
pub async fn delete_topic(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    require_scope(&principal, "study")?;
    svc(&state).topic_delete(id).await.map_err(se)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// 加知识点。
#[utoipa::path(post, path = "/study/topics/{id}/items",
    request_body = serde_json::Value,
    responses((status = 201, body = serde_json::Value)))]
pub async fn add_item(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<StudyItemAddRequest>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), ApiError> {
    require_scope(&principal, "study")?;
    let item = svc(&state)
        .item_add(id, &req.name, req.position)
        .await
        .map_err(se)?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(serde_json::json!({ "id": item, "name": req.name.trim() })),
    ))
}

/// 知识点补丁（状态机/挂资料）。
#[utoipa::path(patch, path = "/study/items/{id}",
    request_body = serde_json::Value,
    responses((status = 200, body = serde_json::Value)))]
pub async fn patch_item(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<StudyItemPatchRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_scope(&principal, "study")?;
    if let Some(status) = &req.status {
        svc(&state).item_set_status(id, status).await.map_err(se)?;
    }
    if req.wiki_slugs.is_some() || req.doc_ids.is_some() {
        svc(&state)
            .item_link(id, req.wiki_slugs.clone(), req.doc_ids.clone())
            .await
            .map_err(se)?;
    }
    if let Some(nr) = req.needs_review {
        let due = match &req.review_due_at {
            Some(raw) => Some(
                chrono::DateTime::parse_from_rfc3339(raw)
                    .map_err(|e| ApiError::BadRequest(format!("review_due_at 非法: {e}")))?
                    .with_timezone(&chrono::Utc),
            ),
            None => None,
        };
        svc(&state).item_set_review(id, nr, due).await.map_err(se)?;
    }
    Ok(Json(serde_json::json!({ "id": id, "ok": true })))
}

/// 删知识点。
#[utoipa::path(delete, path = "/study/items/{id}",
    responses((status = 204)))]
pub async fn delete_item(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    require_scope(&principal, "study")?;
    engram_storage::repo::study::item_delete(&state.pool, id)
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// 复习队列（已标记且到期，review_due_at 升序）。
#[utoipa::path(get, path = "/study/reviews",
    responses((status = 200, body = serde_json::Value)))]
pub async fn reviews_due(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_scope_read(&principal, "study")?;
    let rows = svc(&state).reviews_due().await.map_err(se)?;
    Ok(Json(serde_json::json!({ "reviews": rows, "count": rows.len() })))
}

/// journal 进度时间线：记一笔。
#[utoipa::path(post, path = "/study/topics/{id}/journal",
    request_body = serde_json::Value,
    responses((status = 201, body = serde_json::Value)))]
pub async fn journal_add(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<std::collections::HashMap<String, String>>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), ApiError> {
    require_scope(&principal, "study")?;
    let note = req.get("note").map(String::as_str).unwrap_or("");
    let jid = svc(&state).journal_add(id, note).await.map_err(se)?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(serde_json::json!({ "id": jid, "ok": true })),
    ))
}

/// journal 进度时间线：查最近（新→旧）。
#[utoipa::path(get, path = "/study/topics/{id}/journal",
    responses((status = 200, body = serde_json::Value)))]
pub async fn journal_list(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_scope_read(&principal, "study")?;
    let rows = svc(&state).journal_list(id, 50).await.map_err(se)?;
    Ok(Json(serde_json::json!({ "journal": rows, "count": rows.len() })))
}

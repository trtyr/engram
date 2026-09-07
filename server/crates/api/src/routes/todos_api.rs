//! 待办域端点（第七域）：不绑定项目的临时任务/灵感速记。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use engram_core::todos::{TodoDto, TodoError, TodoService};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::{Principal, require_scope};
use crate::error::ApiError;
use crate::state::AppState;

fn require_todos(p: &Principal) -> Result<(), ApiError> {
    require_scope(p, "todos")
}

fn te(e: TodoError) -> ApiError {
    match e {
        TodoError::NotFound(m) => ApiError::NotFound(m),
        TodoError::BadRequest(m) => ApiError::BadRequest(m),
        TodoError::Storage(m) => ApiError::Unavailable(m),
    }
}

fn svc(state: &AppState) -> TodoService {
    TodoService::new(state.pool.clone())
}

#[derive(Deserialize, utoipa::IntoParams)]
pub struct ListTodosParams {
    pub status: Option<String>,
    pub priority: Option<String>,
    pub tag: Option<String>,
    pub q: Option<String>,
    /// keyset 分页游标：{1|0}|{updated_at ISO8601}|{id}（1=该条 status=open）
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CreateTodoRequest {
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default = "default_priority")]
    pub priority: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub due_at: Option<DateTime<Utc>>,
    /// 可选：相关项目名提示（纯文本，不做绑定）
    pub project_hint: Option<String>,
}

fn default_priority() -> String {
    "normal".into()
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateTodoRequest {
    pub title: Option<String>,
    pub body: Option<String>,
    pub priority: Option<String>,
    /// open | done | archived
    pub status: Option<String>,
    pub due_at: Option<Option<DateTime<Utc>>>,
    pub project_hint: Option<Option<String>>,
    pub tags: Option<Vec<String>>,
}

/// 待办列表（open 优先；status/priority/tag/q 过滤）。
#[utoipa::path(get, path = "/todos", params(ListTodosParams),
    responses((status = 200, body = [TodoDto])))]
pub async fn list_todos(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ListTodosParams>,
) -> Result<Json<Vec<TodoDto>>, ApiError> {
    require_todos(&principal)?;
    Ok(Json(
        svc(&state)
            .list(
                p.status.as_deref(),
                p.priority.as_deref(),
                p.tag.as_deref(),
                p.q.as_deref(),
                p.cursor.as_deref(),
                p.limit.unwrap_or(200),
            )
            .await
            .map_err(te)?,
    ))
}

/// 新建待办。
#[utoipa::path(post, path = "/todos", request_body = CreateTodoRequest,
    responses((status = 201, body = TodoDto)))]
pub async fn create_todo(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<CreateTodoRequest>,
) -> Result<(StatusCode, Json<TodoDto>), ApiError> {
    require_todos(&principal)?;
    let dto = svc(&state)
        .create(
            &req.title,
            &req.body,
            &req.priority,
            &req.tags,
            req.due_at,
            req.project_hint.as_deref(),
        )
        .await
        .map_err(te)?;
    Ok((StatusCode::CREATED, Json(dto)))
}

/// 待办详情。
#[utoipa::path(get, path = "/todos/{id}", responses((status = 200, body = TodoDto), (status = 404, body = crate::error::ErrorEnvelope)))]
pub async fn get_todo(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<TodoDto>, ApiError> {
    require_todos(&principal)?;
    Ok(Json(svc(&state).get(id).await.map_err(te)?))
}

/// 更新待办（部分字段，None 不动；status=done 自动记 done_at）。
#[utoipa::path(put, path = "/todos/{id}", request_body = UpdateTodoRequest,
    responses((status = 200, body = TodoDto), (status = 404, body = crate::error::ErrorEnvelope)))]
pub async fn update_todo(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTodoRequest>,
) -> Result<Json<TodoDto>, ApiError> {
    require_todos(&principal)?;
    Ok(Json(
        svc(&state)
            .update(
                id,
                req.title.as_deref(),
                req.body.as_deref(),
                req.priority.as_deref(),
                req.status.as_deref(),
                req.due_at,
                req.project_hint.as_ref().map(|o| o.as_deref()),
                req.tags.as_deref(),
            )
            .await
            .map_err(te)?,
    ))
}

/// 删除待办（物理删除；归档语义走 status=archived）。
#[utoipa::path(delete, path = "/todos/{id}", responses((status = 204), (status = 404, body = crate::error::ErrorEnvelope)))]
pub async fn delete_todo(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_todos(&principal)?;
    svc(&state).delete(id).await.map_err(te)?;
    Ok(StatusCode::NO_CONTENT)
}

/// 待办全量导出（P4 数据主权）。
#[utoipa::path(get, path = "/todos/export", responses((status = 200, body = [TodoDto])))]
pub async fn export_todos(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<TodoDto>>, ApiError> {
    require_todos(&principal)?;
    Ok(Json(svc(&state).export_all().await.map_err(te)?))
}

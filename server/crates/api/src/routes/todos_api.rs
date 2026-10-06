//! 待办域端点：不绑定项目的临时任务/灵感速记。
//! 工单已拆 /tickets（0074）——ticket 参数在本域直接 400（不带旧兼容）。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use engram_core::todos::{TodoDto, TodoError, TodoService};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::{Principal, require_scope, require_scope_read};
use crate::error::ApiError;
use crate::state::AppState;

fn require_todos(p: &Principal) -> Result<(), ApiError> {
    require_scope(p, "todos")
}
/// 读语义变体：:ro 只读 key 放行（RJ-20，对齐 MCP 读动作口径）。
fn require_todos_read(p: &Principal) -> Result<(), ApiError> {
    require_scope_read(p, "todos")
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
    /// open | done | archived
    pub status: Option<String>,
    pub priority: Option<String>,
    pub tag: Option<String>,
    pub q: Option<String>,
    /// 可选 due 过滤：overdue=未完成且已过期；today=今天到期
    pub due: Option<String>,
    /// keyset 分页游标：{1|0}|{updated_at ISO8601}|{id}（1=该条 status=open）
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CreateTodoRequest {
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub due_at: Option<DateTime<Utc>>,
}

/// serde 双层 Option：字段缺失→None（不动）；字段=null→Some(None)（显式清除）；字段=值→Some(Some(v))。
/// 直接 derive 会把 null 和缺失都解成 None——清除语义到不了服务层（审计驳回点）。
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<T>::deserialize(de)?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateTodoRequest {
    pub title: Option<String>,
    pub body: Option<String>,
    pub priority: Option<String>,
    /// open | done | archived
    pub status: Option<String>,
    /// null=显式清除截止时间
    #[serde(default, deserialize_with = "double_option")]
    pub due_at: Option<Option<DateTime<Utc>>>,
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
    require_todos_read(&principal)?;
    Ok(Json(
        svc(&state)
            .list(
                p.status.as_deref(),
                p.priority.as_deref(),
                p.tag.as_deref(),
                p.q.as_deref(),
                p.due.as_deref(),
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
            req.priority.as_deref().unwrap_or("normal"),
            &req.tags,
            req.due_at,
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
    require_todos_read(&principal)?;
    Ok(Json(svc(&state).get(id).await.map_err(te)?))
}

/// 双向关联列表（GET /todos/{id}/links）——「谁阻塞我」反查。
#[utoipa::path(get, path = "/todos/{id}/links",
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Vec<serde_json::Value>)))]
pub async fn todo_links(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    require_todos_read(&principal)?;
    let raw = svc(&state).links(id).await.map_err(te)?;
    let items: Vec<serde_json::Value> = raw
        .iter()
        .map(|(from, to, kind, dir)| {
            serde_json::json!({
                "from": from, "to": to, "kind": kind, "direction": dir,
            })
        })
        .collect();
    Ok(Json(items))
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
    require_todos_read(&principal)?;
    Ok(Json(svc(&state).export_all().await.map_err(te)?))
}

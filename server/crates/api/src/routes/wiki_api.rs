//! Wiki 域端点（wiki scope）。单库终局（2026-09-20）：多库 API 已移除——
//! 无 `?lib=` 参数、无库管理端点，全部请求恒定落在 main 主库。

mod ingest;
mod ops;
mod pages;
mod repair_ops;
mod search_graph;
pub use ingest::*;
pub(crate) use ops::*;
pub use pages::*;
pub use repair_ops::*;
pub use search_graph::*;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use engram_core::wiki::libraries;
use engram_core::wiki::{CascadeReport, InsightsReport, Purpose};
use engram_core::wiki::{LintReport, WikiError, WikiPageDto, WikiPageMetaDto, WikiService};
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::auth::{Principal, require_scope, require_scope_read};
use crate::error::ApiError;
use crate::state::AppState;

fn require_wiki(p: &Principal) -> Result<(), ApiError> {
    require_scope(p, "wiki")
}
/// 读语义变体：:ro 只读 key 放行（RJ-20，对齐 MCP 读动作口径）。
fn require_wiki_read(p: &Principal) -> Result<(), ApiError> {
    require_scope_read(p, "wiki")
}

// ---------- ingest ----------

// ---------- purpose ----------

// ---------- review ----------

// ---------- queries 存档 ----------

// ---------- sources（级联删除） ----------

// ---------- 图洞察 ----------

// ---------- 知识晋升（EN-59）：项目文档 → wiki 的结构化动作；只读列表对齐 MCP ----------

/// 页面版本列表（时间线；含已删除页的最后快照）。
#[utoipa::path(get, path = "/wiki/pages/{slug}/versions",
    responses((status = 200, body = serde_json::Value)))]
pub async fn list_versions(
    axum::Extension(principal): axum::Extension<Principal>,
    State(state): State<AppState>,
    axum::extract::Path(slug): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_wiki(&principal)?;
    let lib = engram_core::wiki::libraries::resolve(&state.pool, None)
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    let items = svc(&state)
        .page_versions(lib, &slug)
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "versions": items, "count": items.len() }),
    ))
}

/// 某版本快照正文（回滚前预览/对比）。
#[utoipa::path(get, path = "/wiki/pages/{slug}/versions/{version}",
    responses((status = 200, body = serde_json::Value)))]
pub async fn version_content(
    axum::Extension(principal): axum::Extension<Principal>,
    State(state): State<AppState>,
    axum::extract::Path((slug, version)): axum::extract::Path<(String, i32)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_wiki(&principal)?;
    let lib = engram_core::wiki::libraries::resolve(&state.pool, None)
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    let item = svc(&state)
        .page_version_content(lib, &slug, version)
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    Ok(Json(
        serde_json::to_value(&item).unwrap_or(serde_json::json!({})),
    ))
}

/// 回滚到历史版本（破坏性：当前内容生成新快照后覆盖）。
#[utoipa::path(post, path = "/wiki/pages/{slug}/restore",
    request_body = serde_json::Value,
    responses((status = 200, body = serde_json::Value)))]
pub async fn restore_version(
    axum::Extension(principal): axum::Extension<Principal>,
    State(state): State<AppState>,
    axum::extract::Path(slug): axum::extract::Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_wiki(&principal)?;
    let version = body
        .get("version")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| ApiError::BadRequest("缺 version".into()))? as i32;
    let lib = engram_core::wiki::libraries::resolve(&state.pool, None)
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    let page = svc(&state)
        .restore_page_version(lib, &slug, version)
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    Ok(Json(
        serde_json::to_value(&page).unwrap_or(serde_json::json!({})),
    ))
}

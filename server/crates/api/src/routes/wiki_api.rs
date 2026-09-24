//! Wiki 域端点（wiki scope）。单库终局（2026-09-20）：多库 API 已移除——
//! 无 `?lib=` 参数、无库管理端点，全部请求恒定落在 main 主库。

mod ingest;
mod ops;
mod pages;
mod repair_review;
mod search_graph;
pub use ingest::*;
pub(crate) use ops::*;
pub use pages::*;
pub use repair_review::*;
pub use search_graph::*;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use engram_core::wiki::libraries;
use engram_core::wiki::{CascadeReport, InsightsReport, Purpose, ReviewItem};
use engram_core::wiki::{LintReport, WikiError, WikiPageDto, WikiPageMetaDto, WikiService};
use engram_jobs::types::JobEvent;
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

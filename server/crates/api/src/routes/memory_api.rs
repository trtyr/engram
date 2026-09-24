//! 记忆域端点（memory scope）。

mod atoms;
mod entities;
mod kv;
mod ops;
mod persona;
mod sessions;
pub use atoms::*;
pub use entities::*;
pub use kv::*;
pub use ops::*;
pub use persona::*;
pub use sessions::*;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use engram_core::memory::{
    AtomDto, ContextPack, EmbeddingStatus, EntityDetail, EntityDto, EntityGraph, KvEntryDto,
    MemoryError, MemoryService, PersonaVersion, ScenarioDto, SearchResponse, SessionDto,
};
use engram_search::{SearchHit, search_entities};
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::auth::{Principal, require_scope, require_scope_read};
use crate::error::ApiError;
use crate::state::AppState;

fn require_memory(p: &Principal) -> Result<(), ApiError> {
    require_scope(p, "memory")
}
/// 读语义变体：:ro 只读 key 放行（RJ-20，对齐 MCP 读动作口径）。
fn require_memory_read(p: &Principal) -> Result<(), ApiError> {
    require_scope_read(p, "memory")
}

// ---------- L0 会话 ----------

// ---------- L1 原子 ----------

/// deep purge 的确认短语（用户亲口授权的载体——AI 复述破坏半径后由用户给出）
pub use engram_core::memory::PURGE_CONFIRM_PHRASE;

// ---------- L2 场景 ----------

// ---------- L3 画像 ----------

// ---------- 画像编辑（编辑能力：用户直改分面，钉住=蒸馏绕开） ----------

// ---------- 检索 ----------

// ---------- 实体（记忆星系） ----------

// ---------- KV 治理面（EN-60）：只读——写入唯一通道是 MCP memory.kv_put（AI 管道） ----------

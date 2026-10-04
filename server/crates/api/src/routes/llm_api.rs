//! LLM provider / 路由 / 用量 / API key 端点（全部仅管理员）。

mod keys;
mod providers;
mod usage;
pub use keys::*;
pub use providers::*;
pub use usage::*;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use engram_llm::crypto::KeyCipher;
use engram_llm::provider::{LlmProvider, OpenAiCompatProvider, ProviderRegistry};
use engram_llm::types::{ChatMessage, ChatRequest, UsageRecord};
use engram_storage::StoreError;
use engram_storage::repo::keys as keys_repo;
use engram_storage::repo::llm as repo;

use crate::auth::{Principal, create_api_key};
use crate::error::ApiError;
use crate::state::AppState;

fn require_admin(principal: &Principal) -> Result<(), ApiError> {
    match principal {
        Principal::Admin => Ok(()),
        Principal::ApiKey { .. } => Err(ApiError::Forbidden("该端点仅管理员".into())),
    }
}

// ---------- providers ----------

// ---------- usage ----------

// ---------- api keys ----------

// ---------- 模型 ID 自动获取 ----------

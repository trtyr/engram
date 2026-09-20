//! LLM provider / 路由 / 用量 / API key 端点（全部仅管理员）。

mod keys;
mod providers;
mod routing;
mod usage;
pub use keys::*;
pub use providers::*;
pub use routing::*;
pub use usage::*;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use engram_llm::crypto::KeyCipher;
use engram_llm::provider::{LlmProvider, OpenAiCompatProvider, ProviderRegistry};
use engram_llm::router::{PurposeRouter, RoutingTable};
use engram_llm::types::{ChatMessage, ChatRequest, Purpose, UsageRecord};
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

// ---------- routing ----------

#[cfg(test)]
mod tests {
    use super::parse_llm_json;

    #[derive(serde::Deserialize, Debug, PartialEq)]
    pub(crate) struct T {
        a: i32,
    }

    #[test]
    fn parses_pure_json() {
        assert_eq!(parse_llm_json::<T>(r#"{"a":1}"#).unwrap(), T { a: 1 });
    }

    #[test]
    fn parses_json_in_markdown_fence() {
        assert_eq!(
            parse_llm_json::<T>("```json\n{\"a\":2}\n```").unwrap(),
            T { a: 2 }
        );
        assert_eq!(
            parse_llm_json::<T>("```\n{\"a\":3}\n```").unwrap(),
            T { a: 3 }
        );
    }

    #[test]
    fn parses_json_with_leading_prose() {
        assert_eq!(
            parse_llm_json::<T>("好的，以下是路由建议：\n{\"a\":4}\n希望有帮助"),
            Ok(T { a: 4 })
        );
    }

    #[test]
    fn rejects_garbage_with_diagnostic_head() {
        let err = parse_llm_json::<T>("完全不是 JSON").unwrap_err();
        assert!(err.contains("完全不是 JSON"), "错误应含返回片段：{err}");
        assert!(err.contains("不是合法 JSON"), "错误应说明原因：{err}");
    }
}

// ---------- usage ----------

// ---------- api keys ----------

// ---------- 模型 ID 自动获取 ----------

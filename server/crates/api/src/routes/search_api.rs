//! 跨域统一检索端点（memory + wiki）。

use axum::Json;
use axum::extract::State;
use engram_core::unified::{UnifiedError, UnifiedHit, UnifiedSearch};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::auth::Principal;
use crate::error::ApiError;
use crate::state::AppState;

/// 统一检索要求至少一个可读域 scope（Admin 恒通过）。
fn require_search(p: &Principal) -> Result<(), ApiError> {
    if p.has_scope("memory") || p.has_scope("wiki") {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "缺少 scope：memory / wiki 至少其一".into(),
        ))
    }
}

fn ue(e: UnifiedError) -> ApiError {
    match e {
        UnifiedError::BadRequest(m) => ApiError::BadRequest(m),
        UnifiedError::Storage(m) => ApiError::Unavailable(m),
    }
}

fn svc(state: &AppState) -> UnifiedSearch {
    UnifiedSearch::new(
        state.pool.clone(),
        state.registry(),
        state.data_dir.clone(),
        state.llm(),
    )
}

#[derive(Deserialize, ToSchema)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: i64,
    /// R6：可选 LLM 精排（默认关——开启时 top 候选多一次 LLM 调用，失败降级原序）
    #[serde(default)]
    pub rerank: bool,
}

fn default_limit() -> i64 {
    20
}

#[derive(serde::Serialize, ToSchema)]
pub struct SearchResponse {
    pub query: String,
    pub hits: Vec<UnifiedHit>,
}

/// 跨域统一检索：一次查询融合记忆、知识、wiki 三域。
#[utoipa::path(post, path = "/search", operation_id = "unified_search",
    request_body = SearchRequest,
    responses((status = 200, body = SearchResponse)))]
pub async fn search(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, ApiError> {
    require_search(&principal)?;
    let hits = svc(&state)
        .search(&req.query, req.limit, req.rerank)
        .await
        .map_err(ue)?;
    Ok(Json(SearchResponse {
        query: req.query,
        hits,
    }))
}

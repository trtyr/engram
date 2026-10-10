//! 跨域统一检索端点（memory + wiki）。

use axum::Json;
use axum::extract::State;
use engram_core::unified::{UnifiedError, UnifiedHit, UnifiedSearch};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::auth::Principal;
use crate::error::ApiError;
use crate::state::AppState;

use engram_core::auth::DomainAccess;

/// 统一检索实际覆盖的域（与 unified.rs 五域合并一致）。
const SEARCH_DOMAINS: [&str; 5] = ["memory", "wiki", "todos", "tickets", "circles"];

/// 命中域标签 → scope 域（unified merge 时的 domain 字段命名与 scope 域名不同源）。
fn hit_scope_domain(hit_domain: &str) -> &'static str {
    match hit_domain {
        "memory" => "memory",
        "wiki" => "wiki",
        "entity" => "circles",
        "ticket" => "tickets",
        _ => "todos",
    }
}

/// 统一检索的域准入与结果过滤（P019-M2）。
/// 判定收口到 domain_access 统一源：key 至少对一个被检索域有读权限（:ro 变体放行，
/// 对齐全平台读语义）；返回结果按 key 实际可读域过滤——旧实现 require_search
/// 用 has_scope 精确匹配（:ro 被拒），且无结果过滤（wiki-only key 可见 todos 命中）。
fn require_search(p: &Principal) -> Result<(), ApiError> {
    if SEARCH_DOMAINS
        .iter()
        .any(|d| p.domain_access(d) != DomainAccess::None)
    {
        Ok(())
    } else {
        Err(ApiError::Forbidden(format!(
            "缺少 scope：统一检索覆盖 {}，至少需其一（支持 :ro 只读变体）",
            SEARCH_DOMAINS.join(" / ")
        )))
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
    let mut hits = svc(&state)
        .search(&req.query, req.limit, req.rerank)
        .await
        .map_err(ue)?;
    // P019-M2：按 key 实际可读域过滤命中——准入域集合不得大于检索域集合
    hits.retain(|h| principal.domain_access(hit_scope_domain(&h.domain)) != DomainAccess::None);
    Ok(Json(SearchResponse {
        query: req.query,
        hits,
    }))
}

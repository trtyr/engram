//! `wiki_api` 的实现切片（架构治理 2026-09-21：自 wiki_api.rs 纯搬移，零行为变化）。

use super::*;

/// 规模化 task-5：图谱子图过滤参数——community（Louvain 全图编号）/ folder 前缀 / page_type。
#[derive(Deserialize, IntoParams)]
pub struct GraphFilterParams {
    /// Louvain 社区编号（全图口径；不传 = 全图）
    pub community: Option<usize>,
    /// folder 前缀（如 `topic-03`）；不传 = 全部
    pub folder: Option<String>,
    /// 页型过滤（entity/concept/…）；不传 = 全部
    pub page_type: Option<String>,
}

#[utoipa::path(get, path = "/wiki/graph", operation_id = "wiki_graph",
    params(GraphFilterParams),
    responses((status = 200, body = engram_core::wiki::GraphDto)))]
pub async fn graph(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(f): Query<GraphFilterParams>,
) -> Result<Json<engram_core::wiki::GraphDto>, ApiError> {
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    Ok(Json(
        svc(&state)
            .graph_filtered(
                lib,
                f.community,
                f.folder.as_deref(),
                f.page_type.as_deref(),
            )
            .await
            .map_err(we)?,
    ))
}

/// 查询缺口清单（批次② 查询日志飞轮，wiki 大库化）——零命中/低分查询即内容缺口，
/// 织入方向与 Deep Research 的输入。每次检索 UPSERT wiki_query_log（飞轮原料）。
#[utoipa::path(get, path = "/wiki/query-gaps", operation_id = "wiki_query_gaps",
    responses((status = 200)))]
pub async fn query_gaps(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    let gaps = svc(&state).query_gaps(lib, 50).await.map_err(we)?;
    Ok(Json(serde_json::json!({
        "gaps": gaps,
        "note": "零命中=内容缺口；低分=召回质量存疑——织入方向与 Deep Research 的输入",
    })))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct WikiSearchRequest {
    pub query: String,
    pub max_items: Option<i64>,
    /// 批次④：LLM rerank 精排（默认关——检索框速度优先；开启后 top-20 交模型重排，延迟 +2~8s）
    #[serde(default)]
    pub rerank: Option<bool>,
}

/// Wiki 页面检索。
#[utoipa::path(post, path = "/wiki/search", operation_id = "wiki_search",
    request_body = WikiSearchRequest,
    responses((status = 200, body = Object)))]
pub async fn search(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<WikiSearchRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    Ok(Json(
        svc(&state)
            .search_with_purpose(
                lib,
                &req.query,
                req.max_items.unwrap_or(20),
                req.rerank.unwrap_or(false),
            )
            .await
            .map_err(we)?,
    ))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct SetPurposeRequest {
    pub goals: Vec<String>,
    #[serde(default)]
    pub key_questions: Vec<String>,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub thesis: Option<String>,
}

/// 读取 purpose（wiki 方向意图；每库一份）。
#[utoipa::path(get, path = "/wiki/purpose",
    responses((status = 200, body = Purpose)))]
pub async fn get_purpose(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Option<Purpose>>, ApiError> {
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    Ok(Json(svc(&state).get_purpose(lib).await.map_err(we)?))
}

/// 设置 purpose（ingest/query 时注入 LLM；每库一份）。
#[utoipa::path(put, path = "/wiki/purpose",
    request_body = SetPurposeRequest,
    responses((status = 204)))]
pub async fn set_purpose(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<SetPurposeRequest>,
) -> Result<StatusCode, ApiError> {
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    svc(&state)
        .set_purpose(
            lib,
            &Purpose {
                goals: req.goals,
                key_questions: req.key_questions,
                scope: req.scope,
                thesis: req.thesis,
            },
        )
        .await
        .map_err(we)?;
    Ok(StatusCode::NO_CONTENT)
}

//! Wiki 域端点（wiki scope）。

use engram_core::wiki::{CascadeReport, InsightsReport, Purpose, ReviewItem};
use engram_core::wiki::{LintReport, WikiError, WikiPageDto, WikiService};
use engram_jobs::types::JobEvent;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::IntoParams;

use crate::auth::{Principal, require_scope};
use crate::error::ApiError;
use crate::state::AppState;

fn require_wiki(p: &Principal) -> Result<(), ApiError> {
    require_scope(p, "wiki")
}

fn we(e: WikiError) -> ApiError {
    match e {
        WikiError::NotFound(m) => ApiError::NotFound(m),
        WikiError::BadRequest(m) => ApiError::BadRequest(m),
        WikiError::Storage(m) => ApiError::Unavailable(m),
    }
}

fn svc(state: &AppState) -> WikiService {
    WikiService::new(state.pool.clone(), state.registry())
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct IngestRequest {
    pub title: String,
    /// 源文本（也可通过 knowledge 文档 ID）
    pub text: Option<String>,
    /// knowledge 文档 ID（二选一）
    pub document_id: Option<uuid::Uuid>,
}

/// 触发两步 ingest（sha 命中秒跳过，返回 skipped=true）。
#[utoipa::path(post, path = "/wiki/ingest",
    request_body = IngestRequest,
    responses((status = 202, body = IngestAccepted)))]
pub async fn ingest(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<IngestRequest>,
) -> Result<(StatusCode, Json<IngestAccepted>), ApiError> {
    require_wiki(&principal)?;
    let skipped = match (req.text, req.document_id) {
        (Some(text), _) => svc(&state).ingest(&req.title, &text).await.map_err(we)?,
        (None, Some(doc_id)) => svc(&state).ingest_document(doc_id).await.map_err(we)?,
        (None, None) => {
            return Err(ApiError::BadRequest(
                "text 与 document_id 必须提供其一".into(),
            ));
        }
    };
    Ok((StatusCode::ACCEPTED, Json(IngestAccepted { skipped })))
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct IngestAccepted {
    pub skipped: bool,
}

#[derive(Deserialize, IntoParams)]
pub struct ListPagesParams {
    pub page_type: Option<String>,
    pub limit: Option<i64>,
}

#[utoipa::path(get, path = "/wiki/pages", params(ListPagesParams),
    responses((status = 200, body = [WikiPageDto])))]
pub async fn list_pages(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ListPagesParams>,
) -> Result<Json<Vec<WikiPageDto>>, ApiError> {
    require_wiki(&principal)?;
    Ok(Json(
        svc(&state)
            .list_pages(p.page_type.as_deref(), p.limit.unwrap_or(100))
            .await
            .map_err(we)?,
    ))
}

#[utoipa::path(get, path = "/wiki/pages/{slug}",
    responses((status = 200, body = WikiPageDto)))]
pub async fn get_page(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<WikiPageDto>, ApiError> {
    require_wiki(&principal)?;
    Ok(Json(svc(&state).get_page(&slug).await.map_err(we)?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct PutPageRequest {
    pub title: String,
    pub content: String,
    /// 目录树文件夹（可选；None = 保持原值，Obsidian 式 / 分隔多级路径）
    #[serde(default)]
    pub folder: Option<String>,
    /// 执行者标记（S-7）：AI 代用户执行时传 "ai"——落 frontmatter.via 区分真人编辑与 AI 代执行；
    /// Web 用户编辑不传。
    #[serde(default)]
    pub via: Option<String>,
}

/// 人工编辑（origin=human，版本递增；LLM 后续只提案不覆盖）。
#[utoipa::path(put, path = "/wiki/pages/{slug}",
    request_body = PutPageRequest,
    responses((status = 200, body = WikiPageDto)))]
pub async fn put_page(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<PutPageRequest>,
) -> Result<Json<WikiPageDto>, ApiError> {
    require_wiki(&principal)?;
    Ok(Json(
        svc(&state)
            .put_page(
                &slug,
                &req.title,
                &req.content,
                req.folder.as_deref(),
                req.via.as_deref(),
            )
            .await
            .map_err(we)?,
    ))
}

#[utoipa::path(get, path = "/wiki/graph",
    responses((status = 200, body = engram_core::wiki::GraphDto)))]
pub async fn graph(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<engram_core::wiki::GraphDto>, ApiError> {
    require_wiki(&principal)?;
    Ok(Json(svc(&state).graph().await.map_err(we)?))
}

/// Lint（只报告）。
#[utoipa::path(post, path = "/wiki/lint", responses((status = 200, body = LintReport)))]
pub async fn lint(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<LintReport>, ApiError> {
    require_wiki(&principal)?;
    Ok(Json(svc(&state).lint().await.map_err(we)?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ApplyProposalRequest {
    pub slug: String,
    pub title: String,
    pub content: String,
    /// 执行者标记（S-7）：AI 代用户执行时传 "ai"——落 frontmatter.via
    #[serde(default)]
    pub via: Option<String>,
}

/// 人审合入提案。
#[utoipa::path(post, path = "/wiki/proposals/apply",
    request_body = ApplyProposalRequest,
    responses((status = 200, body = WikiPageDto)))]
pub async fn apply_proposal(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<ApplyProposalRequest>,
) -> Result<Json<WikiPageDto>, ApiError> {
    require_wiki(&principal)?;
    Ok(Json(
        svc(&state)
            .apply_proposal(&req.slug, &req.content, &req.title, req.via.as_deref())
            .await
            .map_err(we)?,
    ))
}

/// 待审提案聚合——一条 SQL 取回全部 wiki_generate 任务的最新提案事件，
/// 替代前端 jobs + 逐 job events 的 N+1 请求。
#[utoipa::path(get, path = "/wiki/proposals", responses((status = 200, body = [JobEvent])))]
pub async fn list_proposals(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<JobEvent>>, ApiError> {
    require_wiki(&principal)?;
    let rows = sqlx::query_as::<_, JobEvent>(
        r#"SELECT t.* FROM (
             SELECT DISTINCT ON (e.job_id) e.*
             FROM jobs j JOIN job_events e ON e.job_id = j.id
             WHERE j.kind = 'wiki_generate' AND e.message LIKE '%提案%'
             ORDER BY e.job_id, e.id DESC
           ) t ORDER BY t.id DESC LIMIT 50"#,
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    Ok(Json(rows))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct WikiSearchRequest {
    pub query: String,
    pub max_items: Option<i64>,
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
    Ok(Json(
        svc(&state)
            .search_with_purpose(&req.query, req.max_items.unwrap_or(20))
            .await
            .map_err(we)?,
    ))
}

// ---------- purpose ----------

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

/// 读取 purpose（wiki 方向意图）。
#[utoipa::path(get, path = "/wiki/purpose", responses((status = 200, body = Purpose)))]
pub async fn get_purpose(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Option<Purpose>>, ApiError> {
    require_wiki(&principal)?;
    Ok(Json(svc(&state).get_purpose().await.map_err(we)?))
}

/// 设置 purpose（ingest/query 时注入 LLM）。
#[utoipa::path(put, path = "/wiki/purpose",
    request_body = SetPurposeRequest,
    responses((status = 204)))]
pub async fn set_purpose(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<SetPurposeRequest>,
) -> Result<StatusCode, ApiError> {
    require_wiki(&principal)?;
    svc(&state)
        .set_purpose(&Purpose {
            goals: req.goals,
            key_questions: req.key_questions,
            scope: req.scope,
            thesis: req.thesis,
        })
        .await
        .map_err(we)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- review ----------

#[utoipa::path(get, path = "/wiki/reviews", responses((status = 200, body = [ReviewItem])))]
pub async fn list_reviews(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ReviewItem>>, ApiError> {
    require_wiki(&principal)?;
    Ok(Json(svc(&state).reviews().await.map_err(we)?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ResolveReviewRequest {
    pub action: Option<String>,
    #[serde(default)]
    pub dismiss: bool,
}

/// 处理 review 项（resolve / dismiss + 动作标签）。
#[utoipa::path(post, path = "/wiki/reviews/{id}/resolve",
    request_body = ResolveReviewRequest,
    responses((status = 204)))]
pub async fn resolve_review(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<ResolveReviewRequest>,
) -> Result<StatusCode, ApiError> {
    require_wiki(&principal)?;
    svc(&state)
        .review_resolve(id, req.action.as_deref(), req.dismiss)
        .await
        .map_err(we)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- queries 存档 ----------

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ArchiveQueryRequest {
    pub title: String,
    pub question: String,
    pub answer: String,
}

/// 检索结果/问答存档为 queries 页并自动再摄取。
#[utoipa::path(post, path = "/wiki/queries/archive",
    request_body = ArchiveQueryRequest,
    responses((status = 202, body = IngestAccepted)))]
pub async fn archive_query(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<ArchiveQueryRequest>,
) -> Result<(StatusCode, Json<IngestAccepted>), ApiError> {
    require_wiki(&principal)?;
    let skipped = svc(&state)
        .archive_query(&req.title, &req.question, &req.answer)
        .await
        .map_err(we)?;
    Ok((StatusCode::ACCEPTED, Json(IngestAccepted { skipped })))
}

// ---------- sources（级联删除） ----------

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct WikiSourceDto {
    pub id: uuid::Uuid,
    pub title: Option<String>,
    pub status: String,
}

#[utoipa::path(get, path = "/wiki/sources", responses((status = 200, body = [WikiSourceDto])))]
pub async fn list_sources(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<WikiSourceDto>>, ApiError> {
    require_wiki(&principal)?;
    let rows = svc(&state).list_sources().await.map_err(we)?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, title, status, _)| WikiSourceDto { id, title, status })
            .collect(),
    ))
}

/// 级联删除 source（摘要页删 + 共享页摘源 + dead link 清理 + index 同步）。
#[utoipa::path(delete, path = "/wiki/sources/{id}",
    responses((status = 200, body = CascadeReport)))]
pub async fn delete_source(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<CascadeReport>, ApiError> {
    require_wiki(&principal)?;
    Ok(Json(
        svc(&state).delete_source_cascade(id).await.map_err(we)?,
    ))
}

// ---------- 图洞察 ----------

/// 图洞察（意外连接/孤立页/稀疏社区/桥节点）+ 社区信息。
#[utoipa::path(post, path = "/wiki/insights", responses((status = 200, body = InsightsReport)))]
pub async fn insights(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<InsightsReport>, ApiError> {
    require_wiki(&principal)?;
    Ok(Json(svc(&state).insights().await.map_err(we)?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct DismissInsightRequest {
    pub key: String,
}

/// dismiss 洞察（不再出现）。
#[utoipa::path(post, path = "/wiki/insights/dismiss",
    request_body = DismissInsightRequest,
    responses((status = 204)))]
pub async fn dismiss_insight(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<DismissInsightRequest>,
) -> Result<StatusCode, ApiError> {
    require_wiki(&principal)?;
    svc(&state).insight_dismiss(&req.key).await.map_err(we)?;
    Ok(StatusCode::NO_CONTENT)
}

/// 重置全部 dismiss。
#[utoipa::path(post, path = "/wiki/insights/reset", responses((status = 204)))]
pub async fn reset_insights(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    require_wiki(&principal)?;
    svc(&state).insight_reset().await.map_err(we)?;
    Ok(StatusCode::NO_CONTENT)
}

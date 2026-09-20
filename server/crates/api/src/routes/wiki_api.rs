//! Wiki 域端点（wiki scope）。单库终局（2026-09-20）：多库 API 已移除——
//! 无 `?lib=` 参数、无库管理端点，全部请求恒定落在 main 主库。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use engram_core::wiki::libraries;
use engram_core::wiki::{CascadeReport, InsightsReport, Purpose, ReviewItem};
use engram_core::wiki::{LintReport, WikiError, WikiPageDto, WikiService};
use engram_jobs::types::JobEvent;
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

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
        WikiError::Storage(m) => {
            tracing::error!("wiki 存储错误（对外 503 unavailable）: {m}");
            ApiError::Unavailable(m)
        }
    }
}

fn pe(e: engram_core::promote::PromoteError) -> ApiError {
    use engram_core::promote::PromoteError;
    match e {
        PromoteError::NotFound(m) => ApiError::NotFound(m),
        PromoteError::Conflict(m) => ApiError::Conflict(m),
        PromoteError::BadRequest(m) => ApiError::BadRequest(m),
        PromoteError::Storage(m) => ApiError::Unavailable(m.to_string()),
        PromoteError::Wiki(m) => we(m),
    }
}

fn svc(state: &AppState) -> WikiService {
    WikiService::new(state.pool.clone(), state.registry()).with_llm(state.llm())
}

/// 单库终局：main 主库 id（内部解析，无外部入参）。
async fn main_lib(state: &AppState) -> Result<uuid::Uuid, ApiError> {
    libraries::resolve(&state.pool, None).await.map_err(we)
}

// ---------- ingest ----------

#[derive(Deserialize, utoipa::ToSchema)]
pub struct IngestRequest {
    pub title: String,
    /// 源文本（也可通过 wiki 文档 ID）
    pub text: Option<String>,
    /// wiki 文档 ID（二选一）
    pub document_id: Option<uuid::Uuid>,
}

/// 触发两步 ingest（D27 三态：skipped 仅表示同内容曾成功织入；
/// in_flight=同内容任务处理中（勿重提也非丢失）；enqueued=新入队）。
#[utoipa::path(post, path = "/wiki/ingest",
    request_body = IngestRequest,
    responses((status = 202, body = IngestAccepted)))]
pub async fn ingest(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<IngestRequest>,
) -> Result<(StatusCode, Json<IngestAccepted>), ApiError> {
    use engram_core::wiki::IngestOutcome;
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    let outcome = match (req.text, req.document_id) {
        (Some(text), _) => svc(&state)
            .ingest(lib, &req.title, &text)
            .await
            .map_err(we)?,
        (None, Some(doc_id)) => svc(&state).ingest_document(lib, doc_id).await.map_err(we)?,
        (None, None) => {
            return Err(ApiError::BadRequest(
                "text 与 document_id 必须提供其一".into(),
            ));
        }
    };
    let (status, skipped) = match &outcome {
        IngestOutcome::AlreadyReady(_) => ("ready", true),
        IngestOutcome::InFlight(_, _) => ("in_flight", false),
        IngestOutcome::Enqueued(_, _) => ("enqueued", false),
    };
    Ok((
        StatusCode::ACCEPTED,
        Json(IngestAccepted {
            skipped,
            status: Some(status.into()),
            source_id: Some(outcome.source_id()),
            job_id: outcome.job_id(),
        }),
    ))
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct IngestAccepted {
    /// 仅「同内容曾成功织入」为 true；in_flight/enqueued 均为 false
    pub skipped: bool,
    /// ready | in_flight | enqueued（仅 /wiki/ingest 返回；queries/archive 无此字段）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// wiki_sources 行 id（任务页/审计追踪用；仅 /wiki/ingest 返回）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<uuid::Uuid>,
    /// 织入任务 id（GET /jobs/{job_id} 直查进度；仅 /wiki/ingest 返回）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<uuid::Uuid>,
}

#[derive(Deserialize, IntoParams)]
pub struct ListPagesParams {
    pub page_type: Option<String>,
    /// keyset 分页游标：{updated_at ISO8601}|{id}（上一页最后一条）
    pub cursor: Option<String>,
    /// 返回条数上限；不传 = 全量（2026-09-20 单库终局：不要截断）
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
    let lib = main_lib(&state).await?;
    Ok(Json(
        svc(&state)
            .list_pages(lib, p.page_type.as_deref(), p.limit, p.cursor.as_deref())
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
    let lib = main_lib(&state).await?;
    Ok(Json(svc(&state).get_page(lib, &slug).await.map_err(we)?))
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
    let lib = main_lib(&state).await?;
    Ok(Json(
        svc(&state)
            .put_page(
                lib,
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

/// Lint（只报告）。
#[utoipa::path(post, path = "/wiki/lint",
    responses((status = 200, body = LintReport)))]
pub async fn lint(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<LintReport>, ApiError> {
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    Ok(Json(svc(&state).lint(lib).await.map_err(we)?))
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

/// Repair：lint 修而不只报（wiki 收录哲学线工单③）——确定性修复：
/// 变体死链改写 / 死链去链接化 / ≥3 页引用建 stub / 孤页沿出链回挂 / 同标题重复合并（快照兜底）。
#[utoipa::path(post, path = "/wiki/repair",
    responses((status = 200, body = engram_wiki_engine::repair::RepairReport)))]
pub async fn repair(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<engram_wiki_engine::repair::RepairReport>, ApiError> {
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    Ok(Json(svc(&state).repair(lib).await.map_err(we)?))
}

/// Repair 异步入队（批次⑦ job 化，wiki 大库化）——确定性修复走 jobs 基建，任务页可查历史。
#[utoipa::path(post, path = "/wiki/repair/async", operation_id = "wiki_repair_async",
    responses((status = 202)))]
pub async fn repair_async(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), ApiError> {
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    let job_id = engram_wiki_engine::repair::enqueue(&state.pool, lib)
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(serde_json::json!({"job_id": job_id, "note": "修复已入队——任务页可查进度与历史"})),
    ))
}

/// Merge：新陈代谢合并原语（工单④）——duplicate 并入 primary（冗余丢弃或内容并入），
/// 全库链接改指，delete_page 快照兜底（下架不烧书）。AI 处置重复 flag 与人工逃生门共用。
#[derive(Deserialize, utoipa::ToSchema)]
pub struct MergePagesRequest {
    pub primary: String,
    pub duplicate: String,
}

#[utoipa::path(post, path = "/wiki/pages/merge",
    request_body = MergePagesRequest,
    responses((status = 200)))]
pub async fn merge_pages(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<MergePagesRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    let detail = svc(&state)
        .merge_pages(lib, &req.primary, &req.duplicate)
        .await
        .map_err(we)?;
    Ok(Json(serde_json::json!({ "detail": detail })))
}

/// 规模化 task-4：重复候选聚合出口——标题归一化相同的页面组。
/// 研判流：候选 → 逐组 AI/人工研判 → merge_pages 合并（留痕）或确认共存。
#[utoipa::path(get, path = "/wiki/duplicates",
    responses((status = 200, body = Object)))]
pub async fn duplicates(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    let candidates = svc(&state).duplicate_candidates(lib).await.map_err(we)?;
    Ok(Json(serde_json::json!({ "candidates": candidates })))
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
    let lib = main_lib(&state).await?;
    Ok(Json(
        svc(&state)
            .apply_proposal(lib, &req.slug, &req.content, &req.title, req.via.as_deref())
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
    let rows = engram_jobs::admin::latest_wiki_proposals(&state.pool)
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    Ok(Json(rows))
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

// ---------- review ----------

#[utoipa::path(get, path = "/wiki/reviews",
    responses((status = 200, body = [ReviewItem])))]
pub async fn list_reviews(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ReviewItem>>, ApiError> {
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    Ok(Json(svc(&state).reviews(lib, None).await.map_err(we)?))
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
    let lib = main_lib(&state).await?;
    let skipped = svc(&state)
        .archive_query(lib, &req.title, &req.question, &req.answer)
        .await
        .map_err(we)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(IngestAccepted {
            skipped,
            status: None,
            source_id: None,
            job_id: None,
        }),
    ))
}

// ---------- sources（级联删除） ----------

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct WikiSourceDto {
    pub id: uuid::Uuid,
    pub title: Option<String>,
    pub status: String,
}

#[utoipa::path(get, path = "/wiki/sources",
    responses((status = 200, body = [WikiSourceDto])))]
pub async fn list_sources(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<WikiSourceDto>>, ApiError> {
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    let rows = svc(&state).list_sources(lib).await.map_err(we)?;
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
    let lib = main_lib(&state).await?;
    Ok(Json(
        svc(&state)
            .delete_source_cascade(lib, id)
            .await
            .map_err(we)?,
    ))
}

// ---------- 图洞察 ----------

/// 图洞察（意外连接/孤立页/稀疏社区/桥节点）+ 社区信息。
#[utoipa::path(post, path = "/wiki/insights",
    responses((status = 200, body = InsightsReport)))]
pub async fn insights(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<InsightsReport>, ApiError> {
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    Ok(Json(svc(&state).insights(lib).await.map_err(we)?))
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
    let lib = main_lib(&state).await?;
    svc(&state)
        .insight_dismiss(lib, &req.key)
        .await
        .map_err(we)?;
    Ok(StatusCode::NO_CONTENT)
}

/// 重置全部 dismiss。
#[utoipa::path(post, path = "/wiki/insights/reset",
    responses((status = 204)))]
pub async fn reset_insights(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    svc(&state).insight_reset(lib).await.map_err(we)?;
    Ok(StatusCode::NO_CONTENT)
}

/// 删除页面（连带清理双向 wikilinks；不可逆）。
#[utoipa::path(delete, path = "/wiki/pages/{slug}",
    responses((status = 204), (status = 404, body = crate::error::ErrorEnvelope)))]
pub async fn delete_page(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    axum::extract::Path(slug): axum::extract::Path<String>,
) -> Result<axum::http::StatusCode, ApiError> {
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    svc(&state).delete_page(lib, &slug).await.map_err(we)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// 存量回填：重析全部页面正文重建 wiki_links（D4 存量修复；幂等）。
#[utoipa::path(post, path = "/wiki/links/rebuild",
    responses((status = 200, body = Object)))]
pub async fn rebuild_links(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    let n = svc(&state).rebuild_all_links(lib).await.map_err(we)?;
    Ok(Json(serde_json::json!({ "rebuilt_links": n })))
}

/// 存量内容页 tsv 重刷（EN-63）：slug+title+content、wiki 分词变体；排除 index/log/overview
/// 系统页（结构页不参与 FTS——重刷包含会让系统页霸榜）。对齐 rebuild_links 先例；幂等。
#[utoipa::path(post, path = "/wiki/tsv/rebuild",
    responses((status = 200, body = Object)))]
pub async fn rebuild_tsv(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_wiki(&principal)?;
    let lib = main_lib(&state).await?;
    let n = svc(&state).backfill_tsv(lib).await.map_err(we)?;
    Ok(Json(serde_json::json!({ "rebuilt_tsv": n })))
}

// ---------- 知识晋升（EN-59）：项目文档 → wiki 的结构化动作；只读列表对齐 MCP ----------

/// 晋升请求体。
#[derive(Deserialize, utoipa::ToSchema)]
pub struct PromoteRequest {
    /// 来源项目（名或 id）
    pub project: String,
    /// 来源文档 id
    pub doc_id: Uuid,
    /// 源定位（小节标题/行区间说明）
    #[serde(default)]
    pub anchor: String,
    /// 目标页 slug
    pub slug: String,
    /// 页标题（提炼后的通用标题）
    pub title: String,
    /// 提炼后的通用知识正文（markdown）——提炼由调用方完成
    pub content: String,
}

/// 知识晋升（EN-59）：把项目文档里的一条跨项目知识提炼成 wiki synthesis 页。
/// 服务端自动双向回链：页 frontmatter 带 promoted_from + 源文档追加 ⛳ 标记 + 登记表。
#[utoipa::path(post, path = "/wiki/promote", request_body = PromoteRequest,
    responses((status = 200, body = Object), (status = 404), (status = 409)))]
pub async fn promote(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<PromoteRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_wiki(&principal)?;
    let out = engram_core::promote::PromoteService::new(state.pool.clone())
        .promote(engram_core::promote::PromoteRequest {
            project: req.project,
            doc_id: req.doc_id,
            anchor: req.anchor,
            slug: req.slug,
            title: req.title,
            content: req.content,
            // 单库终局：晋升页恒落 main 主库
            library: None,
        })
        .await
        .map_err(pe)?;
    Ok(Json(serde_json::json!({
        "promoted": true,
        "library": out.library,
        "page_slug": out.page_slug,
        "page_title": out.page_title,
        "project": out.project_name,
    })))
}

/// 晋升登记列表（可选按来源项目名/id 过滤）。
#[utoipa::path(get, path = "/wiki/promotions", params(PromotionsParams),
    responses((status = 200, body = [engram_storage::models::wiki_promotions::WikiPromotionDto])))]
pub async fn promotions(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<PromotionsParams>,
) -> Result<Json<Vec<engram_storage::models::wiki_promotions::WikiPromotionDto>>, ApiError> {
    require_wiki(&principal)?;
    let rows = engram_core::promote::PromoteService::new(state.pool.clone())
        .list_promotions(p.project.as_deref())
        .await
        .map_err(pe)?;
    Ok(Json(rows))
}

#[derive(Deserialize, IntoParams)]
pub struct PromotionsParams {
    /// 可选：按来源项目（名或 id）过滤
    pub project: Option<String>,
}

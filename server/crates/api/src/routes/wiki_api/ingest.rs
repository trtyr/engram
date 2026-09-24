//! `wiki_api` 的实现切片（架构治理 2026-09-21：自 wiki_api.rs 纯搬移，零行为变化）。

use super::*;

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
    require_wiki_read(&principal)?;
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
    let report = svc(&state)
        .delete_source_cascade(lib, id)
        .await
        .map_err(we)?;
    // 级联删页会留下死链/孤页（2026-09-20 实测：删 4 份原料立刻冒出 dead_link 19 / orphan 28，
    // 当时靠事后手跑 repair 才收口）——删完自动补一次 repair，AI 代管、用户零操作。
    // repair 是确定性修复（不调 LLM）且重复跑无害，故不设幂等键（删原料本就是低频手动操作）。
    match engram_wiki_engine::repair::enqueue(&state.pool, lib).await {
        Ok(job_id) => tracing::info!(%job_id, source_id = %id, "删原料后已自动入队 wiki_repair"),
        Err(e) => tracing::warn!(
            error = %e,
            source_id = %id,
            "删原料后自动 repair 入队失败——可手动 POST /wiki/repair/async 补跑"
        ),
    }
    Ok(Json(report))
}

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
    require_wiki_read(&principal)?;
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

//! `wiki_api` 的实现切片（架构治理 2026-09-21：自 wiki_api.rs 纯搬移，零行为变化）。

use super::*;

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

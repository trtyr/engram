//! `wiki_api` 的实现切片（架构治理 2026-09-21：自 wiki_api.rs 纯搬移，零行为变化）。

use super::*;

pub(crate) fn we(e: WikiError) -> ApiError {
    match e {
        WikiError::NotFound(m) => ApiError::NotFound(m),
        WikiError::BadRequest(m) => ApiError::BadRequest(m),
        WikiError::Storage(m) => {
            tracing::error!("wiki 存储错误（对外 503 unavailable）: {m}");
            ApiError::Unavailable(m)
        }
    }
}

pub(crate) fn pe(e: engram_core::promote::PromoteError) -> ApiError {
    use engram_core::promote::PromoteError;
    match e {
        PromoteError::NotFound(m) => ApiError::NotFound(m),
        PromoteError::Conflict(m) => ApiError::Conflict(m),
        PromoteError::BadRequest(m) => ApiError::BadRequest(m),
        PromoteError::Storage(m) => ApiError::Unavailable(m.to_string()),
        PromoteError::Wiki(m) => we(m),
    }
}

/// 单库终局：main 主库 id（内部解析，无外部入参）。
pub(crate) async fn main_lib(state: &AppState) -> Result<uuid::Uuid, ApiError> {
    libraries::resolve(&state.pool, None).await.map_err(we)
}

#[derive(Deserialize, IntoParams)]
pub struct ListPagesParams {
    pub page_type: Option<String>,
    /// 目录子树过滤：folder 精确等于该路径或其下级（folder LIKE '路径/%'）——
    /// 规模化（2026-09-20）：前端树按 folder 懒加载，不再一次拉全库。
    pub folder: Option<String>,
    /// keyset 分页游标：{updated_at ISO8601}|{id}（上一页最后一条）
    pub cursor: Option<String>,
    /// 返回条数上限；不传 = 全量（2026-09-20 单库终局：不要截断）
    pub limit: Option<i64>,
}

#[utoipa::path(get, path = "/wiki/pages", params(ListPagesParams),
    responses((status = 200, body = [WikiPageMetaDto])))]
pub async fn list_pages(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ListPagesParams>,
) -> Result<Json<Vec<WikiPageMetaDto>>, ApiError> {
    require_wiki_read(&principal)?;
    let lib = main_lib(&state).await?;
    Ok(Json(
        svc(&state)
            .list_pages(
                lib,
                p.page_type.as_deref(),
                p.folder.as_deref(),
                p.limit,
                p.cursor.as_deref(),
            )
            .await
            .map_err(we)?,
    ))
}

/// 目录骨架索引：(folder, 页数) 全量——几百行量级，供前端懒加载树先渲染结构。
#[utoipa::path(get, path = "/wiki/folders", responses((status = 200, body = Vec<(String, i64)>)))]
pub async fn list_folders(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<(String, i64)>>, ApiError> {
    require_wiki_read(&principal)?;
    let lib = main_lib(&state).await?;
    Ok(Json(svc(&state).list_folders(lib).await.map_err(we)?))
}

#[utoipa::path(get, path = "/wiki/pages/{slug}",
    responses((status = 200, body = WikiPageDto)))]
pub async fn get_page(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<WikiPageDto>, ApiError> {
    require_wiki_read(&principal)?;
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
    require_wiki_read(&principal)?;
    let lib = main_lib(&state).await?;
    let candidates = svc(&state).duplicate_candidates(lib).await.map_err(we)?;
    Ok(Json(serde_json::json!({ "candidates": candidates })))
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

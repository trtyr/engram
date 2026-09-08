//! 文档知识端点（wiki scope）。
//!
//! 多库（0037）：全部端点接受可选 query 参数 `?lib=<库slug>`（缺省 main 主库），
//! 统一解析成库 id 后透传到服务/仓储层，读写均收窄到该库。

use axum::Json;
use axum::extract::multipart::Multipart;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use engram_core::wiki::WikiError;
use engram_core::wiki::libraries;
use engram_core::wiki_docs::{
    ChunkHit, DocumentDto, IngestSource, WikiDocumentError, WikiDocumentService,
};
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::auth::{Principal, require_scope};
use crate::error::ApiError;
use crate::state::AppState;

fn require_wiki_docs(p: &Principal) -> Result<(), ApiError> {
    require_scope(p, "wiki")
}

fn ke(e: WikiDocumentError) -> ApiError {
    match e {
        WikiDocumentError::NotFound(m) => ApiError::NotFound(m),
        WikiDocumentError::BadRequest(m) => ApiError::BadRequest(m),
        WikiDocumentError::Storage(m) => ApiError::Unavailable(m),
    }
}

fn svc(state: &AppState) -> WikiDocumentService {
    WikiDocumentService::new(state.pool.clone(), state.registry(), state.data_dir.clone())
}

/// `?lib=<库slug>` 提取（多库路由，缺省 main）。
#[derive(Deserialize, IntoParams)]
pub struct LibQuery {
    /// 库 slug（缺省 main 主库）
    pub lib: Option<String>,
}

/// slug → 库 id：None/空取 main；未命中按 NotFound 拒绝。
/// （复用 wiki-engine 的 libraries::resolve——与 wiki_api 同一实现，api src 层零 SQL）
async fn resolve_lib(state: &AppState, lib: Option<&str>) -> Result<Uuid, ApiError> {
    libraries::resolve(&state.pool, lib)
        .await
        .map_err(|e| match e {
            WikiError::NotFound(m) => ApiError::NotFound(m),
            WikiError::BadRequest(m) => ApiError::BadRequest(m),
            WikiError::Storage(m) => ApiError::Unavailable(m),
        })
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct SubmitUrlRequest {
    pub url: String,
}

/// 提交 URL 摄取（SSRF 防护在管道内）。
#[utoipa::path(post, path = "/wiki/documents",
    request_body(content = SubmitUrlRequest, content_type = "application/json"),
    params(LibQuery),
    responses((status = 201, body = DocumentDto)))]
pub async fn submit_url(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(q): Query<LibQuery>,
    Json(req): Json<SubmitUrlRequest>,
) -> Result<(StatusCode, Json<DocumentDto>), ApiError> {
    require_wiki_docs(&principal)?;
    let lib = resolve_lib(&state, q.lib.as_deref()).await?;
    // wiki-engine 并行改造中：submit 增加 lib 首参（库隔离）
    let (id, deduped) = svc(&state)
        .submit(lib, IngestSource::Url(req.url))
        .await
        .map_err(ke)?;
    let doc = svc(&state).get_document(lib, id).await.map_err(ke)?;
    if deduped {
        Ok((StatusCode::OK, Json(doc))) // 幂等命中返回 200
    } else {
        Ok((StatusCode::CREATED, Json(doc)))
    }
}

/// 上传文件摄取（multipart，字段名 file）。
#[utoipa::path(post, path = "/wiki/upload",
    request_body(content = Vec<u8>, content_type = "multipart/form-data"),
    params(LibQuery),
    responses((status = 201, body = DocumentDto)))]
pub async fn upload(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(q): Query<LibQuery>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<DocumentDto>), ApiError> {
    require_wiki_docs(&principal)?;
    let lib = resolve_lib(&state, q.lib.as_deref()).await?;
    let mut name = None;
    let mut content = None;
    let mut content_type = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("multipart 解析失败: {e}")))?
    {
        match field.name() {
            Some("file") => {
                name = Some(
                    field
                        .file_name()
                        .map(String::from)
                        .unwrap_or_else(|| "upload.bin".into()),
                );
                content_type = field.content_type().map(String::from);
                content = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| ApiError::BadRequest(format!("读文件失败: {e}")))?
                        .to_vec(),
                );
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }
    let (Some(name), Some(content)) = (name, content) else {
        return Err(ApiError::BadRequest("缺少 file 字段".into()));
    };
    // wiki-engine 并行改造中：submit 增加 lib 首参（库隔离）
    let (id, deduped) = svc(&state)
        .submit(
            lib,
            IngestSource::Bytes {
                name,
                content,
                content_type,
            },
        )
        .await
        .map_err(ke)?;
    let doc = svc(&state).get_document(lib, id).await.map_err(ke)?;
    if deduped {
        Ok((StatusCode::OK, Json(doc))) // 幂等命中返回 200
    } else {
        Ok((StatusCode::CREATED, Json(doc)))
    }
}

#[derive(Deserialize, IntoParams)]
pub struct ListDocsParams {
    /// 库 slug（缺省 main 主库）
    pub lib: Option<String>,
    pub status: Option<String>,
    pub cursor: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<i64>,
}

#[utoipa::path(get, path = "/wiki/documents", params(ListDocsParams),
    responses((status = 200, body = [DocumentDto])))]
pub async fn list_documents(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ListDocsParams>,
) -> Result<Json<Vec<DocumentDto>>, ApiError> {
    require_wiki_docs(&principal)?;
    let lib = resolve_lib(&state, p.lib.as_deref()).await?;
    // wiki-engine 并行改造中：list_documents 增加 lib 首参（库隔离）
    Ok(Json(
        svc(&state)
            .list_documents(lib, p.status.as_deref(), p.cursor, p.limit.unwrap_or(50))
            .await
            .map_err(ke)?,
    ))
}

#[utoipa::path(get, path = "/wiki/documents/{id}", params(LibQuery),
    responses((status = 200, body = DocumentDto)))]
pub async fn get_document(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<LibQuery>,
) -> Result<Json<DocumentDto>, ApiError> {
    require_wiki_docs(&principal)?;
    let lib = resolve_lib(&state, q.lib.as_deref()).await?;
    // wiki-engine 并行改造中：get_document 增加 lib 首参（库隔离）
    Ok(Json(svc(&state).get_document(lib, id).await.map_err(ke)?))
}

#[utoipa::path(get, path = "/wiki/documents/{id}/chunks", params(LibQuery),
    responses((status = 200, body = [(i32, String, bool)])))]
pub async fn document_chunks(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<LibQuery>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    require_wiki_docs(&principal)?;
    let lib = resolve_lib(&state, q.lib.as_deref()).await?;
    // wiki-engine 并行改造中：chunks 增加 lib 首参（库隔离）
    let chunks = svc(&state).chunks(lib, id, 500).await.map_err(ke)?;
    Ok(Json(
        chunks
            .into_iter()
            .map(|(seq, content, failed)| {
                serde_json::json!({
                    "seq": seq,
                    "content": content.chars().take(300).collect::<String>(),
                    "embed_failed": failed,
                })
            })
            .collect(),
    ))
}

#[utoipa::path(delete, path = "/wiki/documents/{id}", params(LibQuery),
    responses((status = 204)))]
pub async fn delete_document(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<LibQuery>,
) -> Result<StatusCode, ApiError> {
    require_wiki_docs(&principal)?;
    let lib = resolve_lib(&state, q.lib.as_deref()).await?;
    // wiki-engine 并行改造中：delete 增加 lib 首参（库隔离）
    svc(&state).delete(lib, id).await.map_err(ke)?;
    Ok(StatusCode::NO_CONTENT)
}

/// 重新嵌入缺失块（embed_failed / NULL 向量的显式恢复入口，K8）。
#[utoipa::path(post, path = "/wiki/documents/{id}/re-embed", params(LibQuery),
    responses((status = 202, description = "补嵌 job 已入队")))]
pub async fn reembed(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<LibQuery>,
) -> Result<StatusCode, ApiError> {
    require_wiki_docs(&principal)?;
    let lib = resolve_lib(&state, q.lib.as_deref()).await?;
    // wiki-engine 并行改造中：reembed 增加 lib 首参（库隔离）
    svc(&state).reembed(lib, id).await.map_err(ke)?;
    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct WikiDocumentSearchRequest {
    pub query: String,
    pub max_items: Option<i64>,
}

/// 知识混合检索（结果带文档引用 + 高亮片段）。
#[utoipa::path(post, path = "/wiki/documents/search", operation_id = "wiki_docs_search",
    request_body = WikiDocumentSearchRequest,
    params(LibQuery),
    responses((status = 200, body = [ChunkHit])))]
pub async fn search(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(q): Query<LibQuery>,
    Json(req): Json<WikiDocumentSearchRequest>,
) -> Result<Json<Vec<ChunkHit>>, ApiError> {
    require_wiki_docs(&principal)?;
    let lib = resolve_lib(&state, q.lib.as_deref()).await?;
    // W-3（2026-09-04）：空 query 三问拒绝——与 /search 的 400 口径对齐，不再返回全量
    if req.query.trim().is_empty() {
        return Err(ApiError::BadRequest("query 不能为空".into()));
    }
    // wiki-engine 并行改造中：search 增加 lib 首参（库隔离）
    Ok(Json(
        svc(&state)
            .search(lib, &req.query, req.max_items.unwrap_or(20).min(100))
            .await
            .map_err(ke)?,
    ))
}

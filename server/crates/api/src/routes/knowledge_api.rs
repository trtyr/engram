//! 知识域端点（knowledge scope）。

use agent_memory_core::knowledge::{
    ChunkHit, DocumentDto, IngestSource, KnowledgeError, KnowledgeService,
};
use axum::Json;
use axum::extract::multipart::Multipart;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::auth::{Principal, require_scope};
use crate::error::ApiError;
use crate::state::AppState;

fn require_knowledge(p: &Principal) -> Result<(), ApiError> {
    require_scope(p, "knowledge")
}

fn ke(e: KnowledgeError) -> ApiError {
    match e {
        KnowledgeError::NotFound(m) => ApiError::NotFound(m),
        KnowledgeError::BadRequest(m) => ApiError::BadRequest(m),
        KnowledgeError::Storage(m) => ApiError::Unavailable(m),
    }
}

fn svc(state: &AppState) -> KnowledgeService {
    KnowledgeService::new(state.pool.clone(), state.registry(), state.data_dir.clone())
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct SubmitUrlRequest {
    pub url: String,
}

/// 提交 URL 摄取（SSRF 防护在管道内）。
#[utoipa::path(post, path = "/knowledge/documents",
    request_body(content = SubmitUrlRequest, content_type = "application/json"),
    responses((status = 201, body = DocumentDto)))]
pub async fn submit_url(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<SubmitUrlRequest>,
) -> Result<(StatusCode, Json<DocumentDto>), ApiError> {
    require_knowledge(&principal)?;
    let (id, deduped) = svc(&state)
        .submit(IngestSource::Url(req.url))
        .await
        .map_err(ke)?;
    let doc = svc(&state).get_document(id).await.map_err(ke)?;
    if deduped {
        Ok((StatusCode::OK, Json(doc))) // 幂等命中返回 200
    } else {
        Ok((StatusCode::CREATED, Json(doc)))
    }
}

/// 上传文件摄取（multipart，字段名 file）。
#[utoipa::path(post, path = "/knowledge/upload",
    request_body(content = Vec<u8>, content_type = "multipart/form-data"),
    responses((status = 201, body = DocumentDto)))]
pub async fn upload(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<DocumentDto>), ApiError> {
    require_knowledge(&principal)?;
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
    let (id, deduped) = svc(&state)
        .submit(IngestSource::Bytes {
            name,
            content,
            content_type,
        })
        .await
        .map_err(ke)?;
    let doc = svc(&state).get_document(id).await.map_err(ke)?;
    if deduped {
        Ok((StatusCode::OK, Json(doc)))
    } else {
        Ok((StatusCode::CREATED, Json(doc)))
    }
}

#[derive(Deserialize, IntoParams)]
pub struct ListDocsParams {
    pub status: Option<String>,
    pub cursor: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<i64>,
}

#[utoipa::path(get, path = "/knowledge/documents", params(ListDocsParams),
    responses((status = 200, body = [DocumentDto])))]
pub async fn list_documents(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ListDocsParams>,
) -> Result<Json<Vec<DocumentDto>>, ApiError> {
    require_knowledge(&principal)?;
    Ok(Json(
        svc(&state)
            .list_documents(p.status.as_deref(), p.cursor, p.limit.unwrap_or(50))
            .await
            .map_err(ke)?,
    ))
}

#[utoipa::path(get, path = "/knowledge/documents/{id}",
    responses((status = 200, body = DocumentDto)))]
pub async fn get_document(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<DocumentDto>, ApiError> {
    require_knowledge(&principal)?;
    Ok(Json(svc(&state).get_document(id).await.map_err(ke)?))
}

#[utoipa::path(get, path = "/knowledge/documents/{id}/chunks",
    responses((status = 200, body = [(i32, String, bool)])))]
pub async fn document_chunks(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    require_knowledge(&principal)?;
    let chunks = svc(&state).chunks(id, 500).await.map_err(ke)?;
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

#[utoipa::path(delete, path = "/knowledge/documents/{id}", responses((status = 204)))]
pub async fn delete_document(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_knowledge(&principal)?;
    svc(&state).delete(id).await.map_err(ke)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct KnowledgeSearchRequest {
    pub query: String,
    pub max_items: Option<i64>,
}

/// 知识混合检索（结果带文档引用 + 高亮片段）。
#[utoipa::path(post, path = "/knowledge/search", operation_id = "knowledge_search",
    request_body = KnowledgeSearchRequest,
    responses((status = 200, body = [ChunkHit])))]
pub async fn search(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<KnowledgeSearchRequest>,
) -> Result<Json<Vec<ChunkHit>>, ApiError> {
    require_knowledge(&principal)?;
    Ok(Json(
        svc(&state)
            .search(&req.query, req.max_items.unwrap_or(20).min(100))
            .await
            .map_err(ke)?,
    ))
}

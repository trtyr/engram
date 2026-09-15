//! wiki 文档域：文档摄取（解析→分块→嵌入）+ 检索。
//!
//! 设计文档：server/docs/wiki/ingest.md
//! 持久化在 `engram_storage::repo::wiki_docs`（本文件只保留编排：LLM 嵌入调用、
//! 无 token 短路、snippet 截断与文件清理）。

pub mod chunking;
pub mod pipeline;
pub mod ssrf;

use chrono::{DateTime, Utc};
use engram_jobs::JobQueue;
use engram_jobs::types::JobTemplate;
use engram_llm::ProviderRegistry;
use engram_llm::types::Purpose;
use engram_search::tokenize::{has_query_tokens, tsv_query_smart};
use engram_storage::repo::wiki_docs as repo;
use uuid::Uuid;

pub use pipeline::{IngestSource, WikiDocumentError, register_handlers};

pub use engram_storage::models::wiki_docs::DocumentDto;

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct ChunkHit {
    pub chunk_id: Uuid,
    pub document_id: Uuid,
    pub document_title: String,
    pub seq: i32,
    pub snippet: String,
    /// R9：前一相邻块片段（≤200 字；文档首块为空串）
    pub context_prev: String,
    /// R9：后一相邻块片段（≤200 字；文档末块为空串）
    pub context_next: String,
    pub score: f64,
    pub embed_failed: bool,
}

#[derive(Clone)]
pub struct WikiDocumentService {
    pool: engram_storage::PgPool,
    queue: JobQueue,
    registry: ProviderRegistry,
    pub data_dir: std::path::PathBuf,
}

impl WikiDocumentService {
    pub fn new(
        pool: engram_storage::PgPool,
        registry: ProviderRegistry,
        data_dir: impl Into<std::path::PathBuf>,
    ) -> Self {
        Self {
            queue: JobQueue::new(pool.clone()),
            pool,
            registry,
            data_dir: data_dir.into(),
        }
    }

    /// 提交摄取（上传字节或 URL）。sha 命中返回 (既有id, true)。
    pub async fn submit(
        &self,
        lib: Uuid,
        source: IngestSource,
    ) -> Result<(Uuid, bool), WikiDocumentError> {
        if let IngestSource::Bytes { content, .. } = &source
            && content.len() > 50 * 1024 * 1024
        {
            return Err(WikiDocumentError::BadRequest("文件超过 50MB 上限".into()));
        }
        pipeline::enqueue_ingest(&self.queue, &self.registry, &self.data_dir, lib, source).await
    }

    pub async fn list_documents(
        &self,
        lib: Uuid,
        status: Option<&str>,
        cursor: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<DocumentDto>, WikiDocumentError> {
        repo::list_documents(&self.pool, lib, status, cursor, limit.min(200))
            .await
            .map_err(|e| WikiDocumentError::Storage(e.to_string()))
    }

    pub async fn get_document(
        &self,
        lib: Uuid,
        id: Uuid,
    ) -> Result<DocumentDto, WikiDocumentError> {
        repo::get_document(&self.pool, lib, id)
            .await
            .map_err(|e| WikiDocumentError::Storage(e.to_string()))?
            .ok_or_else(|| WikiDocumentError::NotFound(format!("文档 {id} 不存在")))
    }

    pub async fn chunks(
        &self,
        lib: Uuid,
        id: Uuid,
        limit: i64,
    ) -> Result<Vec<(i32, String, bool)>, WikiDocumentError> {
        repo::list_chunks(&self.pool, lib, id, limit)
            .await
            .map_err(|e| WikiDocumentError::Storage(e.to_string()))
    }

    /// 删除：级联 chunks + 文件。
    pub async fn delete(&self, lib: Uuid, id: Uuid) -> Result<(), WikiDocumentError> {
        let raw_path = repo::delete_document_returning_path(&self.pool, lib, id)
            .await
            .map_err(|e| WikiDocumentError::Storage(e.to_string()))?
            .ok_or_else(|| WikiDocumentError::NotFound(format!("文档 {id} 不存在")))?;
        if !raw_path.is_empty() {
            let _ = tokio::fs::remove_file(&raw_path).await;
        }
        let _ = tokio::fs::remove_file(
            self.data_dir
                .join("uploads")
                .join(format!("{id}.extracted.txt")),
        )
        .await;
        Ok(())
    }

    /// 重新嵌入缺失块（K8：embed_failed / NULL 向量的显式恢复入口）。
    pub async fn reembed(&self, lib: Uuid, id: Uuid) -> Result<(), WikiDocumentError> {
        let status = repo::get_document_status(&self.pool, lib, id)
            .await
            .map_err(|e| WikiDocumentError::Storage(e.to_string()))?;
        match status.as_deref() {
            None => Err(WikiDocumentError::NotFound(format!("文档 {id} 不存在"))),
            Some("ready") => self
                .queue
                .enqueue(
                    JobTemplate::new("embed_document")
                        .with_payload(serde_json::json!({"document_id": id}))
                        .with_idempotency_key(format!("reembed-{id}-{}", Uuid::now_v7().simple())),
                )
                .await
                .map(|_| ())
                .map_err(|e| WikiDocumentError::Storage(format!("入队失败: {e}"))),
            Some(s) => Err(WikiDocumentError::BadRequest(format!(
                "文档状态 {s} 不可重嵌（需 ready）"
            ))),
        }
    }

    /// 混合检索 chunks（FTS + 向量 + RRF，带文档引用）。
    pub async fn search(
        &self,
        lib: Uuid,
        query: &str,
        limit: i64,
    ) -> Result<Vec<ChunkHit>, WikiDocumentError> {
        // L6：经记账门面（查询嵌入也计入用量，不再绕过记账）
        let qv: Option<Vec<f32>> = self
            .registry
            .embed_for(
                Purpose::Embed,
                vec![query.to_string()],
                Some(engram_distill::llm_port::embedding_dimensions()),
                None,
            )
            .await
            .ok()
            .and_then(|r| r.embeddings.first().cloned());
        // K7：单字/纯标点等无 token 且无查询向量 → 短路空结果（不再空跑 to_tsquery）
        if qv.is_none() && !has_query_tokens(query) {
            return Ok(vec![]);
        }
        let rows = repo::search_chunks(&self.pool, lib, &tsv_query_smart(query, 3), qv, limit)
            .await
            .map_err(|e| WikiDocumentError::Storage(e.to_string()))?;
        // R9：批量取相邻块片段（seq±1），一条 SQL；文档首尾块自然缺席 = 空串
        let mut keys = Vec::with_capacity(rows.len() * 2);
        for r in &rows {
            keys.push((r.document_id, r.seq - 1));
            keys.push((r.document_id, r.seq + 1));
        }
        let neighbors = repo::neighbor_snippets(&self.pool, &keys)
            .await
            .map_err(|e| WikiDocumentError::Storage(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let prev = neighbors.get(&(r.document_id, r.seq - 1)).cloned();
                let next = neighbors.get(&(r.document_id, r.seq + 1)).cloned();
                ChunkHit {
                    chunk_id: r.id,
                    document_id: r.document_id,
                    document_title: r.title,
                    seq: r.seq,
                    snippet: r.content.chars().take(200).collect(),
                    context_prev: prev.unwrap_or_default(),
                    context_next: next.unwrap_or_default(),
                    score: r.score,
                    embed_failed: r.embed_failed,
                }
            })
            .collect())
    }
}

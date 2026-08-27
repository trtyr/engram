//! 知识域：文档摄取（解析→分块→嵌入）+ 检索。
//!
//! 设计文档：docs/plantree/plans/agent-memory-platform/topics/knowledge-ingest.md

pub mod chunking;
pub mod pipeline;
pub mod ssrf;

use agent_memory_jobs::JobQueue;
use agent_memory_jobs::types::JobTemplate;
use agent_memory_llm::ProviderRegistry;
use agent_memory_llm::provider::LlmProvider as _;
use agent_memory_llm::types::{EmbedRequest, Purpose};
use agent_memory_search::tokenize::{has_query_tokens, tsv_query_smart};
use chrono::{DateTime, Utc};
use sqlx::{PgPool, QueryBuilder, Row};
use uuid::Uuid;

pub use pipeline::{IngestSource, KnowledgeError, register_handlers};

#[derive(Debug, serde::Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct DocumentDto {
    pub id: Uuid,
    pub title: String,
    pub source_uri: String,
    pub mime: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct ChunkHit {
    pub chunk_id: Uuid,
    pub document_id: Uuid,
    pub document_title: String,
    pub seq: i32,
    pub snippet: String,
    pub score: f64,
    pub embed_failed: bool,
}

#[derive(Clone)]
pub struct KnowledgeService {
    pool: PgPool,
    queue: JobQueue,
    registry: ProviderRegistry,
    pub data_dir: std::path::PathBuf,
}

impl KnowledgeService {
    pub fn new(
        pool: PgPool,
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
    pub async fn submit(&self, source: IngestSource) -> Result<(Uuid, bool), KnowledgeError> {
        if let IngestSource::Bytes { content, .. } = &source
            && content.len() > 50 * 1024 * 1024
        {
            return Err(KnowledgeError::BadRequest("文件超过 50MB 上限".into()));
        }
        pipeline::enqueue_ingest(&self.queue, &self.registry, &self.data_dir, source).await
    }

    pub async fn list_documents(
        &self,
        status: Option<&str>,
        cursor: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<DocumentDto>, KnowledgeError> {
        sqlx::query_as::<_, DocumentDto>(
            "SELECT * FROM documents \
             WHERE ($1::text IS NULL OR status = $1) AND ($2::timestamptz IS NULL OR created_at < $2) \
             ORDER BY created_at DESC LIMIT $3",
        )
        .bind(status)
        .bind(cursor)
        .bind(limit.min(200))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| KnowledgeError::Storage(e.to_string()))
    }

    pub async fn get_document(&self, id: Uuid) -> Result<DocumentDto, KnowledgeError> {
        sqlx::query_as::<_, DocumentDto>("SELECT * FROM documents WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| KnowledgeError::Storage(e.to_string()))?
            .ok_or_else(|| KnowledgeError::NotFound(format!("文档 {id} 不存在")))
    }

    pub async fn chunks(
        &self,
        id: Uuid,
        limit: i64,
    ) -> Result<Vec<(i32, String, bool)>, KnowledgeError> {
        sqlx::query_as(
            "SELECT seq, content, embed_failed FROM chunks WHERE document_id = $1 ORDER BY seq LIMIT $2",
        )
        .bind(id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| KnowledgeError::Storage(e.to_string()))
    }

    /// 删除：级联 chunks + 文件。
    pub async fn delete(&self, id: Uuid) -> Result<(), KnowledgeError> {
        let row = sqlx::query_as::<_, (Option<String>,)>(
            "DELETE FROM documents WHERE id = $1 RETURNING raw_path",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| KnowledgeError::Storage(e.to_string()))?
        .ok_or_else(|| KnowledgeError::NotFound(format!("文档 {id} 不存在")))?;
        let (raw_path,) = row;
        if let Some(path) = raw_path
            && !path.is_empty()
        {
            let _ = tokio::fs::remove_file(&path).await;
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
    pub async fn reembed(&self, id: Uuid) -> Result<(), KnowledgeError> {
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM documents WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| KnowledgeError::Storage(e.to_string()))?;
        match status.as_deref() {
            None => Err(KnowledgeError::NotFound(format!("文档 {id} 不存在"))),
            Some("ready") => {
                self.queue
                    .enqueue(
                        JobTemplate::new("embed_document")
                            .with_payload(serde_json::json!({"document_id": id}))
                            .with_idempotency_key(format!(
                                "reembed-{id}-{}",
                                Uuid::now_v7().simple()
                            )),
                    )
                    .await
                    .map(|_| ())
                    .map_err(|e| KnowledgeError::Storage(format!("入队失败: {e}")))
            }
            Some(s) => Err(KnowledgeError::BadRequest(format!(
                "文档状态 {s} 不可重嵌（需 ready）"
            ))),
        }
    }

    /// 混合检索 chunks（FTS + 向量 + RRF，带文档引用）。
    pub async fn search(&self, query: &str, limit: i64) -> Result<Vec<ChunkHit>, KnowledgeError> {
        let qv: Option<Vec<f32>> = match self.registry.resolve(Purpose::Embed).await {
            Ok((provider, model)) => provider
                .embed(EmbedRequest {
                    model,
                    inputs: vec![query.to_string()],
                    dimensions: Some(1024),
                })
                .await
                .ok()
                .and_then(|r| r.embeddings.first().cloned()),
            Err(_) => None,
        };
        // K7：单字/纯标点等无 token 且无查询向量 → 短路空结果（不再空跑 to_tsquery）
        if qv.is_none() && !has_query_tokens(query) {
            return Ok(vec![]);
        }
        let has_vec = qv.is_some();

        let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
            "WITH fts AS (SELECT c.id, ROW_NUMBER() OVER (ORDER BY ts_rank(c.tsv, q) DESC) AS rank \
             FROM chunks c, to_tsquery('simple', ",
        );
        qb.push_bind(tsv_query_smart(query, 3));
        qb.push(") q WHERE c.tsv @@ q LIMIT 200) ");

        if has_vec {
            qb.push(", vec AS (SELECT c.id, ROW_NUMBER() OVER (ORDER BY c.embedding <=> ");
            qb.push_bind(pgvector::Vector::from(qv.clone().unwrap()));
            qb.push(") AS rank FROM chunks c WHERE c.embedding IS NOT NULL LIMIT 200) ");
        }

        qb.push("SELECT c.id, c.document_id, c.seq, c.content, c.embed_failed, COALESCE(1.0/(60 + fts.rank), 0)");
        if has_vec {
            qb.push(" + COALESCE(1.0/(60 + vec.rank), 0)");
        }
        qb.push(
            "::float8 AS score, d.title \
             FROM chunks c JOIN documents d ON d.id = c.document_id \
             LEFT JOIN fts ON fts.id = c.id ",
        );
        if has_vec {
            qb.push("LEFT JOIN vec ON vec.id = c.id ");
        }
        qb.push("WHERE fts.id IS NOT NULL");
        if has_vec {
            qb.push(" OR vec.id IS NOT NULL");
        }
        qb.push(" ORDER BY score DESC LIMIT ");
        qb.push_bind(limit);

        let rows = qb
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(|e| KnowledgeError::Storage(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|r| ChunkHit {
                chunk_id: r.get("id"),
                document_id: r.get("document_id"),
                document_title: r.get("title"),
                seq: r.get("seq"),
                snippet: {
                    let s: String = r.get("content");
                    s.chars().take(200).collect()
                },
                score: r.get("score"),
                embed_failed: r.get("embed_failed"),
            })
            .collect())
    }
}

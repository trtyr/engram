//! 摄取管道 job handlers：parse → chunk → embed 三步链。

use agent_memory_jobs::JobContext;
use agent_memory_jobs::types::{JobError, JobTemplate};
use agent_memory_llm::ProviderRegistry;
use agent_memory_llm::provider::LlmProvider as _;
use agent_memory_llm::types::{EmbedRequest, Purpose};
use agent_memory_search::tokenize::tsv_text;
use serde_json::json;
use std::path::PathBuf;
use uuid::Uuid;

use super::chunking::chunk_text;
use super::parse::parse_bytes;

/// 并发说明：管道并发由 RunnerConfig.concurrency（默认 4）全局约束，
/// 不在 job 内做 per-kind 限流（单用户规模下解析快，避免互相挤死的重试风暴）。
///
/// 入队摄取（幂等：sha 命中返回既有文档）。
pub async fn enqueue_ingest(
    queue: &agent_memory_jobs::JobQueue,
    _registry: &ProviderRegistry,
    data_dir: &std::path::Path,
    source: IngestSource,
) -> Result<(Uuid, bool), KnowledgeError> {
    // sha256 计算
    let bytes = match &source {
        IngestSource::Bytes {
            name,
            content,
            content_type,
        } => {
            let mut hasher = sha2::Sha256::new();
            use sha2::Digest;
            hasher.update(name.as_bytes());
            hasher.update(content_type.as_deref().unwrap_or("").as_bytes());
            hasher.update(content);
            hasher.finalize().to_vec()
        }
        IngestSource::Url(url) => {
            use sha2::Digest;
            let mut h = sha2::Sha256::new();
            h.update(url.as_bytes());
            h.finalize().to_vec()
        }
    };
    let sha = hex(&bytes);

    // 幂等：同 sha 已有文档（任何状态）→ 返回既有
    if let Some((id, _status)) =
        sqlx::query_as::<_, (Uuid, String)>("SELECT id, status FROM documents WHERE sha256 = $1")
            .bind(&sha)
            .fetch_optional(queue.pool())
            .await
            .map_err(|e| KnowledgeError::Storage(e.to_string()))?
    {
        return Ok((id, true));
    }

    // 落盘 / 记 URL
    let id = Uuid::now_v7();
    let (title, raw_path, mime, source_uri) = match &source {
        IngestSource::Bytes {
            name,
            content,
            content_type,
        } => {
            let uploads = data_dir.join("uploads");
            tokio::fs::create_dir_all(&uploads)
                .await
                .map_err(|e| KnowledgeError::Storage(format!("创建 uploads 失败: {e}")))?;
            let safe_name = name.replace(['/', '\\'], "_");
            let path = uploads.join(format!("{id}_{safe_name}"));
            tokio::fs::write(&path, content)
                .await
                .map_err(|e| KnowledgeError::Storage(format!("写文件失败: {e}")))?;
            (
                name.clone(),
                path.to_string_lossy().into_owned(),
                content_type.clone(),
                name.clone(),
            )
        }
        IngestSource::Url(url) => (url.clone(), String::new(), None, url.clone()),
    };

    sqlx::query(
        "INSERT INTO documents (id, title, source_uri, mime, raw_path, sha256, status) \
         VALUES ($1, $2, $3, $4, NULLIF($5,''), $6, 'pending')",
    )
    .bind(id)
    .bind(&title)
    .bind(&source_uri)
    .bind(&mime)
    .bind(&raw_path)
    .bind(&sha)
    .execute(queue.pool())
    .await
    .map_err(|e| KnowledgeError::Storage(e.to_string()))?;

    queue
        .enqueue(
            JobTemplate::new("parse_document")
                .with_payload(json!({"document_id": id}))
                .with_idempotency_key(format!("ingest-{id}")),
        )
        .await
        .ok();
    Ok((id, false))
}

#[derive(Debug)]
pub enum IngestSource {
    Bytes {
        name: String,
        content: Vec<u8>,
        content_type: Option<String>,
    },
    Url(String),
}

#[derive(Debug, thiserror::Error)]
pub enum KnowledgeError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("存储暂时不可用: {0}")]
    Storage(String),
}

/// parse_document：读源 → 纯文本 → 落 extracted_text → 入队 chunk。
/// URL 抓取在此步做（SSRF 防护）。
pub async fn parse_job(ctx: JobContext) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();
    let doc_id: Uuid = ctx
        .job
        .payload
        .0
        .get("document_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| JobError::Permanent("payload 缺 document_id".into()))?;

    // 读取文档行（raw_path 可空：URL 文档摄取前无本地文件）
    let row = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT source_uri, raw_path, mime FROM documents WHERE id = $1",
    )
    .bind(doc_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?
    .ok_or_else(|| JobError::Permanent(format!("文档 {doc_id} 不存在")))?;
    let (source_uri, raw_path, mime) = row;
    let raw_path = raw_path.unwrap_or_default();

    let fail = |msg: String| -> JobError {
        tracing::warn!(doc = %doc_id, error = %msg, "文档摄取失败");
        JobError::Permanent(msg)
    };
    // 状态标记为 failed（尽力而为，事件可查）
    async fn mark_failed(ctx: &JobContext, doc_id: Uuid, msg: &str) {
        let _ = sqlx::query(
            "UPDATE documents SET status = 'failed', error = $2, updated_at = now() WHERE id = $1",
        )
        .bind(doc_id)
        .bind(msg)
        .execute(ctx.pool())
        .await;
        ctx.emit("文档摄取失败", Some(serde_json::json!({"error": msg})))
            .await
            .ok();
    }

    // 取字节：URL 抓取 or 本地文件
    let (name, bytes, ctype) = if raw_path.is_empty() {
        // URL
        ctx.emit(&format!("抓取 {source_uri}"), None).await.ok();
        match super::ssrf::safe_fetch(
            &source_uri,
            20 * 1024 * 1024,
            std::time::Duration::from_secs(30),
        )
        .await
        {
            Ok(page) => {
                // 抓取成功后内容落盘（重试不重复抓）
                let uploads = data_uploads();
                let _ = tokio::fs::create_dir_all(&uploads).await;
                let path = uploads.join(format!("{doc_id}_url.html"));
                let _ = tokio::fs::write(&path, &page.bytes).await;
                sqlx::query("UPDATE documents SET raw_path = $2, mime = COALESCE($3, mime), title = COALESCE($4, title), updated_at = now() WHERE id = $1")
                    .bind(doc_id)
                    .bind(path.to_string_lossy().as_ref())
                    .bind(page.content_type.clone())
                    .bind(extract_title_from_html(&page.bytes).or(Some(source_uri.clone())))
                    .execute(pool)
                    .await
                    .ok();
                (format!("{doc_id}_url.html"), page.bytes, page.content_type)
            }
            Err(e) => {
                let m = format!("URL 抓取失败: {e}");
                mark_failed(&ctx, doc_id, &m).await;
                return Err(fail(m));
            }
        }
    } else {
        let path = PathBuf::from(&raw_path);
        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(e) => {
                let m = format!("读文件失败: {e}");
                mark_failed(&ctx, doc_id, &m).await;
                return Err(fail(m));
            }
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        (name, bytes, mime)
    };

    // 状态 → parsing
    sqlx::query("UPDATE documents SET status = 'parsing', updated_at = now() WHERE id = $1")
        .bind(doc_id)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    // 解析（CPU → blocking）
    let n = name.clone();
    let ct = ctype.clone();
    let b = bytes.clone();
    let text = match tokio::task::spawn_blocking(move || parse_bytes(&n, ct.as_deref(), &b)).await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => {
            let m = e.to_string();
            mark_failed(&ctx, doc_id, &m).await;
            return Err(fail(m));
        }
        Err(e) => return Err(JobError::Permanent(format!("解析线程崩溃: {e}"))),
    };

    // 存 extracted（临时文件，chunk 步消费）；确保目录存在
    let uploads_dir = data_uploads();
    let _ = tokio::fs::create_dir_all(&uploads_dir).await;
    let extracted_path = uploads_dir.join(format!("{doc_id}.extracted.txt"));
    tokio::fs::write(&extracted_path, &text)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    ctx.emit(&format!("解析完成（{} 字）", text.chars().count()), None)
        .await
        .ok();

    // 入队 chunk
    ctx.enqueue_next(
        JobTemplate::new("chunk_document").with_payload(json!({"document_id": doc_id})),
    )
    .await?;
    Ok(json!({"document_id": doc_id, "chars": text.chars().count()}))
}

fn data_uploads() -> PathBuf {
    std::env::var("AGENT_MEMORY_DATA_DIR")
        .unwrap_or_else(|_| "./data".into())
        .into()
}

fn extract_title_from_html(bytes: &[u8]) -> Option<String> {
    let raw = String::from_utf8_lossy(bytes);
    let html = scraper::Html::parse_document(&raw);
    let sel = scraper::Selector::parse("title").ok()?;
    html.select(&sel)
        .next()
        .map(|t| t.text().collect::<String>())
}

/// chunk_document：extracted → 分块入库 → 入队 embed。
pub async fn chunk_job(ctx: JobContext) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();
    let doc_id: Uuid = ctx
        .job
        .payload
        .0
        .get("document_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| JobError::Permanent("payload 缺 document_id".into()))?;

    sqlx::query("UPDATE documents SET status = 'chunking', updated_at = now() WHERE id = $1")
        .bind(doc_id)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    let extracted_path = data_uploads().join(format!("{doc_id}.extracted.txt"));
    let text = tokio::fs::read_to_string(&extracted_path)
        .await
        .map_err(|e| JobError::Permanent(format!("extracted 文件缺失: {e}")))?;

    let chunks = chunk_text(&text);
    if chunks.is_empty() {
        sqlx::query("UPDATE documents SET status = 'failed', error = '解析后内容为空', updated_at = now() WHERE id = $1")
            .bind(doc_id)
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
        return Ok(json!({"document_id": doc_id, "chunks": 0, "empty": true}));
    }

    for c in &chunks {
        sqlx::query(
            "INSERT INTO chunks (id, document_id, seq, content, tsv) \
             VALUES ($1, $2, $3, $4, to_tsvector('simple', $5)) \
             ON CONFLICT (document_id, seq) DO UPDATE SET content = $4, tsv = to_tsvector('simple', $5)",
        )
        .bind(Uuid::now_v7())
        .bind(doc_id)
        .bind(c.seq as i32)
        .bind(&c.content)
        .bind(tsv_text(&c.content))
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    }
    ctx.emit(&format!("分块 {} 块", chunks.len()), None)
        .await
        .ok();

    // 入队 embed
    ctx.enqueue_next(
        JobTemplate::new("embed_document").with_payload(json!({"document_id": doc_id})),
    )
    .await?;
    Ok(json!({"document_id": doc_id, "chunks": chunks.len()}))
}

/// embed_document：批量嵌入 chunks（失败块标 embed_failed 不阻塞）→ ready。
pub async fn embed_job(
    ctx: JobContext,
    registry: ProviderRegistry,
) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();
    let doc_id: Uuid = ctx
        .job
        .payload
        .0
        .get("document_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| JobError::Permanent("payload 缺 document_id".into()))?;

    sqlx::query("UPDATE documents SET status = 'embedding', updated_at = now() WHERE id = $1")
        .bind(doc_id)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    let chunks: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, content FROM chunks WHERE document_id = $1 ORDER BY seq")
            .bind(doc_id)
            .fetch_all(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;

    let mut embedded = 0usize;
    if !chunks.is_empty() {
        // 批量（≤64/批）
        for batch in chunks.chunks(64) {
            let texts: Vec<String> = batch.iter().map(|(_, c)| c.clone()).collect();
            match registry.resolve(Purpose::Embed).await {
                Ok((provider, model)) => match provider
                    .embed(EmbedRequest {
                        model,
                        inputs: texts,
                        dimensions: Some(1024),
                    })
                    .await
                {
                    Ok(resp) => {
                        for (i, (cid, _)) in batch.iter().enumerate() {
                            let emb = resp.embeddings.get(i);
                            sqlx::query(
                                "UPDATE chunks SET embedding = $2, embed_failed = false WHERE id = $1",
                            )
                            .bind(cid)
                            .bind(emb.map(|v| pgvector::Vector::from(v.clone())))
                            .execute(pool)
                            .await
                            .map_err(|e| JobError::Retryable(e.to_string()))?;
                            embedded += 1;
                        }
                        ctx.emit(
                            &format!("嵌入 {}/{}", embedded, chunks.len()),
                            Some(json!({"done": embedded, "total": chunks.len()})),
                        )
                        .await
                        .ok();
                    }
                    Err(e) => {
                        // 本批标 embed_failed（降级 FTS），不阻塞 ready
                        tracing::warn!(error = %e, "嵌入批次失败，标记 embed_failed");
                        for (cid, _) in batch {
                            sqlx::query("UPDATE chunks SET embed_failed = true WHERE id = $1")
                                .bind(cid)
                                .execute(pool)
                                .await
                                .map_err(|e2| JobError::Retryable(e2.to_string()))?;
                        }
                    }
                },
                Err(e) => {
                    // 无 provider：全部降级 FTS
                    tracing::warn!(error = %e, "无 embedding provider，全部降级 FTS");
                    for (cid, _) in batch {
                        sqlx::query("UPDATE chunks SET embed_failed = true WHERE id = $1")
                            .bind(cid)
                            .execute(pool)
                            .await
                            .map_err(|e2| JobError::Retryable(e2.to_string()))?;
                    }
                }
            }
        }
    }

    sqlx::query(
        "UPDATE documents SET status = 'ready', error = NULL, updated_at = now() WHERE id = $1",
    )
    .bind(doc_id)
    .execute(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    // 清理 extracted 临时文件
    let _ = tokio::fs::remove_file(data_uploads().join(format!("{doc_id}.extracted.txt"))).await;

    ctx.emit(
        &format!("文档 ready（{embedded}/{} 嵌入成功）", chunks.len()),
        None,
    )
    .await
    .ok();
    Ok(json!({"document_id": doc_id, "embedded": embedded, "total": chunks.len()}))
}

/// 注册知识域 handlers（main 装配用）。
pub fn register_handlers(
    runner: agent_memory_jobs::Runner,
    registry: ProviderRegistry,
) -> agent_memory_jobs::Runner {
    let r1 = registry.clone();
    runner
        .register("parse_document", |ctx| async move { parse_job(ctx).await })
        .register("chunk_document", |ctx| async move { chunk_job(ctx).await })
        .register("embed_document", move |ctx| {
            let reg = r1.clone();
            async move { embed_job(ctx, reg).await }
        })
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

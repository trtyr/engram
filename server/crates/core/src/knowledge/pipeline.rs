//! 摄取管道 job handlers：parse → chunk → embed 三步链。

use agent_memory_jobs::JobContext;
use agent_memory_jobs::types::{JobError, JobTemplate};
use agent_memory_llm::ProviderRegistry;
use agent_memory_llm::types::Purpose;
use agent_memory_parsing::parse_bytes;
use agent_memory_search::tokenize::tsv_text;
use serde_json::json;
use std::path::PathBuf;
use uuid::Uuid;

use super::chunking::chunk_text;

/// 并发说明：管道并发由 RunnerConfig.concurrency（默认 4）全局约束，
/// 不在 job 内做 per-kind 限流（单用户规模下解析快，避免互相挤死的重试风暴）。
///
/// 入队摄取（幂等：sha 命中返回既有文档；K6 并发同 sha 无竞态、K2 入队失败回滚）。
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

    // 落盘 / 记 URL（冲突路径下再清理刚写的文件）
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

    // K6：INSERT ... ON CONFLICT 单往返——并发同 sha 一个赢、一个幂等命中，不再竞态 503
    let inserted = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO documents (id, title, source_uri, mime, raw_path, sha256, status) \
         VALUES ($1, $2, $3, $4, NULLIF($5,''), $6, 'pending') \
         ON CONFLICT (sha256) DO NOTHING RETURNING id",
    )
    .bind(id)
    .bind(&title)
    .bind(&source_uri)
    .bind(&mime)
    .bind(&raw_path)
    .bind(&sha)
    .fetch_optional(queue.pool())
    .await
    .map_err(|e| KnowledgeError::Storage(e.to_string()))?;

    let Some(id) = inserted else {
        // 幂等命中：清掉刚写的文件
        if !raw_path.is_empty() {
            let _ = tokio::fs::remove_file(&raw_path).await;
        }
        let existing: Uuid = sqlx::query_scalar("SELECT id FROM documents WHERE sha256 = $1")
            .bind(&sha)
            .fetch_one(queue.pool())
            .await
            .map_err(|e| KnowledgeError::Storage(e.to_string()))?;

        // K1 自愈：failed 文档 / 非终态卡死（>5 分钟无 pending·running 活 job）→ 原子重置 + 重新入队。
        // 不再让一次网络抖动永久卡死该 sha；ready 或在途文档不受影响。
        // 5 分钟静默期：杜绝「并发重复提交时，先到者尚未入队」毫秒窗口被误判为卡死。
        let healed = sqlx::query_scalar::<_, Uuid>(
            "UPDATE documents SET status = 'pending', error = NULL, updated_at = now() \
             WHERE id = $1 AND ( \
                status = 'failed' \
                OR (status <> 'ready' \
                    AND updated_at < now() - interval '5 minutes' \
                    AND NOT EXISTS ( \
                        SELECT 1 FROM jobs \
                        WHERE kind IN ('parse_document','chunk_document','embed_document') \
                          AND status IN ('pending','running') \
                          AND payload->>'document_id' = $1::text)) \
             ) RETURNING id",
        )
        .bind(existing)
        .fetch_optional(queue.pool())
        .await
        .map_err(|e| KnowledgeError::Storage(e.to_string()))?;
        if healed.is_some() {
            // 新幂等键：旧 ingest-{id} job 已 failed/dead，复用会被幂等墙挡住
            queue
                .enqueue(
                    JobTemplate::new("parse_document")
                        .with_payload(json!({"document_id": existing}))
                        .with_idempotency_key(format!(
                            "ingest-{existing}-{}",
                            Uuid::now_v7().simple()
                        )),
                )
                .await
                .map_err(|e| KnowledgeError::Storage(format!("自愈入队失败: {e}")))?;
        }
        return Ok((existing, true));
    };

    // K2：入队失败 → 回滚文档行 + 文件，错误上抛（不再 .ok() 吞掉致文档永卡 pending）
    if let Err(e) = queue
        .enqueue(
            JobTemplate::new("parse_document")
                .with_payload(json!({"document_id": id}))
                .with_idempotency_key(format!("ingest-{id}")),
        )
        .await
    {
        let _ = sqlx::query("DELETE FROM documents WHERE id = $1")
            .bind(id)
            .execute(queue.pool())
            .await;
        if !raw_path.is_empty() {
            let _ = tokio::fs::remove_file(&raw_path).await;
        }
        return Err(KnowledgeError::Storage(format!("job 入队失败: {e}")));
    }
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
                // K9：瞬态网络错误（含超时）→ Retryable，交给队列退避重试，不 mark_failed；
                // SSRF 判定/协议/大小/DNS 类保持 Permanent，防恶意 URL 反复探测
                if matches!(e, super::ssrf::FetchError::Network(_)) {
                    ctx.emit(&m, None).await.ok();
                    return Err(JobError::Retryable(m));
                }
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

    // K8：只补缺失块（NULL 向量或 embed_failed）——重跑 / re-embed / Transient 重试
    // 的进度天然保留，已嵌入块不重复计费
    let chunks: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, content FROM chunks \
         WHERE document_id = $1 AND (embedding IS NULL OR embed_failed) ORDER BY seq",
    )
    .bind(doc_id)
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chunks WHERE document_id = $1")
        .bind(doc_id)
        .fetch_one(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    let missing = chunks.len();

    let mut embedded = 0usize;
    if !chunks.is_empty() {
        // 批量（≤64/批）；L6：经记账门面（批量嵌入计入用量，不再绕过记账）
        for batch in chunks.chunks(64) {
            let texts: Vec<String> = batch.iter().map(|(_, c)| c.clone()).collect();
            match registry
                .embed_for(Purpose::Embed, texts, Some(1024), Some(ctx.job.id))
                .await
            {
                Ok(resp) => {
                    // K4：响应数量或维度与批次不符 → 整批按失败处理，
                    // 杜绝「NULL 向量 + embed_failed=false」双重静默入库
                    let bad = resp.embeddings.len() != batch.len()
                        || resp.embeddings.iter().any(|v| v.len() != 1024); // D0010
                    if bad {
                        tracing::warn!(
                            doc = %doc_id,
                            expected = batch.len(),
                            got = resp.embeddings.len(),
                            "embed 响应与批次不符，整批降级 FTS"
                        );
                        for (cid, _) in batch {
                            sqlx::query("UPDATE chunks SET embed_failed = true WHERE id = $1")
                                .bind(cid)
                                .execute(pool)
                                .await
                                .map_err(|e| JobError::Retryable(e.to_string()))?;
                        }
                        continue;
                    }
                    for (i, (cid, _)) in batch.iter().enumerate() {
                        sqlx::query(
                            "UPDATE chunks SET embedding = $2, embed_failed = false WHERE id = $1",
                        )
                        .bind(cid)
                        .bind(pgvector::Vector::from(resp.embeddings[i].clone()))
                        .execute(pool)
                        .await
                        .map_err(|e| JobError::Retryable(e.to_string()))?;
                        embedded += 1;
                    }
                    ctx.emit(
                        &format!("补嵌 {}/{}", embedded, missing),
                        Some(json!({"done": embedded, "missing": missing})),
                    )
                    .await
                    .ok();
                }
                Err(e) => {
                    // K8：瞬态失败（429/5xx/超时）→ 整体重试，只补缺失保证进度不丢；
                    // 永久失败才降级 FTS（不阻塞 ready，可事后 re-embed 恢复）
                    if matches!(e, agent_memory_llm::types::LlmError::Transient(_)) {
                        return Err(JobError::Retryable(format!("嵌入瞬态失败: {e}")));
                    }
                    tracing::warn!(error = %e, "嵌入批次失败，标记 embed_failed");
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

    // 自动织入 Wiki：文档 ready 后织成互链页面（upload 文档读原文件重解析；
    // URL 文档 raw_path 空时用已分块文本拼接——2026-09-04 补，此前静默跳过）。
    // best-effort（sha256 去重 + 失败不影响文档 ready，页面层降级为空）。
    {
        let wiki = crate::wiki::WikiService::new(pool.clone(), registry.clone());
        let _ = wiki.ingest_document(doc_id).await;
    }

    // 清理 extracted 临时文件
    let _ = tokio::fs::remove_file(data_uploads().join(format!("{doc_id}.extracted.txt"))).await;

    ctx.emit(
        &format!("文档 ready（补嵌 {embedded}/{missing}，共 {total} 块）"),
        None,
    )
    .await
    .ok();
    Ok(json!({"document_id": doc_id, "embedded": embedded, "missing": missing, "total": total}))
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

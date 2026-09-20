use super::*;

/// 文档摄取失败错误（统一告警 + Permanent 语义）。
pub(super) fn doc_fail(doc_id: Uuid, msg: String) -> JobError {
    tracing::warn!(doc = %doc_id, error = %msg, "文档摄取失败");
    JobError::Permanent(msg)
}

/// 状态标记为 failed（尽力而为，事件可查）。
pub(super) async fn mark_doc_failed(ctx: &JobContext, lib: Uuid, doc_id: Uuid, msg: &str) {
    if let Err(e) = repo::mark_failed_document(ctx.pool(), lib, doc_id, msg).await {
        tracing::warn!(doc = %doc_id, error = %e, "失败态落库失败（文档可能停在处理中）");
    }
    ctx.emit("文档摄取失败", Some(serde_json::json!({"error": msg})))
        .await
        .ok();
}

/// 解析入库：状态 → parsing →（CPU 解析走 blocking）→ 存 extracted 临时文件 → 入队 chunk。
pub(super) async fn parse_and_store_document(
    ctx: &JobContext,
    pool: &engram_storage::PgPool,
    lib: Uuid,
    doc_id: Uuid,
    name: String,
    bytes: Vec<u8>,
    ctype: Option<String>,
) -> Result<serde_json::Value, JobError> {
    // 状态 → parsing
    repo::update_document_status(pool, lib, doc_id, "parsing")
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
            mark_doc_failed(ctx, lib, doc_id, &m).await;
            return Err(doc_fail(doc_id, m));
        }
        Err(e) => return Err(JobError::Permanent(format!("解析线程崩溃: {e}"))),
    };

    // 存 extracted（临时文件，chunk 步消费）；确保目录存在
    let uploads_dir = data_uploads();
    let _ = tokio::fs::create_dir_all(&uploads_dir).await; // 有意忽略：目录已存在不算失败；后续写入会暴露真错误
    let extracted_path = uploads_dir.join(format!("{doc_id}.extracted.txt"));
    tokio::fs::write(&extracted_path, &text)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    ctx.emit(&format!("解析完成（{} 字）", text.chars().count()), None)
        .await
        .ok();

    // 入队 chunk
    ctx.enqueue_next(
        JobTemplate::new("chunk_document")
            .with_payload(json!({"document_id": doc_id, "library_id": lib})),
    )
    .await?;
    Ok(json!({"document_id": doc_id, "chars": text.chars().count()}))
}

/// 取文档字节：URL 抓取（SSRF 守卫 + 抓取结果落盘）或本地文件读取。
/// 错误分治：HTTP 4xx（除 429）→ Permanent + mark_failed；429/5xx 与网络错误 → Retryable；
/// Retryable 最后一试也 mark_failed（杜绝「job dead + 文档 pending」孤儿态）。
pub(super) async fn fetch_document_bytes(
    ctx: &JobContext,
    pool: &engram_storage::PgPool,
    source_uri: &str,
    raw_path: &str,
    mime: Option<String>,
    lib: Uuid,
    doc_id: Uuid,
) -> Result<(String, Vec<u8>, Option<String>), JobError> {
    let (name, bytes, ctype) = if raw_path.is_empty() {
        // URL
        ctx.emit(&format!("抓取 {source_uri}"), None).await.ok();
        match super::super::ssrf::safe_fetch(
            &source_uri,
            20 * 1024 * 1024,
            std::time::Duration::from_secs(30),
        )
        .await
        {
            Ok(page) => {
                // 抓取成功后内容落盘（重试不重复抓）
                let uploads = data_uploads();
                let _ = tokio::fs::create_dir_all(&uploads).await; // 有意忽略：目录已存在不算失败；后续写入会暴露真错误
                let path = uploads.join(format!("{doc_id}_url.html"));
                if let Err(e) = tokio::fs::write(&path, &page.bytes).await {
                    tracing::warn!(path = %path.display(), error = %e, "上传件落盘失败");
                }
                let title = extract_title_from_html(&page.bytes).or(Some(source_uri.to_string()));
                // 错误上浮：抓取结果落库失败必须可见（否则成功被抓取的文档状态失真）
                if let Err(e) = repo::update_document_fetch_result(
                    pool,
                    lib,
                    doc_id,
                    path.to_string_lossy().as_ref(),
                    page.content_type.as_deref(),
                    title.as_deref(),
                )
                .await
                {
                    tracing::warn!(doc = %doc_id, error = %e, "抓取结果落库失败（文档状态可能失真）");
                }
                (format!("{doc_id}_url.html"), page.bytes, page.content_type)
            }
            Err(e) => return Err(fetch_failure_error(ctx, lib, doc_id, e).await),
        }
    } else {
        let path = PathBuf::from(&raw_path);
        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(e) => {
                let m = format!("读文件失败: {e}");
                mark_doc_failed(ctx, lib, doc_id, &m).await;
                return Err(doc_fail(doc_id, m));
            }
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        (name, bytes, mime)
    };
    Ok((name, bytes, ctype))
}

/// K6/K2：单往返 INSERT（冲突即幂等命中）+ K1 卡死自愈重入队；入队失败回滚文档行与文件。
/// 返回 `(文档 id, 是否幂等命中)`。
#[allow(clippy::too_many_arguments)]
pub(super) async fn insert_and_enqueue_document(
    queue: &engram_jobs::JobQueue,
    lib: Uuid,
    id: Uuid,
    title: &str,
    source_uri: &str,
    raw_path: &str,
    mime: Option<String>,
    sha: &str,
) -> Result<(Uuid, bool), WikiDocumentError> {
    // K6：INSERT ... ON CONFLICT 单往返——并发同 sha 一个赢、一个幂等命中，不再竞态 503
    let inserted = repo::insert_document_sha(
        queue.pool(),
        lib,
        id,
        title,
        source_uri,
        mime.as_deref(),
        raw_path,
        sha,
    )
    .await
    .map_err(|e| WikiDocumentError::Storage(e.to_string()))?;

    let Some(id) = inserted else {
        // 幂等命中：清掉刚写的文件
        if !raw_path.is_empty() {
            let _ = tokio::fs::remove_file(&raw_path).await; // 有意忽略：best-effort 清理/建目录（失败由后续步骤或下次运行暴露）
        }
        let existing = repo::find_document_id_by_sha(queue.pool(), lib, sha)
            .await
            .map_err(|e| WikiDocumentError::Storage(e.to_string()))?;

        // K1 自愈：failed 文档 / 非终态卡死（>5 分钟无 pending·running 活 job）→ 原子重置 + 重新入队。
        // 不再让一次网络抖动永久卡死该 sha；ready 或在途文档不受影响。
        // 5 分钟静默期：杜绝「并发重复提交时，先到者尚未入队」毫秒窗口被误判为卡死。
        let healed = repo::heal_stuck_document(queue.pool(), lib, existing)
            .await
            .map_err(|e| WikiDocumentError::Storage(e.to_string()))?;
        if healed.is_some() {
            // 新幂等键：旧 ingest-{id} job 已 failed/dead，复用会被幂等墙挡住
            queue
                .enqueue(
                    JobTemplate::new("parse_document")
                        .with_payload(json!({"document_id": existing, "library_id": lib}))
                        .with_idempotency_key(format!(
                            "ingest-{existing}-{}",
                            Uuid::now_v7().simple()
                        )),
                )
                .await
                .map_err(|e| WikiDocumentError::Storage(format!("自愈入队失败: {e}")))?;
        }
        return Ok((existing, true));
    };

    // K2：入队失败 → 回滚文档行 + 文件，错误上抛（不再 .ok() 吞掉致文档永卡 pending）
    if let Err(e) = queue
        .enqueue(
            JobTemplate::new("parse_document")
                .with_payload(json!({"document_id": id, "library_id": lib}))
                .with_idempotency_key(format!("ingest-{id}")),
        )
        .await
    {
        let _ = repo::delete_document_quiet(queue.pool(), lib, id).await; // 有意忽略：quiet 删除本就吞错（函数名即契约）
        if !raw_path.is_empty() {
            let _ = tokio::fs::remove_file(&raw_path).await; // 有意忽略：best-effort 清理/建目录（失败由后续步骤或下次运行暴露）
        }
        return Err(WikiDocumentError::Storage(format!("job 入队失败: {e}")));
    }
    Ok((id, false))
}

/// K8：批量补嵌缺失块（≤64/批）。K4 响应数量/维度不符 → 整批按失败处理；
/// 瞬态失败整体重试（进度保留），永久失败降级 FTS（可事后 re-embed）。
pub(super) async fn embed_missing_chunks(
    ctx: &JobContext,
    pool: &engram_storage::PgPool,
    registry: &ProviderRegistry,
    lib: Uuid,
    doc_id: Uuid,
    chunks: &[(Uuid, String)],
    missing: usize,
) -> Result<usize, JobError> {
    let mut embedded = 0usize;
    if !chunks.is_empty() {
        // 批量（≤64/批）；L6：经记账门面（批量嵌入计入用量，不再绕过记账）
        for batch in chunks.chunks(64) {
            let texts: Vec<String> = batch.iter().map(|(_, c)| c.clone()).collect();
            match registry
                .embed_for(
                    Purpose::Embed,
                    texts,
                    Some(engram_distill::llm_port::embedding_dimensions()),
                    Some(ctx.job.id),
                )
                .await
            {
                Ok(resp) => {
                    // K4：响应数量或维度与批次不符 → 整批按失败处理，
                    // 杜绝「NULL 向量 + embed_failed=false」双重静默入库
                    let dim = engram_distill::llm_port::embedding_dimensions() as usize;
                    let bad = resp.embeddings.len() != batch.len()
                        || resp.embeddings.iter().any(|v| v.len() != dim); // D0010
                    if bad {
                        tracing::warn!(
                            doc = %doc_id,
                            expected = batch.len(),
                            got = resp.embeddings.len(),
                            "embed 响应与批次不符，整批降级 FTS"
                        );
                        for (cid, _) in batch {
                            repo::set_chunk_failed(pool, lib, *cid)
                                .await
                                .map_err(|e| JobError::Retryable(e.to_string()))?;
                        }
                        continue;
                    }
                    for (i, (cid, _)) in batch.iter().enumerate() {
                        repo::set_chunk_embedding(pool, lib, *cid, resp.embeddings[i].clone())
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
                    if matches!(e, engram_llm::types::LlmError::Transient(_)) {
                        return Err(JobError::Retryable(format!("嵌入瞬态失败: {e}")));
                    }
                    tracing::warn!(error = %e, "嵌入批次失败，标记 embed_failed");
                    for (cid, _) in batch {
                        repo::set_chunk_failed(pool, lib, *cid)
                            .await
                            .map_err(|e| JobError::Retryable(e.to_string()))?;
                    }
                }
            }
        }
    }
    Ok(embedded)
}

/// URL 抓取失败分治（W-1/W-2）：HTTP 4xx（除 429）/SSRF 类 → Permanent；
/// 429/5xx/网络 → Retryable；Retryable 最后一试也 mark_failed（杜绝 job dead + 文档 pending 孤儿态）。
async fn fetch_failure_error(
    ctx: &JobContext,
    lib: Uuid,
    doc_id: Uuid,
    e: super::super::ssrf::FetchError,
) -> JobError {
    let m = format!("URL 抓取失败: {e}");
    // W-1/W-2（2026-09-04）：按错误类分治——
    // · HTTP 4xx（除 429）重试无意义 → Permanent + mark_failed（404 文档
    //   直接 failed，不再退避重试到 job dead 而文档永久卡 pending）；
    // · 429/5xx 瞬态 → Retryable 退避重试；
    // · 网络错误（连接/超时/流中断）保持 Retryable；
    // · Retryable 最后一试也 mark_failed——重试耗尽 job dead 前文档必须
    //   落终态，杜绝「job dead + 文档 pending」孤儿态。
    // SSRF 判定/协议/大小/DNS 类保持 Permanent，防恶意 URL 反复探测。
    let permanent = match &e {
        super::super::ssrf::FetchError::Status(c) => !matches!(c, 429) && *c < 500,
        super::super::ssrf::FetchError::Network(_) => false,
        _ => true,
    };
    let last_attempt = ctx.job.attempts >= ctx.job.max_attempts;
    if permanent || last_attempt {
        mark_doc_failed(ctx, lib, doc_id, &m).await;
    }
    if permanent {
        return doc_fail(doc_id, m);
    }
    ctx.emit(&m, None).await.ok();
    JobError::Retryable(m)
}

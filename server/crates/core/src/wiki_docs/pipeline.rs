//! 摄取管道 job handlers：parse → chunk → embed 三步链。
//!
//! 持久化在 `engram_storage::repo::wiki_docs`（job 内不再出现裸 SQL）。

use engram_jobs::JobContext;

mod steps;
use steps::*;

mod util;
use util::*;

use engram_jobs::types::{JobError, JobTemplate};
use engram_llm::ProviderRegistry;
use engram_llm::types::Purpose;
use engram_parsing::parse_bytes;
use engram_search::tokenize::tsv_text;
use engram_storage::repo::wiki_docs as repo;
use serde_json::json;
use std::path::PathBuf;
use uuid::Uuid;

use super::chunking::chunk_text;

/// 并发说明：管道并发由 RunnerConfig.concurrency（默认 4）全局约束，
/// 不在 job 内做 per-kind 限流（单用户规模下解析快，避免互相挤死的重试风暴）。
///
/// 入队摄取（幂等：sha 命中返回既有文档；K6 并发同 sha 无竞态、K2 入队失败回滚）。
pub async fn enqueue_ingest(
    queue: &engram_jobs::JobQueue,
    _registry: &ProviderRegistry,
    data_dir: &std::path::Path,
    lib: Uuid,
    source: IngestSource,
) -> Result<(Uuid, bool), WikiDocumentError> {
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
            h.update(normalize_url(url).as_bytes());
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
                .map_err(|e| WikiDocumentError::Storage(format!("创建 uploads 失败: {e}")))?;
            let safe_name = name.replace(['/', '\\'], "_");
            let path = uploads.join(format!("{id}_{safe_name}"));
            tokio::fs::write(&path, content)
                .await
                .map_err(|e| WikiDocumentError::Storage(format!("写文件失败: {e}")))?;
            (
                name.clone(),
                path.to_string_lossy().into_owned(),
                mime_from_name(name).or_else(|| content_type.clone()),
                name.clone(),
            )
        }
        IngestSource::Url(url) => (normalize_url(url), String::new(), None, normalize_url(url)),
    };

    // INSERT/幂等命中自愈 + 入队（失败回滚文档行与文件）
    insert_and_enqueue_document(queue, lib, id, &title, &source_uri, &raw_path, mime, &sha).await
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
pub enum WikiDocumentError {
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

    // 文档所属库（多库：全部 repo 调用按库收窄）
    let lib: Uuid = repo::document_library(pool, doc_id)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?
        .ok_or_else(|| JobError::Permanent(format!("文档 {doc_id} 不存在")))?;
    // 读取文档行（raw_path 可空：URL 文档摄取前无本地文件）
    let (source_uri, raw_path, mime) = repo::get_document_source(pool, lib, doc_id)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?
        .ok_or_else(|| JobError::Permanent(format!("文档 {doc_id} 不存在")))?;
    let raw_path = raw_path.unwrap_or_default();

    // 取字节：URL 抓取 or 本地文件
    // 取字节：URL 抓取 or 本地文件
    let (name, bytes, ctype) =
        fetch_document_bytes(&ctx, pool, &source_uri, &raw_path, mime, lib, doc_id).await?;

    // 状态 → parsing → 解析 → 存 extracted → 入队 chunk
    parse_and_store_document(&ctx, pool, lib, doc_id, name, bytes, ctype).await
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

    let lib: Uuid = repo::document_library(pool, doc_id)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?
        .ok_or_else(|| JobError::Permanent(format!("文档 {doc_id} 不存在")))?;
    repo::update_document_status(pool, lib, doc_id, "chunking")
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    let extracted_path = data_uploads().join(format!("{doc_id}.extracted.txt"));
    let text = tokio::fs::read_to_string(&extracted_path)
        .await
        .map_err(|e| JobError::Permanent(format!("extracted 文件缺失: {e}")))?;

    let chunks = chunk_text(&text);
    if chunks.is_empty() {
        repo::mark_failed_document(pool, lib, doc_id, "解析后内容为空")
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
        return Ok(json!({"document_id": doc_id, "chunks": 0, "empty": true}));
    }

    for c in &chunks {
        repo::insert_chunk(
            pool,
            lib,
            Uuid::now_v7(),
            doc_id,
            c.seq as i32,
            &c.content,
            &tsv_text(&c.content),
        )
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    }
    ctx.emit(&format!("分块 {} 块", chunks.len()), None)
        .await
        .ok();

    // 入队 embed
    ctx.enqueue_next(
        JobTemplate::new("embed_document")
            .with_payload(json!({"document_id": doc_id, "library_id": lib})),
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

    let lib: Uuid = repo::document_library(pool, doc_id)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?
        .ok_or_else(|| JobError::Permanent(format!("文档 {doc_id} 不存在")))?;
    repo::update_document_status(pool, lib, doc_id, "embedding")
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    // K8：只补缺失块（NULL 向量或 embed_failed）——重跑 / re-embed / Transient 重试
    // 的进度天然保留，已嵌入块不重复计费
    let chunks = repo::missing_chunks(pool, lib, doc_id)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    let total = repo::count_chunks(pool, lib, doc_id)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    let missing = chunks.len();

    // K8：只补缺失块（批量 ≤64/批，经记账门面），返回补嵌数
    let embedded =
        embed_missing_chunks(&ctx, pool, &registry, lib, doc_id, &chunks, missing).await?;

    repo::set_ready_document(pool, lib, doc_id)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    // 自动织入 Wiki：文档 ready 后织成互链页面（upload 文档读原文件重解析；
    // URL 文档 raw_path 空时用已分块文本拼接——2026-09-04 补，此前静默跳过）。
    // best-effort（sha256 去重 + 失败不影响文档 ready，页面层降级为空）。
    {
        let wiki = crate::wiki::WikiService::new(pool.clone(), registry.clone());
        let _ = wiki.ingest_document(lib, doc_id).await; // 有意忽略：织入 Wiki 是 best-effort（见上方注释）
    }

    // 清理 extracted 临时文件
    let _ = tokio::fs::remove_file(data_uploads().join(format!("{doc_id}.extracted.txt"))).await; // 有意忽略：best-effort 清理/建目录（失败由后续步骤或下次运行暴露）

    ctx.emit(
        &format!("文档 ready（补嵌 {embedded}/{missing}，共 {total} 块）"),
        None,
    )
    .await
    .ok();
    Ok(json!({"document_id": doc_id, "embedded": embedded, "missing": missing, "total": total}))
}

/// 注册 wiki 文档域 handlers（main 装配用）。
pub fn register_handlers(
    runner: engram_jobs::Runner,
    registry: ProviderRegistry,
) -> engram_jobs::Runner {
    let r1 = registry.clone();
    runner
        .register("parse_document", |ctx| async move { parse_job(ctx).await })
        .register("chunk_document", |ctx| async move { chunk_job(ctx).await })
        .register("embed_document", move |ctx| {
            let reg = r1.clone();
            async move { embed_job(ctx, reg).await }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_from_extension() {
        assert_eq!(mime_from_name("a.txt"), Some("text/plain".into()));
        assert_eq!(mime_from_name("a.md"), Some("text/markdown".into()));
        assert_eq!(mime_from_name("a.MARKDOWN"), Some("text/markdown".into()));
        assert_eq!(mime_from_name("a.html"), Some("text/html".into()));
        assert_eq!(mime_from_name("a.htm"), Some("text/html".into()));
        assert_eq!(mime_from_name("a.pdf"), Some("application/pdf".into()));
        assert_eq!(
            mime_from_name("a.docx"),
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document".into())
        );
        assert_eq!(mime_from_name("a.unknown"), None);
        assert_eq!(mime_from_name("noext"), None);
    }

    #[test]
    fn url_normalization_strips_tracking_and_fragment() {
        assert_eq!(
            normalize_url("https://a.com/x?utm_source=s&id=1#sec"),
            "https://a.com/x?id=1"
        );
        // 只去 utm_*，保留其他 query（内容相关参数不动）
        assert_eq!(
            normalize_url("https://a.com/x?q=rust&utm_medium=m"),
            "https://a.com/x?q=rust"
        );
        // 纯 tracking → query 全清
        assert_eq!(
            normalize_url("https://a.com/x?utm_source=s&utm_campaign=c"),
            "https://a.com/x"
        );
        // 无 query 不变
        assert_eq!(normalize_url("https://a.com/x"), "https://a.com/x");
        // 非法 URL 原样返回（不 panic）
        assert_eq!(normalize_url("not a url"), "not a url");
    }
}

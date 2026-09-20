//! 两步 ingest job handlers：wiki_analyze → wiki_generate。

use engram_jobs::JobContext;

mod generate;
mod pages;
pub use generate::*;
pub use pages::*;

use engram_jobs::types::{JobError, JobTemplate};
use serde_json::json;
use sha2::Digest;
use uuid::Uuid;

use crate::markup::{extract_wikilinks, is_valid_slug, normalize_wikilinks};
use crate::prompts;
use crate::service::folder_for_type;

/// 数据根解析——全 workspace 唯一收口（EN-47）。
///
/// `AGENT_MEMORY_DATA_DIR` 优先；未设（或空串）时 fallback 到 `~/.engram/app`
/// （宿主运行时家，与 engramctl 注入的值一致），无 HOME 才退 `./data`。
/// 任何 fallback 都会打 WARN（进程级一次）——**不再静默**：此前 5 处各自
/// `unwrap_or_else(|_| "./data")`，数据根随进程 cwd 漂移且无告警。
pub fn data_root() -> std::path::PathBuf {
    let (path, fallback) = resolve_data_root(
        std::env::var("AGENT_MEMORY_DATA_DIR").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    );
    if fallback {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            tracing::warn!(
                dir = %path.display(),
                "AGENT_MEMORY_DATA_DIR 未设——数据根 fallback（不再静默使用随 cwd 漂移的 ./data；\
                 宿主长期运行请显式设置环境变量）"
            );
        });
    }
    path
}

/// 纯逻辑：给定 env 值与 HOME，解析出数据根 + 是否发生 fallback（便于测试）。
fn resolve_data_root(env_val: Option<&str>, home: Option<&str>) -> (std::path::PathBuf, bool) {
    if let Some(d) = env_val.map(str::trim).filter(|d| !d.is_empty()) {
        return (std::path::PathBuf::from(d), false);
    }
    match home {
        Some(h) => (
            std::path::PathBuf::from(h).join(".engram").join("app"),
            true,
        ),
        None => (std::path::PathBuf::from("./data"), true),
    }
}

fn wiki_sources_dir() -> std::path::PathBuf {
    data_root().join("wiki-sources")
}

fn sha256_hex(b: &[u8]) -> String {
    let mut h = sha2::Sha256::new();
    h.update(b);
    h.finalize().iter().map(|x| format!("{x:02x}")).collect()
}

/// D27：织入提交的三态结果——此前「已完成跳过」与「在途处理中」都返回 skipped=true，
/// 调用方无从分辨、sha 去重又封锁了重试手段（在途窗口的任务表现为「丢失」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestOutcome {
    /// 同 sha 来源已就绪（曾成功织入）——幂等跳过
    AlreadyReady(Uuid),
    /// 同 sha 任务在途（pending/processing）——勿重复提交；携带在途 job id
    InFlight(Uuid, Option<Uuid>),
    /// 新入队（或失败后重入队）；携带 analyze job id
    Enqueued(Uuid, Uuid),
}

impl IngestOutcome {
    pub fn source_id(&self) -> Uuid {
        match self {
            IngestOutcome::AlreadyReady(id)
            | IngestOutcome::InFlight(id, _)
            | IngestOutcome::Enqueued(id, _) => *id,
        }
    }

    /// 进度查询通道（R8 观察 3）：GET /jobs/{job_id}（任意 scope 的 key 可读）或任务页
    pub fn job_id(&self) -> Option<Uuid> {
        match self {
            IngestOutcome::AlreadyReady(_) => None,
            IngestOutcome::InFlight(_, job) => *job,
            IngestOutcome::Enqueued(_, job) => Some(*job),
        }
    }

    /// 兼容旧布尔语义：仅「已就绪」算 skipped
    pub fn skipped(&self) -> bool {
        matches!(self, IngestOutcome::AlreadyReady(_))
    }
}

/// 入队 ingest：source 内容（复用 wiki 文档的解析产物文本或直接文本）。
/// sha 命中且已 ingest → 跳过（幂等）。多库（0037）：sha 去重按库隔离
/// （同内容可在不同库各织一份），source 落到指定库。
pub async fn enqueue_ingest(
    queue: &engram_jobs::JobQueue,
    lib: Uuid,
    title: &str,
    text: &str,
) -> Result<IngestOutcome, JobError> {
    let sha = sha256_hex(text.as_bytes());

    // 步骤 1：同 sha 去重（库内 sha256 唯一）——命中即早退
    if let Some(outcome) = dedup_outcome(queue, lib, &sha).await? {
        return Ok(outcome);
    }

    // 步骤 2：落不可变原料副本 + 登记 source 行（冲突时返回既有行 id）
    let (id, real_id) = store_raw_source(queue, lib, title, text, &sha).await?;

    // 步骤 3：W1 状态感知重入队（analyze 在途 → 秒跳过；analyze 成功 → 直发 generate）
    if let Some(outcome) = reenqueue_stale_source(queue, real_id, id, title).await? {
        return Ok(outcome);
    }

    let job = queue
        .enqueue(
            JobTemplate::new("wiki_analyze")
                .with_payload(json!({"source_id": real_id}))
                .with_idempotency_key(format!("wiki-analyze-{real_id}")),
        )
        .await?;
    Ok(IngestOutcome::Enqueued(real_id, job.id))
}

/// 步骤 1：同 sha 去重（D11 + D27 语义细化；库内生效）。
/// ready → AlreadyReady（跳过）/ pending|processing → InFlight（带在途 job id）/
/// failed → None（交下游 W1 状态感知重入队）。
async fn dedup_outcome(
    queue: &engram_jobs::JobQueue,
    lib: Uuid,
    sha: &str,
) -> Result<Option<IngestOutcome>, JobError> {
    let existing: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, status FROM wiki_sources WHERE sha256 = $1 AND library_id = $2")
            .bind(sha)
            .bind(lib)
            .fetch_optional(queue.pool())
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
    if let Some((id, status)) = existing {
        match status.as_str() {
            "ready" => return Ok(Some(IngestOutcome::AlreadyReady(id))),
            "pending" | "processing" => {
                // R8 观察 3：带上在途 job id，调用方可 GET /jobs/{id} 直查进度
                let job: Option<(Uuid,)> = sqlx::query_as(
                    "SELECT id FROM jobs                      WHERE payload->>'source_id' = $1 AND kind IN ('wiki_analyze','wiki_generate')                        AND status IN ('pending','running')                      ORDER BY created_at DESC LIMIT 1",
                )
                .bind(id.to_string())
                .fetch_optional(queue.pool())
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
                return Ok(Some(IngestOutcome::InFlight(id, job.map(|(j,)| j))));
            }
            _ => {} // failed → 走下方 W1 状态感知重入队
        }
    }
    Ok(None)
}

/// 步骤 2：原料落盘 + source 行 UPSERT（RETURNING id：sha 冲突时返回旧行 id，
/// payload 必须用它——曾因用新生成 uuid 导致 analyze 读不到原料行 no-rows 死循环）。
/// 返回 `(新 id, 实际入库 id)`。
async fn store_raw_source(
    queue: &engram_jobs::JobQueue,
    lib: Uuid,
    title: &str,
    text: &str,
    sha: &str,
) -> Result<(Uuid, Uuid), JobError> {
    // 落不可变原料副本
    let dir = wiki_sources_dir();
    let _ = tokio::fs::create_dir_all(&dir).await; // 有意忽略：目录已存在不算失败；后续写入会二次暴露真错误
    let id = Uuid::now_v7();
    let path = dir.join(format!("{id}.md"));
    tokio::fs::write(&path, text)
        .await
        .map_err(|e| JobError::Permanent(format!("写原料失败: {e}")))?;

    // RETURNING id：sha 冲突时返回旧行 id（payload 必须用它——曾因用新生成
    // uuid 导致 analyze 读不到原料行 no-rows 死循环，Phase 7 审计修复）。
    // 多库：冲突目标是 (library_id, sha256)
    let row = sqlx::query_as::<_, (Uuid,)>(
        "INSERT INTO wiki_sources (id, library_id, sha256, raw_path, title, status) \
         VALUES ($1, $2, $3, $4, $5, 'pending') \
         ON CONFLICT (library_id, sha256) \
         DO UPDATE SET title = EXCLUDED.title, status = 'pending' RETURNING id",
    )
    .bind(id)
    .bind(lib)
    .bind(sha)
    .bind(path.to_string_lossy().as_ref())
    .bind(title)
    .fetch_one(queue.pool())
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    let real_id = row.0;

    // 冲突行的新原料内容以返回的 id 落盘（read_source 按 id 找路径）
    if real_id != id {
        let _ = tokio::fs::remove_file(&path).await; // 有意忽略：冲突原料清理 best-effort
        let path = dir.join(format!("{real_id}.md"));
        tokio::fs::write(&path, text)
            .await
            .map_err(|e| JobError::Permanent(format!("写原料失败: {e}")))?;
        sqlx::query("UPDATE wiki_sources SET raw_path = $2 WHERE id = $1")
            .bind(real_id)
            .bind(path.to_string_lossy().as_ref())
            .execute(queue.pool())
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
    }
    Ok((id, real_id))
}

/// 步骤 3：W1 状态感知重入队——analyze 在途则跳过；analyze 已成功则直发 generate
/// （复用 payload 里的 analysis，省一次 LLM）；否则重跑 analyze。
/// 返回 Some(outcome) 表示已入队 / None 表示无需重入队（走常规 analyze 入队）。
async fn reenqueue_stale_source(
    queue: &engram_jobs::JobQueue,
    real_id: Uuid,
    id: Uuid,
    title: &str,
) -> Result<Option<IngestOutcome>, JobError> {
    // W1 状态感知重入队：source 非 ready 时查两步 job 实况——
    // analyze 已成功而 generate 缺失/终态失败 → 从失败 job 的 payload 取 analysis
    // 重入队 generate（新幂等键）；analyze 在途 → 真正的秒跳过。
    // 旧逻辑无条件入队 analyze（幂等键墙直接返回既有终态 job，链断即死锁）。
    if real_id != id {
        let jobs: Vec<(String, String, serde_json::Value)> = sqlx::query_as(
            "SELECT kind, status, payload FROM jobs \
             WHERE kind IN ('wiki_analyze','wiki_generate') \
               AND payload->>'source_id' = $1 \
             ORDER BY created_at DESC",
        )
        .bind(real_id.to_string())
        .fetch_all(queue.pool())
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        let has_active = jobs.iter().any(|(k, s, _)| {
            (k == "wiki_analyze" || k == "wiki_generate")
                && matches!(s.as_str(), "pending" | "running")
        });
        if !has_active {
            // generate 曾成功？source ready 已在上面早退；这里 generate 无终态成功 → 可安全重跑
            let generate_done = jobs
                .iter()
                .any(|(k, s, _)| k == "wiki_generate" && s == "succeeded");
            if !generate_done {
                // analyze 成功过 → analysis 在它链式入队的 generate job payload 里
                // （即使 generate 失败，payload 也带着 analysis——直发新 generate 省一次 LLM）
                let analyze_ok = jobs
                    .iter()
                    .any(|(k, s, _)| k == "wiki_analyze" && s == "succeeded");
                let analysis = jobs
                    .iter()
                    .find(|(k, _, _)| k == "wiki_generate")
                    .and_then(|(_, _, payload)| payload.get("analysis").cloned())
                    .filter(|a| a.as_object().is_some_and(|o| !o.is_empty()));
                if analyze_ok && let Some(analysis) = analysis {
                    // W4：failed/卡死源重置（generate 结束时会落 ready）
                    sqlx::query(
                        "UPDATE wiki_sources SET status = 'pending', error = NULL WHERE id = $1 AND status <> 'ready'",
                    )
                    .bind(real_id)
                    .execute(queue.pool())
                    .await
                    .map_err(|e| JobError::Retryable(e.to_string()))?;
                    // 注意必须 return：直发 generate 后不得坠落进下方 analyze 重跑分支
                    // （.map(...)? 只是求值丢弃，不是提前返回——曾因此多入队 analyze 抢走恢复用的 LLM 响应）
                    return queue
                        .enqueue(
                            JobTemplate::new("wiki_generate")
                                .with_payload(json!({
                                    "source_id": real_id,
                                    "analysis": analysis,
                                    "source_title": title,
                                }))
                                .with_idempotency_key(format!(
                                    "wiki-generate-{real_id}-{}",
                                    Uuid::now_v7().simple()
                                )),
                        )
                        .await
                        .map(|j| Some(IngestOutcome::Enqueued(real_id, j.id)));
                }
                // analyze 未成功或 analysis 不可得 → 重跑 analyze（原料文件刚重写过，可读）
                sqlx::query("UPDATE wiki_sources SET status = 'pending', error = NULL WHERE id = $1 AND status <> 'ready'")
                    .bind(real_id)
                    .execute(queue.pool())
                    .await
                    .map_err(|e| JobError::Retryable(e.to_string()))?;
                return queue
                    .enqueue(
                        JobTemplate::new("wiki_analyze")
                            .with_payload(json!({"source_id": real_id}))
                            .with_idempotency_key(format!(
                                "wiki-analyze-{real_id}-{}",
                                Uuid::now_v7().simple()
                            )),
                    )
                    .await
                    .map(|j| Some(IngestOutcome::Enqueued(real_id, j.id)));
            }
        }
    }
    Ok(None)
}

/// 第一步：分析。source 全文 + 既有 index → 结构化分析（存 wiki_sources.status + 事件）。
/// 多库（0037）：library_id 从 source 行取定，全流程只在库内读写。
pub async fn analyze_job(
    ctx: JobContext,
    llm: crate::service::LlmRef,
) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();
    let source_id: Uuid = ctx
        .job
        .payload
        .0
        .get("source_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| JobError::Permanent("payload 缺 source_id".into()))?;

    // 置 processing 的同时取源所属库（同批一次往返；源不存在此处即 RowNotFound）
    let lib: Uuid = sqlx::query_scalar(
        "UPDATE wiki_sources SET status = 'processing' WHERE id = $1 RETURNING library_id",
    )
    .bind(source_id)
    .fetch_one(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    let text = read_source(pool, source_id).await?;
    let index = read_index(pool, lib).await?;
    let purpose = crate::purpose::purpose_context(pool, lib).await;

    let user = format!(
        "== 知识库 Purpose（方向意图，分析时纳入考量）==\n{purpose}\n\n== 现有页面目录 ==\n{index}\n\n== 源文档 ==\n{text}"
    );
    let out = engram_distill::llm_port::chat_json_retrying(
        &ctx,
        llm.as_ref(),
        engram_llm::types::Purpose::WikiAnalysis,
        &prompts::analysis_system(),
        &user,
        ctx.job.id,
    )
    .await?;

    ctx.emit("分析完成", Some(out.clone())).await.ok();

    // review flag 落库（llm_wiki 异步人审：不阻塞 ingest）
    if let Some(flags) = out.get("reviews").and_then(|v| v.as_array()) {
        let parsed: Vec<crate::review::LlmReviewFlag> = flags
            .iter()
            .filter_map(|f| serde_json::from_value(f.clone()).ok())
            .collect();
        if !parsed.is_empty() {
            let n = crate::review::create_items(pool, lib, source_id, &parsed)
                .await?
                .len();
            ctx.emit(&format!("人审项 {n} 个已入队"), None).await.ok();
        }
    }

    // purpose 建议（llm_wiki：LLM 可建议更新 purpose——经人审队列，不直接改）。
    // 建议项挂源所属库（wiki_review_items.library_id）。
    if let Some(sugg) = out.get("purpose_suggestion").filter(|v| v.is_object()) {
        let pid = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO wiki_review_items (id, library_id, kind, payload, search_queries, source_id) \
             VALUES ($1, $2, 'flag', $3, '[]'::jsonb, $4)",
        )
        .bind(pid)
        .bind(lib)
        .bind(sqlx::types::Json(sugg))
        .bind(source_id)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        ctx.emit("purpose 更新建议已入人审队列", Some(sugg.clone()))
            .await
            .ok();
    }

    // 链式入队生成
    ctx.enqueue_next(
        JobTemplate::new("wiki_generate")
            .with_payload(json!({"source_id": source_id, "analysis": out, "source_title": title_of(pool, source_id).await?}))
            .with_idempotency_key(format!("wiki-generate-{source_id}")),
    )
    .await?;
    Ok(json!({"source_id": source_id}))
}

/// 按段落边界（\n\n）贪心切片；单段超限时硬切。
fn slice_source(text: &str, limit: usize) -> Vec<String> {
    if text.chars().count() <= limit {
        return vec![text.to_string()];
    }
    let mut slices = Vec::new();
    let mut cur = String::new();
    for para in text.split("\n\n") {
        let para_len = para.chars().count();
        if para_len > limit {
            // 单段超限：硬切
            if !cur.is_empty() {
                slices.push(std::mem::take(&mut cur));
            }
            let chars: Vec<char> = para.chars().collect();
            for chunk in chars.chunks(limit) {
                slices.push(chunk.iter().collect());
            }
            continue;
        }
        if cur.chars().count() + para_len + 2 > limit {
            slices.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push_str("\n\n");
        }
        cur.push_str(para);
    }
    if !cur.is_empty() {
        slices.push(cur);
    }
    slices
}

/// 注册 Wiki handler。
pub fn register_handlers(
    runner: engram_jobs::Runner,
    llm: crate::service::LlmRef,
    wiki: crate::service::WikiService,
) -> engram_jobs::Runner {
    let l1 = llm.clone();
    let l2 = llm.clone();
    let l3 = llm.clone();
    runner
        .register("wiki_analyze", move |ctx| {
            let llm = l1.clone();
            async move {
                let pool = ctx.pool().clone();
                let job = ctx.job.clone();
                let r = analyze_job(ctx, llm).await;
                if let Err(msg) = source_failure_msg(&job, &r) {
                    mark_source_failed(&pool, &job, &msg).await;
                }
                r
            }
        })
        .register("wiki_generate", move |ctx| {
            let llm = l2.clone();
            async move {
                let pool = ctx.pool().clone();
                let job = ctx.job.clone();
                let r = generate_job(ctx, llm).await;
                if let Err(msg) = source_failure_msg(&job, &r) {
                    mark_source_failed(&pool, &job, &msg).await;
                }
                r
            }
        })
        .register("wiki_lint_deep", move |ctx| {
            let llm = l3.clone();
            async move { crate::lint_deep::lint_deep_job(&ctx, &llm).await }
        })
        .register("wiki_repair", move |ctx| {
            let wiki = wiki.clone();
            async move {
                let payload = &ctx.job.payload.0;
                let lib: Uuid = payload
                    .get("library_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or_else(|| JobError::Permanent("payload 缺 library_id".into()))?;
                let report = wiki
                    .repair(lib)
                    .await
                    .map_err(|e| JobError::Permanent(e.to_string()))?;
                // 审计缺陷④：repair 顺带补嵌入（自愈入口——存量缺向量页 cap 50/次）
                let backfilled = wiki.backfill_embeddings(lib).await.unwrap_or(0);
                ctx.emit(
                    &format!(
                        "确定性修复完成：检查 {} 页，{} 项动作；向量回填 {} 页",
                        report.checked_pages,
                        report.actions.len(),
                        backfilled
                    ),
                    Some(serde_json::to_value(&report).unwrap_or_default()),
                )
                .await
                .ok();
                Ok(serde_json::to_value(&report).unwrap_or_default())
            }
        })
}

#[cfg(test)]
mod slice_tests {
    use super::{GEN_SLICE_CHARS, slice_source};

    #[test]
    fn small_text_single_slice() {
        assert_eq!(slice_source("短文本", GEN_SLICE_CHARS).len(), 1);
    }

    #[test]
    fn large_text_split_on_paragraphs() {
        let paras: Vec<String> = (0..50)
            .map(|i| format!("第{i}段。{}", "内容".repeat(300)))
            .collect();
        let text = paras.join("\n\n");
        let slices = slice_source(&text, GEN_SLICE_CHARS);
        assert!(slices.len() > 1, "应切多片");
        assert!(
            slices
                .iter()
                .all(|s| s.chars().count() <= GEN_SLICE_CHARS + 2),
            "单片不超限（+2 容忍拼接）"
        );
        // 无内容丢失（去分隔符拼回等长）
        let total: usize = slices.iter().map(|s| s.chars().count()).sum();
        let origin = text.chars().count();
        assert!(total >= origin - (slices.len() * 2), "切片不丢内容");
    }

    #[test]
    fn single_oversized_paragraph_hard_split() {
        let huge = "长".repeat(GEN_SLICE_CHARS * 2 + 100);
        let slices = slice_source(&huge, GEN_SLICE_CHARS);
        assert!(slices.len() >= 3, "单段超限硬切: {}", slices.len());
        assert!(slices.iter().all(|s| s.chars().count() <= GEN_SLICE_CHARS));
    }
}

#[cfg(test)]
mod data_root_tests {
    use super::resolve_data_root;

    #[test]
    fn env_设定时直接使用且不告警() {
        let (p, fallback) = resolve_data_root(Some("/var/data/engram"), Some("/home/x"));
        assert_eq!(p, std::path::PathBuf::from("/var/data/engram"));
        assert!(!fallback);
    }

    #[test]
    fn env_为空串视同未设() {
        let (p, fallback) = resolve_data_root(Some("   "), Some("/home/x"));
        assert_eq!(p, std::path::PathBuf::from("/home/x/.engram/app"));
        assert!(fallback);
    }

    #[test]
    fn env_未设有_home_fallback_到运行时家() {
        let (p, fallback) = resolve_data_root(None, Some("/home/x"));
        assert_eq!(p, std::path::PathBuf::from("/home/x/.engram/app"));
        assert!(fallback);
    }

    #[test]
    fn env_与_home_都未设_退回_cwd_相对路径() {
        let (p, fallback) = resolve_data_root(None, None);
        assert_eq!(p, std::path::PathBuf::from("./data"));
        assert!(fallback);
    }
}

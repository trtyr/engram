//! 两步 ingest job handlers：wiki_analyze → wiki_generate。

use agent_memory_jobs::JobContext;
use agent_memory_jobs::types::{JobError, JobTemplate};
use serde_json::json;
use sha2::Digest;
use uuid::Uuid;

use crate::markup::{extract_wikilinks, is_valid_slug, normalize_wikilinks};
use crate::prompts;
use crate::service::folder_for_type;

/// 单次织入建页量软上限（测试方 W-7③漂移校准）：超限截断+告警，防 llm 页自我漂移。
const MAX_PAGES_PER_GENERATE: usize = 20;

fn data_dir() -> std::path::PathBuf {
    std::env::var("AGENT_MEMORY_DATA_DIR")
        .unwrap_or_else(|_| "./data".into())
        .into()
}

fn wiki_sources_dir() -> std::path::PathBuf {
    data_dir().join("wiki-sources")
}

fn sha256_hex(b: &[u8]) -> String {
    let mut h = sha2::Sha256::new();
    h.update(b);
    h.finalize().iter().map(|x| format!("{x:02x}")).collect()
}

/// 入队 ingest：source 内容（复用 knowledge 的解析产物文本或直接文本）。
/// sha 命中且已 ingest → 跳过（幂等）。
pub async fn enqueue_ingest(
    queue: &agent_memory_jobs::JobQueue,
    title: &str,
    text: &str,
) -> Result<(Uuid, bool), JobError> {
    let sha = sha256_hex(text.as_bytes());
    // 已有同 sha 且成功 ingest 的原料 → 幂等跳过
    let existing: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, status FROM wiki_sources WHERE sha256 = $1")
            .bind(&sha)
            .fetch_optional(queue.pool())
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
    if let Some((id, status)) = existing
        && status == "ready"
    {
        return Ok((id, true));
    }

    // 落不可变原料副本
    let dir = wiki_sources_dir();
    let _ = tokio::fs::create_dir_all(&dir).await;
    let id = Uuid::now_v7();
    let path = dir.join(format!("{id}.md"));
    tokio::fs::write(&path, text)
        .await
        .map_err(|e| JobError::Permanent(format!("写原料失败: {e}")))?;

    // RETURNING id：sha 冲突时返回旧行 id（payload 必须用它——曾因用新生成
    // uuid 导致 analyze 读不到原料行 no-rows 死循环，Phase 7 审计修复）
    let row = sqlx::query_as::<_, (Uuid,)>(
        "INSERT INTO wiki_sources (id, sha256, raw_path, title, status) \
         VALUES ($1, $2, $3, $4, 'pending') \
         ON CONFLICT (sha256) DO UPDATE SET title = EXCLUDED.title, status = 'pending' RETURNING id",
    )
    .bind(id)
    .bind(&sha)
    .bind(path.to_string_lossy().as_ref())
    .bind(title)
    .fetch_one(queue.pool())
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    let real_id = row.0;

    // 冲突行的新原料内容以返回的 id 落盘（read_source 按 id 找路径）
    if real_id != id {
        let _ = tokio::fs::remove_file(&path).await;
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
                    queue
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
                        .await?;
                    return Ok((real_id, false));
                }
                // analyze 未成功或 analysis 不可得 → 重跑 analyze（原料文件刚重写过，可读）
                sqlx::query("UPDATE wiki_sources SET status = 'pending', error = NULL WHERE id = $1 AND status <> 'ready'")
                    .bind(real_id)
                    .execute(queue.pool())
                    .await
                    .map_err(|e| JobError::Retryable(e.to_string()))?;
                queue
                    .enqueue(
                        JobTemplate::new("wiki_analyze")
                            .with_payload(json!({"source_id": real_id}))
                            .with_idempotency_key(format!(
                                "wiki-analyze-{real_id}-{}",
                                Uuid::now_v7().simple()
                            )),
                    )
                    .await?;
                return Ok((real_id, false));
            }
        }
    }

    queue
        .enqueue(
            JobTemplate::new("wiki_analyze")
                .with_payload(json!({"source_id": real_id}))
                .with_idempotency_key(format!("wiki-analyze-{real_id}")),
        )
        .await?;
    Ok((real_id, false))
}

/// 第一步：分析。source 全文 + 既有 index → 结构化分析（存 wiki_sources.status + 事件）。
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

    sqlx::query("UPDATE wiki_sources SET status = 'processing' WHERE id = $1")
        .bind(source_id)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    let text = read_source(pool, source_id).await?;
    let index = read_index(pool).await?;
    let purpose = crate::purpose::purpose_context(pool).await;

    let user = format!(
        "== 知识库 Purpose（方向意图，分析时纳入考量）==\n{purpose}\n\n== 现有页面目录 ==\n{index}\n\n== 源文档 ==\n{text}"
    );
    let out = agent_memory_distill::llm_port::chat_json_retrying(
        &ctx,
        llm.as_ref(),
        agent_memory_llm::types::Purpose::WikiAnalysis,
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
            let n = crate::review::create_items(pool, source_id, &parsed)
                .await?
                .len();
            ctx.emit(&format!("人审项 {n} 个已入队"), None).await.ok();
        }
    }

    // purpose 建议（llm_wiki：LLM 可建议更新 purpose——经人审队列，不直接改）
    if let Some(sugg) = out.get("purpose_suggestion").filter(|v| v.is_object()) {
        let pid = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO wiki_review_items (id, kind, payload, search_queries, source_id) \
             VALUES ($1, 'flag', $2, '[]'::jsonb, $3)",
        )
        .bind(pid)
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

/// 第二步：生成/更新页面 + 索引 + 链接图 + 嵌入。
pub async fn generate_job(
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
    let analysis = ctx
        .job
        .payload
        .0
        .get("analysis")
        .cloned()
        .ok_or_else(|| JobError::Permanent("payload 缺 analysis".into()))?;
    let source_title = ctx
        .job
        .payload
        .0
        .get("source_title")
        .and_then(|v| v.as_str())
        .unwrap_or("未命名源")
        .to_string();

    let text = read_source(pool, source_id).await?;
    let existing_pages = read_index(pool).await?;

    let purpose = crate::purpose::purpose_context(pool).await;
    let user = format!(
        "== 知识库 Purpose（方向意图，写作风格与侧重纳入考量）==\n{purpose}\n\n== 分析结果 ==\n{}\n\n== 源文档 ==\n{}\n\n== 既有页面集合（已存在，勿重建）==\n{}",
        serde_json::to_string_pretty(&analysis).unwrap_or_default(),
        text,
        existing_pages
    );
    let out = agent_memory_distill::llm_port::chat_json_retrying(
        &ctx,
        llm.as_ref(),
        agent_memory_llm::types::Purpose::WikiGeneration,
        &prompts::generation_system(),
        &user,
        ctx.job.id,
    )
    .await?;

    let mut pages = out
        .get("pages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    // 漂移校准（测试方 W-7③）：单次织入建页量软上限——超限截断 + 告警，不中断织入。
    // 防 LLM 幻觉/暴走批量建页导致 llm 页自我漂移；正常多主题文档（<20 页）不受影响。
    if pages.len() > MAX_PAGES_PER_GENERATE {
        let dropped = pages.len() - MAX_PAGES_PER_GENERATE;
        pages.truncate(MAX_PAGES_PER_GENERATE);
        ctx.emit(
            "建页量超软上限，已截断",
            Some(json!({
                "max_pages": MAX_PAGES_PER_GENERATE,
                "dropped_pages": dropped,
                "hint": "源可能过广或 purpose 需收紧——考虑拆分源或人工 review",
            })),
        )
        .await
        .ok();
    }
    let mut created = 0usize;
    let mut updated = 0usize;
    let mut proposals = 0usize;
    let mut all_slugs: Vec<String> = Vec::new();

    // 链接规范化：查全库 slug 建 lowercase → real 映射，生成时对齐大小写变体
    // （防 case_mismatch 落到事后 lint；只修大小写，不补死链）
    let existing_slugs: Vec<String> = sqlx::query_scalar("SELECT slug FROM wiki_pages")
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    let lower_slug_map: std::collections::HashMap<String, String> = existing_slugs
        .into_iter()
        .map(|s| (s.to_lowercase(), s))
        .collect();

    for p in &pages {
        let slug = p
            .get("slug")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let page_type = p
            .get("page_type")
            .and_then(|v| v.as_str())
            .unwrap_or("concept");
        let title = p
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or(&slug)
            .trim()
            .to_string();
        let content = p
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let content = normalize_wikilinks(&content, &lower_slug_map);
        if !is_valid_slug(&slug) || content.is_empty() {
            continue;
        }
        all_slugs.push(slug.clone());

        let fm = json!({
            "title": title,
            "page_type": page_type,
            "sources": [source_id.to_string()],
            "origin_if_new": "llm",
        });

        // W6：单语句 UPSERT——消除 check-then-act 竞态（并发 generate 不再撞
        // slug UNIQUE，也不会双 UPDATE 互相覆盖）。human 页保护语义收进
        // DO UPDATE 的 WHERE：冲突且 origin=human 时子句为假 → RETURNING 无行 → 提案路径。
        let upserted: Option<bool> = sqlx::query_scalar(
            "INSERT INTO wiki_pages (id, slug, title, page_type, content, frontmatter, origin, version, folder) \
             VALUES ($1, $2, $3, $4, $5, $6::jsonb, 'llm', 1, $8) \
             ON CONFLICT (slug) DO UPDATE SET \
                content = $5, \
                folder = CASE WHEN wiki_pages.folder = '' THEN $8 ELSE wiki_pages.folder END, \
                frontmatter = jsonb_set(wiki_pages.frontmatter, '{sources}', \
                    (SELECT COALESCE(jsonb_agg(DISTINCT s), '[]'::jsonb) FROM \
                        (SELECT jsonb_array_elements_text(wiki_pages.frontmatter->'sources') AS s \
                         UNION ALL SELECT $7::text) sub)), \
                version = wiki_pages.version + 1, updated_at = now() \
             WHERE wiki_pages.origin = 'llm' \
             RETURNING (xmax = 0)",
        )
        .bind(Uuid::now_v7())
        .bind(&slug)
        .bind(&title)
        .bind(page_type)
        .bind(&content)
        .bind(sqlx::types::Json(&fm))
        .bind(source_id.to_string())
        .bind(folder_for_type(page_type))
        .fetch_optional(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

        match upserted {
            Some(true) => created += 1,
            Some(false) => updated += 1,
            None => {
                // 冲突且 origin=human：不覆盖 → 提案（内容存事件流，待人工合入）
                proposals += 1;
                ctx.emit(
                    "人工页面更新提案（待审核）",
                    Some(json!({
                        "page_slug": slug,
                        "proposal_content": content,
                        "current_version_note": "人工编辑页，需 UI 确认后合入",
                    })),
                )
                .await
                .ok();
            }
        }
    }

    // 链接图：本批页面的 wikilinks + 到既有页的边
    rebuild_links(pool, &all_slugs).await?;

    // 索引 + 日志 + overview 维护
    update_index_and_log(pool, source_id, &source_title, created, updated, proposals).await?;

    // 新/变页索引与嵌入（W2/W3 重构）
    // W2：tsv 统一 title+content 口径（旧实现只嵌 slug——LLM 页内容词搜不到）；
    // W3：tsv 写入与嵌入解耦——嵌入失败只丢向量不丢 FTS 索引，页面不再从检索消失
    let pages: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT slug, COALESCE(frontmatter->>'title', slug), content FROM wiki_pages WHERE slug = ANY($1)",
    )
    .bind(&all_slugs)
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    let mut tsv_written = 0usize;
    for (slug, title, content) in &pages {
        let text = format!("{title}\n{content}");
        sqlx::query("UPDATE wiki_pages SET tsv = to_tsvector('simple', $2) WHERE slug = $1")
            .bind(slug)
            .bind(agent_memory_search::tokenize::tsv_text(&text))
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
        tsv_written += 1;
    }

    let texts: Vec<String> = pages
        .iter()
        .map(|(_, title, content)| format!("{title}\n{content}"))
        .collect();
    let mut embedded_pages = 0usize;
    if !texts.is_empty() {
        match llm.embed(&texts, ctx.job.id).await {
            Ok(emb) => {
                // K4 守卫（移植）：响应数量/维度与批次不符 → 拒绝写入，
                // 不再静默跳过部分页（短响应旁路）
                if emb.len() != texts.len() || emb.iter().any(|v| v.len() != 1024) {
                    tracing::warn!(
                        source = %source_id,
                        expected = texts.len(),
                        got = emb.len(),
                        "wiki 页嵌入响应与批次不符，本批向量全部放弃（FTS 检索不受影响）"
                    );
                    ctx.emit(
                        "嵌入响应异常，页面保留 FTS 检索（向量缺失）",
                        Some(json!({"expected": texts.len(), "got": emb.len()})),
                    )
                    .await
                    .ok();
                } else {
                    for (i, (slug, _, _)) in pages.iter().enumerate() {
                        sqlx::query("UPDATE wiki_pages SET embedding = $2 WHERE slug = $1")
                            .bind(slug)
                            .bind(pgvector::Vector::from(emb[i].clone()))
                            .execute(pool)
                            .await
                            .map_err(|e| JobError::Retryable(e.to_string()))?;
                        embedded_pages += 1;
                    }
                }
            }
            Err(e) => {
                // W3：嵌入失败不阻塞——页面已入库、tsv 已写，向量可由下次更新或重跑补
                tracing::warn!(error = %e, source = %source_id, "wiki 页嵌入失败（FTS 检索不受影响）");
                ctx.emit(
                    "嵌入失败，页面保留 FTS 检索（向量缺失）",
                    Some(json!({"error": e.to_string()})),
                )
                .await
                .ok();
            }
        }
    }
    let _ = tsv_written;
    let _ = embedded_pages;

    sqlx::query("UPDATE wiki_sources SET status = 'ready', last_ingested_at = now() WHERE id = $1")
        .bind(source_id)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    // overview.md 重生成 + 4 信号权重重算（llm_wiki：每次 ingest 后全局状态更新）
    if created + updated > 0 {
        rebuild_overview_page(pool).await?;
        let n = crate::relevance::rebuild_weights(pool).await?;
        ctx.emit(&format!("相关性权重更新 {n} 条边"), None)
            .await
            .ok();
    }

    ctx.emit(
        &format!(
            "Wiki 生成：新建 {created} / 更新 {updated} / 提案 {proposals}（{embedded_pages} 页已嵌入）"
        ),
        Some(json!({"created": created, "updated": updated, "proposals": proposals, "embedded": embedded_pages})),
    )
    .await
    .ok();

    Ok(
        json!({"created": created, "updated": updated, "proposals": proposals, "source_id": source_id}),
    )
}

async fn title_of(pool: &sqlx::PgPool, id: Uuid) -> Result<String, JobError> {
    let t: Option<Option<String>> =
        sqlx::query_scalar("SELECT title FROM wiki_sources WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(t.flatten().unwrap_or_else(|| "未命名源".into()))
}

async fn read_source(pool: &sqlx::PgPool, id: Uuid) -> Result<String, JobError> {
    let path: String = sqlx::query_scalar("SELECT raw_path FROM wiki_sources WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| JobError::Permanent(format!("读原料失败: {e}")))
}

async fn read_index(pool: &sqlx::PgPool) -> Result<String, JobError> {
    let pages: Vec<(String, String)> = sqlx::query_as(
        "SELECT slug, COALESCE(frontmatter->>'title', slug) FROM wiki_pages ORDER BY updated_at DESC LIMIT 200",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(pages
        .into_iter()
        .map(|(slug, title)| format!("- [[{slug}]] {title}"))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// 重建给定页面的出边（wikilink 边）。W6：包事务——与并发 generate 的
/// 链接重建交错时不再留 DELETE/INSERT 半态。
async fn rebuild_links(pool: &sqlx::PgPool, slugs: &[String]) -> Result<(), JobError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    for slug in slugs {
        let content: Option<String> =
            sqlx::query_scalar("SELECT content FROM wiki_pages WHERE slug = $1")
                .bind(slug)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?
                .flatten();
        let Some(content) = content else { continue };
        sqlx::query("DELETE FROM wiki_links WHERE from_slug = $1")
            .bind(slug)
            .execute(&mut *tx)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
        for target in extract_wikilinks(&content) {
            sqlx::query(
                "INSERT INTO wiki_links (from_slug, to_slug, weight) VALUES ($1, $2, 3.0) \
                 ON CONFLICT (from_slug, to_slug) DO NOTHING",
            )
            .bind(slug)
            .bind(&target)
            .execute(&mut *tx)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
        }
    }
    tx.commit()
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(())
}

/// index/log/overview 系统页维护。
async fn update_index_and_log(
    pool: &sqlx::PgPool,
    source_id: Uuid,
    source_title: &str,
    created: usize,
    updated: usize,
    proposals: usize,
) -> Result<(), JobError> {
    rebuild_index_page(pool).await?;

    // log：append 一行（存 latest 系统页；全量历史靠 job_events）
    let log_line = format!(
        "- {} ingest `{}` → 新建 {created} / 更新 {updated} / 提案 {proposals}",
        chrono::Utc::now().format("%Y-%m-%d %H:%M"),
        source_title
    );
    let prev_log: Option<String> = sqlx::query_scalar(
        "SELECT content FROM wiki_pages WHERE slug = 'log' AND page_type = 'log'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?
    .flatten();
    let log_md = format!(
        "{}\n{}",
        prev_log.unwrap_or_else(|| "# 操作日志\n".into()),
        log_line
    );
    // W7：截最近 500 行——log 不再无界增长（全量历史由 job_events 承担）
    let lines: Vec<&str> = log_md.lines().collect();
    let log_md = if lines.len() > 500 {
        lines[lines.len() - 500..].join("\n")
    } else {
        log_md
    };

    upsert_system_page(pool, "log", "log", "日志", &log_md).await?;
    let _ = source_id;
    Ok(())
}

/// 重建 index 系统页（cascade 删除后同步复用）。
pub async fn rebuild_index_page(pool: &sqlx::PgPool) -> Result<(), JobError> {
    let pages: Vec<(String, String)> = sqlx::query_as(
        "SELECT slug, COALESCE(frontmatter->>'title', slug) FROM wiki_pages \
         WHERE page_type NOT IN ('index','log') ORDER BY updated_at DESC LIMIT 300",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    let index_md = format!(
        "# 知识库索引\n\n{}\n",
        pages
            .iter()
            .map(|(s, t)| format!("- [[{s}]] — {t}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    upsert_system_page(pool, "index", "index", "索引", &index_md).await?;
    Ok(())
}

/// 重建 overview 系统页：全局摘要（每页一行标题+summary），ingest 后调用。
pub async fn rebuild_overview_page(pool: &sqlx::PgPool) -> Result<(), JobError> {
    let pages: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT slug, COALESCE(frontmatter->>'title', slug), left(content, 120)          FROM wiki_pages WHERE page_type NOT IN ('index','log','overview')          ORDER BY updated_at DESC LIMIT 100",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    let body = pages
        .iter()
        .map(|(slug, title, excerpt)| format!("## [[{slug}]] {title}\n\n{excerpt}…"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let overview_md = format!(
        "# Overview\n\n知识库当前包含 {} 个页面。\n\n{body}\n",
        pages.len()
    );
    upsert_system_page(pool, "overview", "overview", "总览", &overview_md).await?;
    Ok(())
}

async fn upsert_system_page(
    pool: &sqlx::PgPool,
    slug: &str,
    page_type: &str,
    title: &str,
    content: &str,
) -> Result<(), JobError> {
    sqlx::query(
        "INSERT INTO wiki_pages (id, slug, title, page_type, content, frontmatter, origin, version, folder) \
         VALUES ($1, $2, $3, $4, $5, '{}'::jsonb, 'llm', 1, '系统') \
         ON CONFLICT (slug) DO UPDATE SET content = $5, updated_at = now()",
    )
    .bind(Uuid::now_v7())
    .bind(slug)
    .bind(title)
    .bind(page_type)
    .bind(content)
    .execute(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(())
}

/// W4：Permanent 失败 → wiki_sources 标 failed + error 落列（此前 'failed' 态全代码无人写）。
async fn mark_source_failed(
    pool: &sqlx::PgPool,
    ctx_job: &agent_memory_jobs::types::Job,
    msg: &str,
) {
    let Some(sid) = ctx_job
        .payload
        .0
        .get("source_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
    else {
        return;
    };
    let _ = sqlx::query("UPDATE wiki_sources SET status = 'failed', error = $2 WHERE id = $1")
        .bind(sid)
        .bind(msg)
        .execute(pool)
        .await;
}

/// 注册 Wiki handler。
pub fn register_handlers(
    runner: agent_memory_jobs::Runner,
    llm: crate::service::LlmRef,
) -> agent_memory_jobs::Runner {
    let l1 = llm.clone();
    let l2 = llm.clone();
    runner
        .register("wiki_analyze", move |ctx| {
            let llm = l1.clone();
            async move {
                let pool = ctx.pool().clone();
                let job = ctx.job.clone();
                let r = analyze_job(ctx, llm).await;
                if let Err(agent_memory_jobs::types::JobError::Permanent(msg)) = &r {
                    mark_source_failed(&pool, &job, msg).await;
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
                if let Err(agent_memory_jobs::types::JobError::Permanent(msg)) = &r {
                    mark_source_failed(&pool, &job, msg).await;
                }
                r
            }
        })
}

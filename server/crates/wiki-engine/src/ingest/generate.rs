//! `ingest` 的实现切片（架构治理 2026-09-20：自 ingest.rs 纯搬移，零行为变化）。

use super::*;

/// 单次织入建页量软上限（测试方 W-7③漂移校准）：超限截断+告警，防 llm 页自我漂移。
pub(super) const MAX_PAGES_PER_GENERATE: usize = 20;

/// 第二步：生成/更新页面 + 索引 + 链接图 + 嵌入。
/// 单片字符上限：超过则按段落边界切片分别调用（防超时+巨 JSON）。
pub(super) const GEN_SLICE_CHARS: usize = 24_000;

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

    // 多库（0037）：取源所属库，全流程只在库内读写（源不存在此处即报错）
    let lib: Uuid = sqlx::query_scalar("SELECT library_id FROM wiki_sources WHERE id = $1")
        .bind(source_id)
        .fetch_one(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    let text = read_source(pool, source_id).await?;
    let existing_pages = read_index(pool, lib).await?;

    let purpose = crate::purpose::purpose_context(pool, lib).await;
    // 分片消费（工单「高反斜杠大文档织入必败」）：整篇塞给 LLM 会超时+产出巨 JSON 易坏
    // ——超过 GEN_SLICE_CHARS 按段落边界切片，每片独立调用，聚合候选页后走既有去重/截断链
    let slices = slice_source(&text, GEN_SLICE_CHARS);
    if slices.len() > 1 {
        ctx.emit(
            "源文档过大，分片织入",
            Some(json!({
                "total_chars": text.chars().count(),
                "slices": slices.len(),
                "slice_chars": GEN_SLICE_CHARS,
            })),
        )
        .await
        .ok();
    }
    let mut pages: Vec<serde_json::Value> = Vec::new();
    let mut seen_slugs: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (i, slice) in slices.iter().enumerate() {
        let part_note = if slices.len() > 1 {
            format!(
                "\n\n（本批为源文档第 {}/{} 片——只产出与本片内容对应的页面）",
                i + 1,
                slices.len()
            )
        } else {
            String::new()
        };
        let user = format!(
            "== 知识库 Purpose（方向意图，写作风格与侧重纳入考量）==\n{purpose}\n\n== 分析结果 ==\n{}\n\n== 源文档 ==\n{}{}\n\n== 既有页面集合（已存在，勿重建）==\n{}",
            serde_json::to_string_pretty(&analysis).unwrap_or_default(),
            slice,
            part_note,
            existing_pages
        );
        let out = engram_distill::llm_port::chat_json_retrying(
            &ctx,
            llm.as_ref(),
            engram_llm::types::Purpose::WikiGeneration,
            &prompts::generation_system(),
            &user,
            ctx.job.id,
        )
        .await?;
        let batch = out
            .get("pages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for p in batch {
            // 聚合去重：同 slug 只保留首次产出（分片边界重复内容不产重复页）
            let slug = p
                .get("slug")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if seen_slugs.insert(slug) {
                pages.push(p);
            }
        }
    }
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

    // 链接规范化：查库内 slug 建 lowercase → real 映射，生成时对齐大小写变体
    // （防 case_mismatch 落到事后 lint；只修大小写，不补死链）
    let existing_slugs: Vec<String> =
        sqlx::query_scalar("SELECT slug FROM wiki_pages WHERE library_id = $1")
            .bind(lib)
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
        // 多库：冲突目标 (library_id, slug)，页面落到源所属库
        let upserted: Option<bool> = sqlx::query_scalar(
            "INSERT INTO wiki_pages (id, library_id, slug, title, page_type, content, frontmatter, origin, version, folder) \
             VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb, 'llm', 1, $9) \
             ON CONFLICT (library_id, slug) DO UPDATE SET \
                content = $6, \
                folder = CASE WHEN wiki_pages.folder = '' THEN $9 ELSE wiki_pages.folder END, \
                frontmatter = jsonb_set(wiki_pages.frontmatter, '{sources}', \
                    (SELECT COALESCE(jsonb_agg(DISTINCT s), '[]'::jsonb) FROM \
                        (SELECT jsonb_array_elements_text(wiki_pages.frontmatter->'sources') AS s \
                         UNION ALL SELECT $8::text) sub)), \
                version = wiki_pages.version + 1, updated_at = now() \
             WHERE wiki_pages.origin = 'llm' \
             RETURNING (xmax = 0)",
        )
        .bind(Uuid::now_v7())
        .bind(lib)
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

    // 链接图：本批页面的 wikilinks + 到既有页的边（库内）
    rebuild_links(pool, lib, &all_slugs).await?;

    // 索引 + 日志 + overview 维护
    update_index_and_log(
        pool,
        lib,
        source_id,
        &source_title,
        created,
        updated,
        proposals,
    )
    .await?;

    // 新/变页索引与嵌入（W2/W3 重构；库内回读）
    // W2：tsv 统一 title+content 口径（旧实现只嵌 slug——LLM 页内容词搜不到）；
    // W3：tsv 写入与嵌入解耦——嵌入失败只丢向量不丢 FTS 索引，页面不再从检索消失
    let pages: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT slug, COALESCE(frontmatter->>'title', slug), content FROM wiki_pages \
         WHERE slug = ANY($1) AND library_id = $2",
    )
    .bind(&all_slugs)
    .bind(lib)
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    for (slug, title, content) in &pages {
        let text = engram_search::tokenize::tsv_text_wiki(&format!("{slug} {title} {content}"));
        sqlx::query("UPDATE wiki_pages SET tsv = to_tsvector('simple', $3) WHERE slug = $1 AND library_id = $2")
            .bind(slug)
            .bind(lib)
            .bind(&text)
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
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
                if emb.len() != texts.len()
                    || emb.iter().any(|v| {
                        v.len() != engram_distill::llm_port::embedding_dimensions() as usize
                    })
                {
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
                        sqlx::query(
                            "UPDATE wiki_pages SET embedding = $3 WHERE slug = $1 AND library_id = $2",
                        )
                        .bind(slug)
                        .bind(lib)
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

    sqlx::query("UPDATE wiki_sources SET status = 'ready', last_ingested_at = now() WHERE id = $1")
        .bind(source_id)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    // overview.md 重生成 + 4 信号权重重算（llm_wiki：每次 ingest 后全局状态更新；按库）
    if created + updated > 0 {
        rebuild_overview_page(pool, lib).await?;
        let n = crate::relevance::rebuild_weights(pool, lib).await?;
        ctx.emit(&format!("相关性权重更新 {n} 条边"), None)
            .await
            .ok();
    }

    // 批次③ 社区摘要层（wiki 大库化）：Louvain 社区 → synthesis 综述页参与召回。
    // hash 守卫增量（成员不变不重调 LLM）；失败只告警不阻塞织入主流程。
    if created + updated > 0 {
        match crate::community_summaries::refresh_community_summaries(pool, &llm, lib, &ctx).await {
            Ok(stats) => {
                let c = stats.get("created").and_then(|v| v.as_i64()).unwrap_or(0);
                let d = stats.get("deleted").and_then(|v| v.as_i64()).unwrap_or(0);
                if c + d > 0 {
                    ctx.emit(&format!("社区摘要层更新：新建 {c} / 清理 {d}"), Some(stats))
                        .await
                        .ok();
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "社区摘要刷新失败（织入主流程不受影响）");
                ctx.emit("社区摘要刷新失败", Some(json!({"error": e.to_string()})))
                    .await
                    .ok();
            }
        }
        // 审计缺陷④自愈：存量页向量回填（cap 50/次）——嵌入失败丢失的页在后续织入中自动补全
        match crate::service::backfill_page_embeddings(pool, Some(&llm), lib).await {
            Ok(n) if n > 0 => {
                ctx.emit(&format!("存量页向量回填 {n} 页"), None).await.ok();
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "存量页向量回填失败（织入主流程不受影响）");
            }
        }
    }

    let thin_hint = if created + updated + proposals == 0 {
        "（0 产物：内容较薄，LLM 未产出页面——status=ready 仅代表处理完成，不代表有产物）"
    } else {
        ""
    };
    ctx.emit(
        &format!(
            "Wiki 生成：新建 {created} / 更新 {updated} / 提案 {proposals}（{embedded_pages} 页已嵌入）{thin_hint}"
        ),
        Some(json!({"created": created, "updated": updated, "proposals": proposals, "embedded": embedded_pages})),
    )
    .await
    .ok();

    Ok(
        json!({"created": created, "updated": updated, "proposals": proposals, "source_id": source_id}),
    )
}

/// W4：Permanent 失败 → wiki_sources 标 failed + error 落列（此前 'failed' 态全代码无人写）。
pub(super) async fn mark_source_failed(
    pool: &sqlx::PgPool,
    ctx_job: &engram_jobs::types::Job,
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
    // 失败态必须落库；写失败要可见（否则原料永远停在 pending）
    if let Err(e) =
        sqlx::query("UPDATE wiki_sources SET status = 'failed', error = $2 WHERE id = $1")
            .bind(sid)
            .bind(msg)
            .execute(pool)
            .await
    {
        tracing::warn!(source = %sid, error = %e, "标记 source 失败态写库失败");
    }
    // 织入失败可见（工单「write_page 哑写」②）：失败落人审队列 flag（via=ingest_failed）——
    // 人能在 reviews 里看到「这条原料织不进来」，而不是只有 source 表里一行 failed
    let row: Option<(Uuid, Option<String>)> =
        sqlx::query_as("SELECT library_id, title FROM wiki_sources WHERE id = $1")
            .bind(sid)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    if let Some((lib, title)) = row {
        // 让「织不进来」在日志里可见（失败不再静默）
        if let Err(e) = sqlx::query(
            "INSERT INTO wiki_review_items (id, library_id, kind, payload, search_queries, source_id) \
             VALUES ($1, $2, 'flag', $3, '[]'::jsonb, $4)",
        )
        .bind(Uuid::now_v7())
        .bind(lib)
        .bind(serde_json::json!({
            "via": "ingest_failed",
            "source_id": sid,
            "title": title,
            "reason": format!("织入失败：{msg}——修复后可重新 ingest 同内容（sha 变更即重试）"),
        }))
        .bind(sid)
        .execute(pool)
        .await
        {
            tracing::warn!(source = %sid, error = %e, "落人审 flag 失败（原料织不进来这件事将不可见）");
        }
    }
}

/// source 标 failed 的判定：Permanent 立即标；Retryable 在末次尝试（重试耗尽将转 dead）也标，
/// 否则 source 永卡 processing 无人报错。
pub(super) fn source_failure_msg(
    job: &engram_jobs::types::Job,
    r: &Result<serde_json::Value, JobError>,
) -> Result<(), String> {
    match r {
        Err(JobError::Permanent(msg)) => Err(msg.clone()),
        Err(JobError::Retryable(msg)) if job.attempts >= job.max_attempts => {
            Err(format!("重试耗尽（{msg}）"))
        }
        _ => Ok(()),
    }
}

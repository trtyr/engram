use super::*;

/// 候选页产出：分片消费 → 每片 LLM 织入 → 聚合去重 → 建页量软上限截断。
pub(super) async fn generate_candidates(
    ctx: &JobContext,
    llm: &crate::service::LlmRef,
    analysis: &serde_json::Value,
    text: &str,
    existing_pages: &str,
    purpose: &str,
) -> Result<Vec<serde_json::Value>, JobError> {
    // 分片消费（工单「高反斜杠大文档织入必败」）：整篇塞给 LLM 会超时+产出巨 JSON 易坏
    // ——超过 GEN_SLICE_CHARS 按段落边界切片，每片独立调用，聚合候选页后走既有去重/截断链
    let slices = slice_source(text, GEN_SLICE_CHARS);
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
            ctx,
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
    Ok(pages)
}

/// 候选页落库：slug 规范化 → 单语句 UPSERT（human 页冲突转提案）→ 计数与 slug 清单。
pub(super) async fn upsert_generated_pages(
    ctx: &JobContext,
    pool: &sqlx::PgPool,
    lib: Uuid,
    source_id: Uuid,
    pages: &[serde_json::Value],
) -> Result<(usize, usize, usize, Vec<String>), JobError> {
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

    for p in pages {
        let Some(c) = normalize_candidate(p, &lower_slug_map, source_id) else {
            continue;
        };
        all_slugs.push(c.slug.clone());
        match upsert_candidate_page(pool, lib, source_id, &c).await? {
            Some(true) => created += 1,
            Some(false) => updated += 1,
            None => {
                // 冲突且 origin=human：不覆盖 → 提案（内容存事件流，待人工合入）
                proposals += 1;
                ctx.emit(
                    "人工页面更新提案（待审核）",
                    Some(json!({
                        "page_slug": c.slug,
                        "proposal_content": c.content,
                        "current_version_note": "人工编辑页，需 UI 确认后合入",
                    })),
                )
                .await
                .ok();
            }
        }
    }
    Ok((created, updated, proposals, all_slugs))
}

/// 规范化单个候选页：取字段 → wikilinks 大小写对齐 → 校验（非法 slug / 空内容 → None）。
pub(super) fn normalize_candidate(
    p: &serde_json::Value,
    lower_slug_map: &std::collections::HashMap<String, String>,
    source_id: Uuid,
) -> Option<CandidatePage> {
    let slug = p
        .get("slug")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let page_type = p
        .get("page_type")
        .and_then(|v| v.as_str())
        .unwrap_or("concept")
        .to_string();
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
    let content = normalize_wikilinks(&content, lower_slug_map);
    if !is_valid_slug(&slug) || content.is_empty() {
        return None;
    }
    let frontmatter = json!({
        "title": title,
        "page_type": page_type,
        "sources": [source_id.to_string()],
        "origin_if_new": "llm",
    });
    Some(CandidatePage {
        slug,
        title,
        page_type,
        content,
        frontmatter,
    })
}

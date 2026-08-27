//! extract：L0 会话 → 候选 L1 原子。

use agent_memory_jobs::JobContext;
use agent_memory_jobs::types::{JobError, JobTemplate};
use serde_json::json;
use uuid::Uuid;

use crate::llm_port::LlmRef;
use crate::prompts;

/// 原始会话行（extract 内部用）。
struct SessionRow {
    id: Uuid,
    agent: String,
    content: serde_json::Value,
}

pub async fn run(ctx: JobContext, llm: LlmRef) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();

    // 1. 认领待蒸馏会话（processing 中防重复认领）
    let sessions: Vec<SessionRow> = sqlx::query_as::<_, (Uuid, String, serde_json::Value)>(
        "UPDATE raw_sessions SET distill_status = 'processing' \
         WHERE distill_status = 'pending' RETURNING id, agent, content",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?
    .into_iter()
    .map(|(id, agent, content)| SessionRow { id, agent, content })
    .collect();

    if sessions.is_empty() {
        return Ok(json!({"session_ids": [], "candidate_ids": []}));
    }
    let session_ids: Vec<Uuid> = sessions.iter().map(|s| s.id).collect();

    // 失败回滚：任何 Err 都把认领会话退回 pending（重试不丢数据）
    match run_claimed(ctx.clone(), llm, sessions).await {
        Ok(v) => Ok(v),
        Err(e) => {
            let revert = sqlx::query(
                "UPDATE raw_sessions SET distill_status = 'pending' \
                 WHERE id = ANY($1) AND distill_status = 'processing'",
            )
            .bind(&session_ids)
            .execute(ctx.pool())
            .await;
            if let Err(re) = revert {
                tracing::error!(error = %re, "会话状态回滚失败（将滞留 processing，需人工处理）");
            }
            Err(e)
        }
    }
}

/// 分段字符预算（B1 覆盖率）：段内拼多轮，超预算开新段；单段一次 LLM 调用。
const SEGMENT_CHARS: usize = 6000;

struct SegmentLine {
    text: String,
    is_header: bool,
}

async fn run_claimed(
    ctx: JobContext,
    llm: LlmRef,
    sessions: Vec<SessionRow>,
) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();
    let session_ids: Vec<Uuid> = sessions.iter().map(|s| s.id).collect();

    // 2. 构建行列表（会话头 + 全局编号轮次）→ 按预算贪心分段
    //    B1：一批会话全拼一个 prompt 时，超上下文的中间轮次会被模型静默丢弃；
    //    分段后每段独立调用，段级事件留痕，覆盖率可审计。
    let mut lines: Vec<SegmentLine> = Vec::new();
    let mut turn_map: Vec<Uuid> = Vec::new(); // 轮次编号(1-based) → session_id
    for s in &sessions {
        lines.push(SegmentLine {
            text: format!("— 会话 {}（agent: {}）—", s.id, s.agent),
            is_header: true,
        });
        if let Some(turns) = s.content.as_array() {
            for t in turns {
                let speaker = t.get("speaker").and_then(|v| v.as_str()).unwrap_or("?");
                let text = t.get("text").and_then(|v| v.as_str()).unwrap_or("");
                turn_map.push(s.id);
                lines.push(SegmentLine {
                    text: format!("[{}] {}: {}", turn_map.len(), speaker, text),
                    is_header: false,
                });
            }
        }
    }

    // 贪心打包：会话头不落单（开新段时若末行是头，连带迁去新段）
    let mut segments: Vec<Vec<SegmentLine>> = vec![Vec::new()];
    let mut used = 0usize;
    for line in lines {
        let len = line.text.len() + 1;
        if !segments.last().unwrap().is_empty() && used + len > SEGMENT_CHARS {
            let mut new_seg: Vec<SegmentLine> = Vec::new();
            // 头不落单：上一段末行若是会话头，迁移到新段首
            if segments.last().unwrap().last().map(|l| l.is_header).unwrap_or(false) {
                if let Some(header) = segments.last_mut().unwrap().pop() {
                    used -= header.text.len() + 1;
                    new_seg.push(header);
                }
            }
            segments.push(new_seg);
            used = 0;
        }
        used += len;
        segments.last_mut().unwrap().push(line);
    }
    segments.retain(|s| !s.is_empty());
    let total_segments = segments.len();

    // 3. 逐段抽取（每段独立 LLM 调用，段级事件留痕；空段显式确认「无持久洞察」）
    let mut texts: Vec<String> = Vec::new();
    let mut pending: Vec<(String, String, f32, serde_json::Value)> = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        let user = seg.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
        let out = crate::llm_port::chat_json_retrying(
            &ctx,
            llm.as_ref(),
            agent_memory_llm::types::Purpose::Extract,
            &prompts::extract_system(),
            &user,
            ctx.job.id,
        )
        .await?;

        let atoms = out
            .get("atoms")
            .and_then(|a| a.as_array())
            .cloned()
            .unwrap_or_default();
        let seg_count = atoms.len();
        for a in &atoms {
            let kind = a
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("fact")
                .to_string();
            let content = a
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if content.is_empty() || content.chars().count() > 120 {
                continue;
            }
            let confidence = a
                .get("confidence")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.7)
                .clamp(0.0, 1.0) as f32;
            let refs = a
                .get("turn_refs")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|n| n.as_u64().map(|i| (i as usize).checked_sub(1)))
                        .flatten()
                        .filter_map(|i| turn_map.get(i).copied())
                        .map(|sid| json!({"session_id": sid}))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            texts.push(content.clone());
            pending.push((kind, content, confidence, json!(refs)));
        }
        if seg_count == 0 {
            ctx.emit(
                &format!("分段抽取 {}/{}：无持久洞察（显式确认）", i + 1, total_segments),
                None,
            )
            .await
            .ok();
        } else {
            ctx.emit(
                &format!("分段抽取 {}/{}：{} 条候选", i + 1, total_segments, seg_count),
                None,
            )
            .await
            .ok();
        }
    }

    let mut candidate_ids = Vec::new();
    if !pending.is_empty() {
        let embeddings = llm.embed(&texts, ctx.job.id).await?;
        for (i, (kind, content, confidence, refs)) in pending.into_iter().enumerate() {
            let id = Uuid::now_v7();
            let needs_review = confidence < 0.55;
            sqlx::query(
                "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, needs_review, embedding, tsv)
                 VALUES ($1, $2, $3, $4, 'candidate', $5, $6, $7, to_tsvector('simple', $8))",
            )
            .bind(id)
            .bind(&kind)
            .bind(&content)
            .bind(confidence)
            .bind(sqlx::types::Json(&refs))
            .bind(needs_review)
            // B2：嵌入缺失或全零（上游异常）一律置 NULL——零向量会污染余弦近邻
            .bind(
                embeddings
                    .get(i)
                    .filter(|v| v.iter().any(|&x| x != 0.0))
                    .map(|v| pgvector::Vector::from(v.clone())),
            )
            .bind(agent_memory_search::tokenize::tsv_text(&content))
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
            candidate_ids.push(id);
        }
    }

    // 4. 会话标记完成
    sqlx::query("UPDATE raw_sessions SET distill_status = 'done' WHERE id = ANY($1)")
        .bind(&session_ids)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    ctx.emit(
        &format!(
            "抽取 {} 会话 → {} 候选",
            session_ids.len(),
            candidate_ids.len()
        ),
        None,
    )
    .await
    .ok();

    // 5. 链式入队仲裁
    if !candidate_ids.is_empty() {
        ctx.enqueue_next(
            JobTemplate::new("arbitrate_atoms")
                .with_payload(json!({"candidate_ids": candidate_ids})),
        )
        .await?;
    }

    Ok(json!({"session_ids": session_ids, "candidate_ids": candidate_ids}))
}

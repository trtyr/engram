//! extract：L0 会话 → 候选 L1 原子。

use agent_memory_jobs::JobContext;
use agent_memory_jobs::types::{JobError, JobTemplate};
use serde_json::json;
use std::fmt::Write as _;
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

async fn run_claimed(
    ctx: JobContext,
    llm: LlmRef,
    sessions: Vec<SessionRow>,
) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();
    let session_ids: Vec<Uuid> = sessions.iter().map(|s| s.id).collect();

    // 2. 构造 user 提示：轮次全局编号 → session 映射
    let mut numbered = String::new();
    let mut turn_map: Vec<Uuid> = Vec::new(); // 轮次编号(1-based) → session_id
    for s in &sessions {
        writeln!(numbered, "— 会话 {}（agent: {}）—", s.id, s.agent).ok();
        if let Some(turns) = s.content.as_array() {
            for t in turns {
                let speaker = t.get("speaker").and_then(|v| v.as_str()).unwrap_or("?");
                let text = t.get("text").and_then(|v| v.as_str()).unwrap_or("");
                turn_map.push(s.id);
                writeln!(numbered, "[{}] {}: {}", turn_map.len(), speaker, text).ok();
            }
        }
    }

    let out = crate::llm_port::chat_json_retrying(
        &ctx,
        llm.as_ref(),
        agent_memory_llm::types::Purpose::Extract,
        &prompts::extract_system(),
        &numbered,
        ctx.job.id,
    )
    .await?;

    // 3. 解析候选 → 入库（candidate 状态）
    let atoms = out
        .get("atoms")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();
    let mut texts: Vec<String> = Vec::new();
    let mut pending: Vec<(String, String, f32, serde_json::Value)> = Vec::new();
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
            .bind(pgvector::Vector::from(embeddings.get(i).cloned().unwrap_or_default()))
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

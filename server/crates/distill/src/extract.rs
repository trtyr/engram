//! extract：L0 会话 → 候选 L1 原子。
//!
//! 分步（架构治理 2026-09-20）：认领会话（失败回滚）→ 分段 → 逐段 LLM 抽取
//! → 落库（含实体挂链）→ 关系落库 → 会话标记完成 → 链式入队仲裁。
//! 纯逻辑（分段打包、解析、白名单）在 `extract_model`，可独立单测。

use engram_jobs::JobContext;
use engram_jobs::types::{JobError, JobTemplate};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

use crate::extract_model::{
    PendingAtom, SegmentLine, SessionRow, build_segments, parse_atoms, parse_relations,
};
use crate::llm_port::LlmRef;
use crate::prompts;

pub async fn run(ctx: JobContext, llm: LlmRef) -> Result<serde_json::Value, JobError> {
    let sessions = claim_pending_sessions(&ctx).await?;
    if sessions.is_empty() {
        // P-B（直写重建死路）：无待蒸馏会话 ≠ 无事可做——直写原子（scenario_id NULL）
        // 也要进场景聚类。organize 自带空输入幂等（收敛扫描 + 未归组原子），空转成本可忽略。
        ctx.enqueue_next(JobTemplate::new("organize_scenarios"))
            .await?;
        return Ok(json!({"session_ids": [], "candidate_ids": [], "chained_organize": true}));
    }
    let session_ids: Vec<Uuid> = sessions.iter().map(|s| s.id).collect();

    // 失败回滚：任何 Err 都把认领会话退回 pending（重试不丢数据）
    match run_claimed(ctx.clone(), llm, sessions).await {
        Ok(v) => Ok(v),
        Err(e) => {
            revert_sessions(&ctx, &session_ids).await;
            Err(e)
        }
    }
}

/// 1. 认领待蒸馏会话（processing 中防重复认领）。
///
/// v2 修复（H-A2）：`metadata.distill=off` 的会话**永久豁免**——off 是会话级语义，
/// 不再被后续任何 extract 任务的 pending 全量扫描顺带蒸掉。
async fn claim_pending_sessions(ctx: &JobContext) -> Result<Vec<SessionRow>, JobError> {
    let rows = sqlx::query_as::<_, (Uuid, String, serde_json::Value, bool, serde_json::Value)>(
        "UPDATE raw_sessions SET distill_status = 'processing' \
         WHERE distill_status = 'pending' AND COALESCE(metadata->>'distill','') <> 'off' \
         RETURNING id, agent, content, sensitive, metadata",
    )
    .fetch_all(ctx.pool())
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|(id, agent, content, sensitive, metadata)| SessionRow {
            id,
            agent,
            content,
            sensitive,
            metadata,
        })
        .collect())
}

async fn revert_sessions(ctx: &JobContext, session_ids: &[Uuid]) {
    let revert = sqlx::query(
        "UPDATE raw_sessions SET distill_status = 'pending' \
         WHERE id = ANY($1) AND distill_status = 'processing'",
    )
    .bind(session_ids)
    .execute(ctx.pool())
    .await;
    if let Err(re) = revert {
        tracing::error!(error = %re, "会话状态回滚失败（将滞留 processing，需人工处理）");
    }
}

async fn run_claimed(
    ctx: JobContext,
    llm: LlmRef,
    sessions: Vec<SessionRow>,
) -> Result<serde_json::Value, JobError> {
    let session_ids: Vec<Uuid> = sessions.iter().map(|s| s.id).collect();
    // 会话敏感标记映射：任一轮次来源会话标 sensitive → 产物继承敏感
    let session_sensitive: HashMap<Uuid, bool> =
        sessions.iter().map(|s| (s.id, s.sensitive)).collect();

    let (segments, turn_map) = build_segments(&sessions);
    let total_segments = segments.len();

    let mut pending: Vec<PendingAtom> = Vec::new();
    let mut all_relations: Vec<(String, String, String)> = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        let (atoms, rels) = extract_segment(&ctx, llm.as_ref(), seg, i + 1, total_segments).await?;
        all_relations.extend(rels);
        pending.extend(parse_atoms(&atoms, &turn_map, &session_sensitive));
    }

    let candidate_ids = persist_atoms(&ctx, llm.as_ref(), pending).await?;
    persist_relations(&ctx, &all_relations).await;
    mark_sessions_done(&ctx, &session_ids).await?;

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

    enqueue_arbitrate(&ctx, &candidate_ids).await?;
    Ok(json!({"session_ids": session_ids, "candidate_ids": candidate_ids}))
}

/// 单段抽取：一次 LLM 调用 + 段级事件留痕（空段显式确认「无持久洞察」）。
async fn extract_segment(
    ctx: &JobContext,
    llm: &dyn crate::llm_port::DistillLlm,
    seg: &[SegmentLine],
    index: usize,
    total: usize,
) -> Result<(serde_json::Value, Vec<(String, String, String)>), JobError> {
    let user = seg
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let out = crate::llm_port::chat_json_retrying(
        ctx,
        llm,
        engram_llm::types::Purpose::Extract,
        &prompts::extract_system(),
        &user,
        ctx.job.id,
    )
    .await?;
    let seg_count = out
        .get("atoms")
        .and_then(|a| a.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    if seg_count == 0 {
        ctx.emit(
            &format!("分段抽取 {index}/{total}：无持久洞察（显式确认）"),
            None,
        )
        .await
        .ok();
    } else {
        ctx.emit(
            &format!("分段抽取 {index}/{total}：{seg_count} 条候选"),
            None,
        )
        .await
        .ok();
    }
    Ok((out.clone(), parse_relations(&out)))
}

/// 3. 落库：嵌入 + 插入候选原子 + 实体挂链（失败不阻断主链）。
async fn persist_atoms(
    ctx: &JobContext,
    llm: &dyn crate::llm_port::DistillLlm,
    pending: Vec<PendingAtom>,
) -> Result<Vec<Uuid>, JobError> {
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    let pool = ctx.pool();
    let texts: Vec<String> = pending.iter().map(|p| p.content.clone()).collect();
    let embeddings = llm.embed(&texts, ctx.job.id).await?;
    let mut candidate_ids = Vec::with_capacity(pending.len());
    for (i, p) in pending.into_iter().enumerate() {
        let id = Uuid::now_v7();
        let needs_review = p.confidence < 0.55;
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, needs_review, occurred_at, valid_until, sensitive, embedding, tsv, strength, source_kind)
             VALUES ($1, $2, $3, $4, 'candidate', $5, $6, $7, $8, $9, $10, to_tsvector('simple', $11), $12, 'agent_inferred')",
        )
        .bind(id)
        .bind(&p.kind)
        .bind(&p.content)
        .bind(p.confidence)
        .bind(sqlx::types::Json(&p.refs))
        .bind(needs_review)
        .bind(p.occurred_at)
        .bind(p.valid_until)
        .bind(p.sensitive)
        // B2：嵌入缺失或全零（上游异常）一律置 NULL——零向量会污染余弦近邻
        .bind(
            embeddings
                .get(i)
                .filter(|v| v.iter().any(|&x| x != 0.0))
                .map(|v| pgvector::Vector::from(v.clone())),
        )
        .bind(engram_search::tokenize::tsv_text(&p.content))
        .bind(&p.strength)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        for (name, kind) in &p.entities {
            if let Err(e) = link_entity(pool, id, name, kind).await {
                tracing::warn!(error = %e, entity = %name, "实体挂链失败（不影响原子产出）");
            }
        }
        candidate_ids.push(id);
    }
    Ok(candidate_ids)
}

/// 实体挂链：同名同类活体复用（部分唯一索引），否则新建。
/// v2 修复（N5）：新建前先做**名字包含**近似归并——LLM 对同一实体常给出措辞
/// 繁简不同的称呼（「星云」/「星云项目」），互为子串且同 kind 即视为同一实体，
/// 挂到既有实体上，不再各建一个空档案。
async fn link_entity(
    pool: &sqlx::PgPool,
    atom_id: Uuid,
    name: &str,
    kind: &str,
) -> Result<(), String> {
    let similar: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM entities WHERE kind = $2 AND merged_into IS NULL AND archived_at IS NULL \
         AND (position(lower($1) in lower(name)) > 0 OR position(lower(name) in lower($1)) > 0) LIMIT 1",
    )
    .bind(name)
    .bind(kind)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let eid: Uuid = match similar {
        Some(eid) => eid,
        None => {
            sqlx::query(
                "INSERT INTO entities (id, name, kind) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            )
            .bind(Uuid::now_v7())
            .bind(name)
            .bind(kind)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
            sqlx::query_scalar(
                "SELECT id FROM entities WHERE lower(btrim(name)) = lower(btrim($1)) AND kind = $2 \
                 AND merged_into IS NULL AND archived_at IS NULL",
            )
            .bind(name)
            .bind(kind)
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?
        }
    };
    sqlx::query(
        "INSERT INTO atom_entities (atom_id, entity_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(atom_id)
    .bind(eid)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    // 复活语义已废除（审计五驳）：近似查询只命中活体（merged_into/archived_at 双排除）——
    // eid 恒为活体；归档/墓碑行不再被挂原子、不再被清 archived_at。
    // 抽取要「并」的是活档，不是唤醒死档；同名检测与合并 = 命中活体即复用其 id（幂等）。
    Ok(())
}

/// 关系落库：实体已在挂链里 upsert，name→id 解析后建关系（best-effort，不阻断主链）。
async fn persist_relations(ctx: &JobContext, relations: &[(String, String, String)]) {
    let pool = ctx.pool();
    for (from, to, rel_type) in relations {
        let fid = lookup_entity(pool, from).await;
        let tid = lookup_entity(pool, to).await;
        let (Some(fid), Some(tid)) = (fid, tid) else {
            continue;
        };
        if let Err(e) = sqlx::query(
            "INSERT INTO entity_relations (id, from_id, to_id, rel_type, weight, source) \
             VALUES ($1, $2, $3, $4, 1, 'distill') \
             ON CONFLICT (from_id, to_id, rel_type) DO UPDATE SET weight = entity_relations.weight + 1, updated_at = now()",
        )
        .bind(Uuid::now_v7())
        .bind(fid)
        .bind(tid)
        .bind(rel_type)
        .execute(pool)
        .await
        {
            tracing::warn!(error = %e, "关系落库失败（不影响主链）");
        }
    }
}

async fn lookup_entity(pool: &sqlx::PgPool, name: &str) -> Option<Uuid> {
    sqlx::query_scalar(
        "SELECT id FROM entities WHERE name = $1 AND merged_into IS NULL ORDER BY created_at DESC LIMIT 1",
    )
    .bind(name)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// 4. 会话标记完成。
async fn mark_sessions_done(ctx: &JobContext, session_ids: &[Uuid]) -> Result<(), JobError> {
    sqlx::query("UPDATE raw_sessions SET distill_status = 'done' WHERE id = ANY($1)")
        .bind(session_ids)
        .execute(ctx.pool())
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(())
}

/// 5. 链式入队仲裁。
async fn enqueue_arbitrate(ctx: &JobContext, candidate_ids: &[Uuid]) -> Result<(), JobError> {
    if !candidate_ids.is_empty() {
        ctx.enqueue_next(
            JobTemplate::new("arbitrate_atoms")
                .with_payload(json!({"candidate_ids": candidate_ids})),
        )
        .await?;
    }
    Ok(())
}

//! organize：未归组 L1 → L2 场景（P012 agentic 唯一形态）。
//!
//! 流程：快照收敛（确定性代码，scenario_converge.rs）→ agentic 六工具循环
//! （organize_agentic.rs，模型自主探索决策）→ 场景补嵌 → 链式画像 → 实体画像。
//! `run()` 只做编排。旧单发路径已删除（用户拍板 2026-10-03：不做旧版兼容）。

use engram_jobs::JobContext;
use engram_jobs::types::{JobError, JobTemplate};
use serde_json::json;
use uuid::Uuid;

use crate::llm_port::LlmRef;
use crate::scenario_converge::{converge_snapshots, emit_converge};

pub async fn run(ctx: JobContext, llm: LlmRef) -> Result<serde_json::Value, JobError> {
    let mut converge = converge_snapshots(&ctx, llm.as_ref()).await?;
    emit_converge(&ctx, &converge).await;

    // F4 治：converge_only=true → 只跑收敛段（归档/标敏感触发的刷新），不进主组织流程
    if payload_bool(&ctx, "converge_only") {
        return converge_only_reply(&ctx, &mut converge).await;
    }

    let atoms = fetch_atoms(&ctx).await?;
    if atoms.is_empty() {
        return no_atoms_reply(&ctx, &mut converge).await;
    }

    // P012：agentic 六工具循环（唯一实现——旧单发路径已删除）
    let out = crate::organize_agentic::run_agentic(&ctx, llm.as_ref(), atoms.len() as i64).await?;
    // agentic 写过的场景补 embedding（内容从库读——工具层不携带全文）
    refresh_embeddings_by_ids(&ctx, llm.as_ref(), &out.touched).await?;
    // retire 释放的表述随 persona 链明确剔除（F4 治——与 converge 同通道）
    let mut all_touched = out.touched;
    converge.removed_texts.extend(out.removed_texts);

    ctx.emit(&format!("组织：{} 个场景有变动", all_touched.len()), None)
        .await
        .ok();

    for sid in converge.touched {
        if !all_touched.contains(&sid) {
            all_touched.push(sid);
        }
    }
    enqueue_persona(&ctx, &all_touched, &mut converge.removed_texts).await?;

    // v2（N5）：实体画像随 organize 链路生成——新实体的 summary 不再等 full consolidate。
    // 小限额 best-effort，失败不影响主链。
    let portraits = match crate::entity_portraits::fill_entity_portraits(&ctx, &llm, 5).await {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(error = %e, "organize 末尾实体画像生成失败（跳过）");
            0
        }
    };

    Ok(json!({"scenario_ids": all_touched, "portraits": portraits}))
}

fn payload_bool(ctx: &JobContext, key: &str) -> bool {
    ctx.job
        .payload
        .0
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

// ---------- 主组织流程 ----------

/// 原子（payload 指定 ∪ 少量历史未归组，防漏；敏感原子照常进组织素材——P001 决策 001）。
async fn fetch_atoms(ctx: &JobContext) -> Result<Vec<(Uuid, String, String)>, JobError> {
    let ids = payload_uuids(ctx, "atom_ids");
    sqlx::query_as(
        "SELECT id, kind, content FROM atoms \
         WHERE status = 'active' AND (id = ANY($1) OR scenario_id IS NULL) \
         ORDER BY created_at LIMIT 300",
    )
    .bind(&ids)
    .fetch_all(ctx.pool())
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))
}

fn payload_uuids(ctx: &JobContext, key: &str) -> Vec<Uuid> {
    ctx.job
        .payload
        .0
        .get(key)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect()
        })
        .unwrap_or_default()
}

/// agentic 路径的场景重嵌：按 id 集从库读文本 → embed → 写回 embedding。
/// embed 失败容忍（失败时保持旧 embedding，留待下次巡逻/重嵌补）。
async fn refresh_embeddings_by_ids(
    ctx: &JobContext,
    llm: &dyn crate::llm_port::DistillLlm,
    sids: &[Uuid],
) -> Result<(), JobError> {
    if sids.is_empty() {
        return Ok(());
    }
    let pool = ctx.pool();
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, topic || E'\n' || summary || E'\n' || body FROM scenarios \
         WHERE id = ANY($1) AND retired_at IS NULL",
    )
    .bind(sids)
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    if rows.is_empty() {
        return Ok(());
    }
    let texts: Vec<String> = rows.iter().map(|(_, t)| t.clone()).collect();
    let embeddings = llm.embed(&texts, ctx.job.id).await.unwrap_or_default();
    for (i, (sid, _text)) in rows.iter().enumerate() {
        let emb = embeddings.get(i).cloned().map(pgvector::Vector::from);
        sqlx::query("UPDATE scenarios SET embedding = COALESCE($2, embedding) WHERE id = $1")
            .bind(sid)
            .bind(emb)
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
    }
    Ok(())
}

/// 链式入队画像（L2 有变动时）：removed_texts 排序去重截断后随 payload 传给 persona。
async fn enqueue_persona(
    ctx: &JobContext,
    scenario_ids: &[Uuid],
    removed_texts: &mut Vec<String>,
) -> Result<(), JobError> {
    if scenario_ids.is_empty() {
        return Ok(());
    }
    removed_texts.sort();
    removed_texts.dedup();
    removed_texts.truncate(40);
    ctx.enqueue_next(JobTemplate::new("distill_persona").with_payload(json!({
        "scenario_ids": scenario_ids,
        "removed_texts": removed_texts,
    })))
    .await?;
    Ok(())
}

/// converge_only 快速通道：只跑收敛段（归档/标敏感触发的刷新），不进主组织流程。
async fn converge_only_reply(
    ctx: &JobContext,
    converge: &mut crate::scenario_converge::Converge,
) -> Result<serde_json::Value, JobError> {
    if !converge.touched.is_empty() {
        enqueue_persona(ctx, &converge.touched, &mut converge.removed_texts).await?;
    }
    Ok(json!({
        "scenario_ids": converge.touched,
        "converged": true,
        "converge_only": true
    }))
}

/// 无新原子：仍让收敛结果链下去（解散/重算同样该触发画像刷新）。
async fn no_atoms_reply(
    ctx: &JobContext,
    converge: &mut crate::scenario_converge::Converge,
) -> Result<serde_json::Value, JobError> {
    // 无新原子也要让收敛结果链下去（解散/重算同样该触发画像刷新）
    if !converge.touched.is_empty() {
        enqueue_persona(ctx, &converge.touched, &mut converge.removed_texts).await?;
    }
    Ok(json!({"scenario_ids": converge.touched, "converged": true}))
}

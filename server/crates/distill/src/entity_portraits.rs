//! 实体档案生成（v2 N5）：有原子且摘要滞后的实体 → LLM 聚合切片为一句档案。
//!
//! 架构治理 2026-09-20 自 consolidate.rs 摘出（独立关注点：实体摘要补写，
//! 与「近重复合并 / 关系回溯 / stale 降权」正交）。consolidate 用 limit=10，
//! organize 链路末尾用 limit=5（新实体摘要随蒸馏自动填上，不再等 full consolidate）。

use engram_jobs::JobContext;
use engram_jobs::types::JobError;
use uuid::Uuid;

use crate::llm_port::{DistillLlm, LlmRef};
use crate::prompts;

/// 批量补写实体档案，返回成功条数（单条失败 warn 跳过，不影响主链）。
pub async fn fill_entity_portraits(
    ctx: &JobContext,
    llm: &LlmRef,
    limit: usize,
) -> Result<usize, JobError> {
    let pool = ctx.pool();
    let limit = limit as i64;
    let candidates: Vec<(Uuid, String, String)> = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT e.id, e.name, e.kind FROM entities e \
         JOIN atom_entities ae ON ae.entity_id = e.id \
         JOIN atoms a ON a.id = ae.atom_id \
         WHERE e.merged_into IS NULL AND NOT e.manually_edited \
         GROUP BY e.id, e.name, e.kind, e.summary, e.updated_at \
         HAVING count(ae.atom_id) >= 1 AND (e.summary = '' OR max(a.created_at) > e.updated_at) \
         ORDER BY count(ae.atom_id) DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    let mut portraits = 0usize;
    for (eid, name, kind) in &candidates {
        let atoms: Vec<String> = sqlx::query_scalar(
            "SELECT a.content FROM atoms a JOIN atom_entities ae ON ae.atom_id = a.id \
             WHERE ae.entity_id = $1 AND a.status = 'active' AND NOT a.sensitive \
         ORDER BY a.created_at DESC LIMIT 20",
        )
        .bind(eid)
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        if atoms.is_empty() {
            continue;
        }
        let user = format!(
            "实体：{name}（{kind}）\n\n涉及记忆：\n{}",
            atoms
                .iter()
                .map(|c| format!("- {c}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        match write_entity_portrait(ctx, llm.as_ref(), *eid, &user).await {
            Ok(true) => portraits += 1,
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(error = %e, entity = %name, "实体档案生成失败（跳过，不影响主链）");
            }
        }
    }
    Ok(portraits)
}

/// 单个实体的档案生成：模型没给摘要 → 跳过（不算失败）。
async fn write_entity_portrait(
    ctx: &JobContext,
    llm: &dyn DistillLlm,
    eid: Uuid,
    user: &str,
) -> Result<bool, JobError> {
    let out = crate::llm_port::chat_json_retrying(
        ctx,
        llm,
        engram_llm::types::Purpose::Consolidate,
        &prompts::entity_portrait_system(),
        user,
        ctx.job.id,
    )
    .await?;
    let summary = out
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if summary.is_empty() {
        return Ok(false); // 模型没给 → 跳过，不算失败
    }
    sqlx::query("UPDATE entities SET summary = $2, updated_at = now() WHERE id = $1")
        .bind(eid)
        .bind(&summary)
        .execute(ctx.pool())
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(true)
}

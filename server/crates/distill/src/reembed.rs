//! reembed_memory：补嵌记忆域缺失向量（换 embedding 供应商后的修复路径）。
//!
//! atoms（active）+ scenarios 的 NULL embedding 批量补算。这是用户显式触发的
//! 修复动作——无 provider 时明确失败（与蒸馏链 best-effort 跳过语义相反）。

use agent_memory_jobs::JobContext;
use agent_memory_jobs::types::JobError;
use uuid::Uuid;

use crate::llm_port::LlmRef;

/// 单批嵌入条数（对齐知识域 embed_job 的批次粒度）。
const BATCH: usize = 64;

pub async fn run(ctx: JobContext, llm: LlmRef) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();

    // 原子：active 且无向量（可检索语料）
    let atoms: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, content FROM atoms WHERE status = 'active' AND embedding IS NULL \
         ORDER BY created_at LIMIT 1000",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    // 场景：topic + summary 拼接作为嵌入文本（对齐 organize 写入时的口径）
    let scenarios: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, topic || '\n' || summary FROM scenarios WHERE embedding IS NULL LIMIT 500",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    let total = atoms.len() + scenarios.len();
    if total == 0 {
        ctx.emit("重嵌：无缺失向量（空跑）", None).await.ok();
        return Ok(serde_json::json!({"atoms": 0, "scenarios": 0, "total": 0}));
    }
    ctx.emit(
        &format!(
            "重嵌：{total} 条缺失（原子 {} / 场景 {}）",
            atoms.len(),
            scenarios.len()
        ),
        None,
    )
    .await
    .ok();

    let mut done_atoms = 0usize;
    let mut done_scenarios = 0usize;

    for chunk in atoms.chunks(BATCH) {
        let texts: Vec<String> = chunk.iter().map(|(_, c)| c.clone()).collect();
        let embs = llm.embed(&texts, ctx.job.id).await?;
        for ((id, _), vec) in chunk.iter().zip(embs.iter()) {
            if vec.iter().all(|&x| x == 0.0) {
                continue; // B2：全零向量拒绝入库
            }
            sqlx::query("UPDATE atoms SET embedding = $2 WHERE id = $1 AND embedding IS NULL")
                .bind(id)
                .bind(pgvector::Vector::from(vec.clone()))
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
            done_atoms += 1;
        }
    }

    for chunk in scenarios.chunks(BATCH) {
        let texts: Vec<String> = chunk.iter().map(|(_, c)| c.clone()).collect();
        let embs = llm.embed(&texts, ctx.job.id).await?;
        for ((id, _), vec) in chunk.iter().zip(embs.iter()) {
            if vec.iter().all(|&x| x == 0.0) {
                continue;
            }
            sqlx::query("UPDATE scenarios SET embedding = $2 WHERE id = $1 AND embedding IS NULL")
                .bind(id)
                .bind(pgvector::Vector::from(vec.clone()))
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
            done_scenarios += 1;
        }
    }

    ctx.emit(
        &format!("重嵌完成：原子 {done_atoms} / 场景 {done_scenarios}"),
        None,
    )
    .await
    .ok();
    Ok(serde_json::json!({
        "atoms": done_atoms,
        "scenarios": done_scenarios,
        "total": total
    }))
}

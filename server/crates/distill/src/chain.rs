//! 蒸馏链装配：handler 注册 + 防抖触发。

use agent_memory_jobs::types::{JobError, JobTemplate};
use agent_memory_jobs::{JobQueue, Runner};
use agent_memory_llm::KeyCipher;
use sqlx::PgPool;
use std::sync::Arc;

use crate::llm_port::{GatewayLlm, LlmRef};

/// 注册全部蒸馏 handler（main 装配用）。
pub fn register_handlers(runner: Runner, llm: LlmRef) -> Runner {
    let l_extract = llm.clone();
    let l_arbitrate = llm.clone();
    let l_organize = llm.clone();
    let l_persona = llm.clone();
    let l_consolidate = llm.clone();
    let l_reembed = llm.clone();
    runner
        .register("extract_atoms", move |ctx| {
            let llm = l_extract.clone();
            async move { crate::extract::run(ctx, llm).await }
        })
        .register("arbitrate_atoms", move |ctx| {
            let llm = l_arbitrate.clone();
            async move { crate::arbitrate::run(ctx, llm).await }
        })
        .register("organize_scenarios", move |ctx| {
            let llm = l_organize.clone();
            async move { crate::organize::run(ctx, llm).await }
        })
        .register("distill_persona", move |ctx| {
            let llm = l_persona.clone();
            async move { crate::persona::run(ctx, llm).await }
        })
        .register("consolidate", move |ctx| {
            let llm = l_consolidate.clone();
            async move { crate::consolidate::run(ctx, llm).await }
        })
        .register("reembed_memory", move |ctx| {
            let llm = l_reembed.clone();
            async move { crate::reembed::run(ctx, llm).await }
        })
}

/// 构建真实网关 LLM 端口。
pub fn gateway_llm(pool: PgPool, cipher: KeyCipher) -> LlmRef {
    Arc::new(GatewayLlm::new(pool, cipher))
}

/// L0 写入后的防抖触发：同一 30s 窗口内的写入共用一个 extract 任务。
/// 窗口键 = unix_epoch / 30 —— 状态less，任务到期时自取全部 pending 会话。
pub async fn trigger_auto_extract(
    queue: &JobQueue,
    window_secs: i64,
) -> Result<agent_memory_jobs::Job, JobError> {
    let now = chrono::Utc::now();
    let bucket = now.timestamp() / window_secs;
    queue
        .enqueue(
            JobTemplate::new("extract_atoms")
                .with_idempotency_key(format!("extract-debounce-{bucket}"))
                .with_payload(serde_json::json!({"reason": "auto"}))
                .with_due(now + chrono::Duration::seconds(window_secs)),
        )
        .await
}

/// 手动触发（立即执行；scope=full 时附带 consolidate）。
pub async fn trigger_manual(
    queue: &JobQueue,
    with_consolidate: bool,
) -> Result<Vec<agent_memory_jobs::Job>, JobError> {
    let mut out = vec![
        queue
            .enqueue(
                JobTemplate::new("extract_atoms")
                    .with_payload(serde_json::json!({"reason": "manual"})),
            )
            .await?,
    ];
    if with_consolidate {
        out.push(queue.enqueue(JobTemplate::new("consolidate")).await?);
    }
    Ok(out)
}

//! maintain_wiki：wiki 定期巡逻（对齐 memory 域 maintain_memory 模式，P015 后）。
//!
//! 编排既有确定性组件，不引入新的 LLM 循环：
//! ① lint 全量体检（死链/孤儿/坏 frontmatter/重复实体/过时源）
//! ② repair 确定性修复自动执行（三级边界现成：自动做/留痕做/不做）+ 向量回填
//! ③ duplicate_candidates 重复页候选（只报告，不自动合并——语义级判断留 lint_deep + 人）
//! ④ lint_deep LLM 深度检查入队（矛盾/过时/缺页的语义级判断）
//! 巡检报告聚合落 jobs progress；节律每日桶 + 手动触发（单飞守卫见 count_running_maintain_wiki）。

use crate::service::WikiService;
use engram_jobs::types::JobError;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// 巡检报告（jobs progress 载荷；前端巡检报告视图直接消费）。
#[derive(Debug, serde::Serialize)]
pub struct WikiPatrolReport {
    pub lib: Uuid,
    pub lint_issues: usize,
    pub lint_checked_pages: usize,
    /// lint 问题摘要（kind + 计数）
    pub lint_summary: Vec<(String, usize)>,
    pub repair_actions: usize,
    pub repair_checked_pages: usize,
    /// 修复动作明细（kind + slug）
    pub repair_detail: Vec<(String, String)>,
    pub embedding_backfilled: i64,
    pub duplicate_candidates: usize,
    /// LLM 深检任务 id（异步跑，结果看该 job 的 progress）
    pub lint_deep_job: Option<Uuid>,
}

/// 巡逻一次：lint → repair（自动）→ duplicates → lint_deep 入队 → 报告。
pub async fn run_patrol(
    pool: &PgPool,
    wiki: &WikiService,
    lib: Uuid,
) -> Result<WikiPatrolReport, JobError> {
    // ① lint 全量体检
    let lint = wiki.lint(lib).await.map_err(job_err)?;
    let mut lint_summary: std::collections::BTreeMap<String, usize> = Default::default();
    for i in &lint.issues {
        *lint_summary.entry(i.rule.clone()).or_default() += 1;
    }

    // ② repair 确定性修复自动执行（三级边界现成）+ 向量回填
    let repair = wiki.repair(lib).await.map_err(job_err)?;
    let backfilled = wiki.backfill_embeddings(lib).await.unwrap_or(0);

    // ③ 重复页候选（只报告）
    let dups = wiki.duplicate_candidates(lib).await.map_err(job_err)?;

    // ④ LLM 深检入队（异步；有 issue 才值得深检）
    let lint_deep_job = if lint.issues.is_empty() {
        None
    } else {
        Some(
            crate::lint_deep::enqueue(pool, lib, None)
                .await
                .map_err(job_err)?,
        )
    };

    let report = WikiPatrolReport {
        lib,
        lint_issues: lint.issues.len(),
        lint_checked_pages: lint.checked_pages,
        lint_summary: lint_summary.into_iter().collect(),
        repair_actions: repair.actions.len(),
        repair_checked_pages: repair.checked_pages,
        repair_detail: repair
            .actions
            .iter()
            .map(|a| (a.action.clone(), a.slug.clone()))
            .collect(),
        embedding_backfilled: backfilled as i64,
        duplicate_candidates: dups.len(),
        lint_deep_job,
    };
    Ok(report)
}

/// 单飞守卫：是否存在 running 的 maintain_wiki。
pub async fn count_running_maintain_wiki(pool: &PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE kind = 'maintain_wiki' AND status = 'running'",
    )
    .fetch_one(pool)
    .await
}

/// job handler 主体（register_handlers 薄壳调用）。
pub async fn patrol_job(
    ctx: &engram_jobs::JobContext,
    wiki: &WikiService,
) -> Result<Value, JobError> {
    let pool = ctx.pool().clone();
    let lib = crate::libraries::resolve(&pool, None)
        .await
        .map_err(job_err)?;
    let report = run_patrol(&pool, wiki, lib).await?;
    ctx.emit(
        &format!(
            "巡逻完成：lint {} 项 / 修复 {} 动作 / 重复候选 {} / 深检 {}",
            report.lint_issues,
            report.repair_actions,
            report.duplicate_candidates,
            if report.lint_deep_job.is_some() {
                "已入队"
            } else {
                "跳过"
            }
        ),
        Some(serde_json::to_value(&report).unwrap_or_default()),
    )
    .await
    .ok();
    serde_json::to_value(&report).map_err(|e| JobError::Permanent(e.to_string()))
}

fn job_err(e: impl std::fmt::Display) -> JobError {
    JobError::Permanent(e.to_string())
}

// ---------- 节律：每日巡逻桶（自续期，不依赖 distill rhythm） ----------

/// 节律壳任务 kind（不入 WORKFLOW_KINDS 的守卫语义由 maintain_wiki 承担）。
pub const KIND_MAINTAIN_WIKI_RHYTHM: &str = "rhythm_maintain_wiki";

/// 今日桶幂等键（本地日期）。
fn patrol_bucket_key(day: chrono::NaiveDate) -> String {
    format!("rhythm-maintain-wiki-{}", day.format("%Y%m%d"))
}

/// 注册节律壳 handler：续期明天桶 + 投递 maintain_wiki 真任务。
pub fn register_patrol_rhythm(runner: engram_jobs::Runner) -> engram_jobs::Runner {
    runner.register(KIND_MAINTAIN_WIKI_RHYTHM, move |ctx| async move {
        // 先续期明天（cron 语义：本轮失败不影响下期）
        let tomorrow = chrono::Local::now()
            .date_naive()
            .succ_opt()
            .unwrap_or_else(|| chrono::Local::now().date_naive());
        ctx.enqueue_next(
            engram_jobs::JobTemplate::new(KIND_MAINTAIN_WIKI_RHYTHM)
                .with_idempotency_key(patrol_bucket_key(tomorrow))
                .with_payload(serde_json::json!({"reason": "rhythm"}))
                .with_due(chrono::Utc::now() + chrono::Duration::hours(24)),
        )
        .await?;
        // 投真任务（单飞守卫在 WORKFLOW_KINDS）
        ctx.enqueue_next(
            engram_jobs::JobTemplate::new("maintain_wiki")
                .with_payload(serde_json::json!({"reason": "rhythm"})),
        )
        .await?;
        Ok(serde_json::json!({"enqueued": "maintain_wiki"}))
    })
}

/// 启动自检补建：今天的桶不在队则建（幂等键命中即复用，重复启动安全）。
pub async fn bootstrap_patrol(queue: &engram_jobs::JobQueue) -> Result<(), String> {
    let today = chrono::Local::now().date_naive();
    queue
        .enqueue(
            engram_jobs::JobTemplate::new(KIND_MAINTAIN_WIKI_RHYTHM)
                .with_idempotency_key(patrol_bucket_key(today))
                .with_payload(serde_json::json!({"reason": "bootstrap"})),
        )
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

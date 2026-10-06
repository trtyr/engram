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
    /// 维护 Agent 纪要（语义裁决一句话总结）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_summary: Option<String>,
    /// 建议人工处理的动作（Agent 裁决产出）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual_actions: Option<Vec<String>>,
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
        agent_summary: None,
        manual_actions: None,
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
    llm: &crate::service::LlmRef,
) -> Result<Value, JobError> {
    let pool = ctx.pool().clone();
    let lib = crate::libraries::resolve(&pool, None)
        .await
        .map_err(job_err)?;
    let mut report = run_patrol(&pool, wiki, lib).await?;

    // ⑤ 维护 Agent：语义裁决 + 纪要（工具循环只读，建议不自动执行）
    let evidence = serde_json::json!({
        "lint_issues": report.lint_issues,
        "lint_summary": report.lint_summary,
        "duplicate_candidates": wiki.duplicate_candidates(lib).await.unwrap_or_default(),
        "repair_actions_done": report.repair_detail,
    });
    match crate::patrol_agent::run(ctx, llm, wiki, lib, evidence).await {
        Ok(brief) => {
            report.agent_summary = Some(brief.summary);
            report.manual_actions = if brief.manual_actions.is_empty() {
                None
            } else {
                Some(brief.manual_actions)
            };
        }
        Err(e) => tracing::warn!("维护 Agent 循环失败（不阻塞巡逻）: {e}"),
    }

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
    let mut progress =
        serde_json::to_value(&report).map_err(|e| JobError::Permanent(e.to_string()))?;
    // Markdown 巡逻报告（历史存档，前端详情栏直接渲染）
    if let Some(obj) = progress.as_object_mut() {
        obj.insert("markdown".into(), Value::String(render_markdown(&report)));
    }
    Ok(progress)
}

/// 巡逻报告 → Markdown 存档（人类可读，详情栏直接渲染）。
fn render_markdown(r: &WikiPatrolReport) -> String {
    let mut md = String::from("# Wiki 巡逻报告\n\n");
    md.push_str(&format!(
        "- 体检 {} 页，lint **{}** 项问题；修复扫描 {} 页，自动执行 **{}** 项动作；向量回填 {} 页\n",
        r.lint_checked_pages, r.lint_issues, r.repair_checked_pages, r.repair_actions, r.embedding_backfilled,
    ));
    if !r.lint_summary.is_empty() {
        md.push_str("\n## Lint 体检\n\n| 类型 | 数量 |\n|---|---|\n");
        for (kind, n) in &r.lint_summary {
            md.push_str(&format!("| {kind} | {n} |\n"));
        }
    } else {
        md.push_str("\n## Lint 体检\n\n全部干净。\n");
    }
    if !r.repair_detail.is_empty() {
        md.push_str("\n## 自动修复\n\n");
        for (action, slug) in &r.repair_detail {
            md.push_str(&format!("- `{action}` · {slug}\n"));
        }
    }
    if r.duplicate_candidates > 0 {
        md.push_str(&format!(
            "\n## 重复页候选\n\n{} 组——只报告不自动合并，确认后手动处理。\n",
            r.duplicate_candidates
        ));
    }
    if let Some(s) = &r.agent_summary {
        md.push_str(&format!("\n## 维护官纪要\n\n{s}\n"));
    }
    if let Some(actions) = r.manual_actions.as_ref().filter(|a| !a.is_empty()) {
        md.push_str("\n## 待人工处理\n\n");
        for a in actions {
            md.push_str(&format!("- [ ] {a}\n"));
        }
    }
    if let Some(job) = r.lint_deep_job {
        md.push_str(&format!(
            "\n---\n\nLLM 深度检查已入队（`{}`）——语义发现见该任务的进度。\n",
            &job.to_string()[..8]
        ));
    }
    md
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

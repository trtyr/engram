//! 项目文档整理 Agent（P017）：定期巡逻单个项目的文档——捋结构、发现并留痕问题。
//!
//! 模式对齐：maintain_agent（JSON 协议工具循环）+ patrol_agent（只读裁决+纪要）。
//! 并发模型：**不进 WORKFLOW_KINDS**（全局串行信号量会扼杀多项目并行）——
//! 并发靠 runner worker 池，互斥靠 per-project 单飞守卫（同项目不重入）。

use engram_jobs::types::JobError;
use engram_llm::types::Purpose;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::llm_port::{LlmRef, chat_json_retrying};

pub const KIND_MAINTAIN_PROJECT: &str = "maintain_project";
pub const KIND_MAINTAIN_PROJECT_RHYTHM: &str = "rhythm_maintain_project";

/// 步数闸门（预算闸门：超限优雅收尾，不报错）。
const MAX_STEPS: usize = 12;
/// 单文档送审正文截断（字符）。
const DOC_SNIPPET: usize = 2400;

// ---------- Agent ----------

/// 整理纪要。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ProjectBrief {
    pub summary: String,
    pub issues_noted: usize,
    pub steps: usize,
}

#[derive(Debug, Deserialize)]
struct AgentTurn {
    tool: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Deserialize)]
struct FinishArgs {
    summary: String,
    #[serde(default)]
    issues_noted: usize,
}

fn system_prompt() -> String {
    "你是项目文档整理官。你负责梳理一个项目的文档集合——检查结构是否清晰、内容是否过时、\
     分类是否放对、文档之间是否矛盾。发现问题就用 note_issue 工具把维护注记写进对应文档\
     （带你的研判与建议），供人后续处理。你只做整理与留痕，不大改文档正文。\
     可用工具（每轮只调一个，输出 JSON）：\n\
     {\"tool\":\"list_docs\",\"args\":{}} — 项目文档树（id/分类/标题/字数）；\n\
     {\"tool\":\"get_doc\",\"args\":{\"doc_id\":\"...\"}} — 读文档全文（截 2400 字）；\n\
     {\"tool\":\"note_issue\",\"args\":{\"doc_id\":\"...\",\"issue\":\"...\",\"suggestion\":\"...\"}} \
     — 把问题作为维护注记写进该文档（留痕，幂等安全）；\n\
     {\"tool\":\"finish\",\"args\":{\"summary\":\"...\",\"issues_noted\":0}} — 收尾交纪要。\
     没问题就直接 finish——不要为了用工具而用工具，也不要编造问题。"
        .to_string()
}

/// 整理 Agent 巡逻循环：读文档树 → 逐份研判 → note_issue 留痕 → finish 交纪要。
pub async fn run(
    ctx: &engram_jobs::JobContext,
    llm: &LlmRef,
    pool: &PgPool,
    project_id: Uuid,
) -> Result<ProjectBrief, JobError> {
    let system = system_prompt();
    let mut history: Vec<Value> = Vec::new();
    let mut brief = ProjectBrief::default();

    for step in 0..MAX_STEPS {
        brief.steps = step + 1;
        let mut user = json!({ "project_id": project_id, "history": history });
        if history.is_empty() {
            user["hint"] = json!("从 list_docs 开始总览。");
        }
        let user_text =
            serde_json::to_string(&user).map_err(|e| JobError::Retryable(e.to_string()))?;
        let out = chat_json_retrying(
            ctx,
            llm.as_ref(),
            Purpose::Consolidate,
            &system,
            &user_text,
            ctx.job.id,
        )
        .await?;
        let turn: AgentTurn = serde_json::from_value(out)
            .map_err(|e| JobError::Retryable(format!("项目整理 Agent 输出结构不符: {e}")))?;

        let tool = turn.tool.as_str();
        let args = &turn.args;
        let result: Value = match tool {
            "list_docs" => {
                let rows: Vec<(Uuid, String, Option<String>, String, i64)> = sqlx::query_as(
                    "SELECT id, category, folder, title, length(content)::bigint \
                         FROM project_docs WHERE project_id = $1 ORDER BY category, title",
                )
                .bind(project_id)
                .fetch_all(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
                json!({
                    "docs": rows.iter().map(|(id, cat, folder, title, chars)| json!({
                        "doc_id": id, "category": cat, "folder": folder,
                        "title": title, "chars": chars,
                    })).collect::<Vec<_>>(),
                })
            }
            "get_doc" => {
                let doc_id = args
                    .get("doc_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok());
                match doc_id {
                    Some(id) => {
                        let row: Option<(String, String, String)> = sqlx::query_as(
                            "SELECT title, content, category FROM project_docs \
                             WHERE id = $1 AND project_id = $2",
                        )
                        .bind(id)
                        .bind(project_id)
                        .fetch_optional(pool)
                        .await
                        .map_err(|e| JobError::Retryable(e.to_string()))?;
                        match row {
                            Some((title, content, category)) => {
                                let mut body: String = content.chars().take(DOC_SNIPPET).collect();
                                if content.chars().count() > DOC_SNIPPET {
                                    body.push('…');
                                }
                                json!({ "doc_id": id, "title": title, "category": category, "content": body })
                            }
                            None => json!({ "error": "文档不存在（或非本项目）" }),
                        }
                    }
                    None => json!({ "error": "doc_id 非法 UUID" }),
                }
            }
            "note_issue" => {
                let doc_id = args
                    .get("doc_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok());
                let issue = args
                    .get("issue")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let suggestion = args
                    .get("suggestion")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                match (doc_id, issue.is_empty()) {
                    (Some(id), false) => {
                        match note_issue(pool, project_id, id, &issue, &suggestion).await {
                            Ok(()) => {
                                brief.issues_noted += 1;
                                json!({ "ok": true, "doc_id": id, "hint": "已留痕——人审时可见" })
                            }
                            Err(e) => json!({ "error": e }),
                        }
                    }
                    _ => json!({ "error": "需要合法 doc_id 与非空 issue" }),
                }
            }
            "finish" => {
                match serde_json::from_value::<FinishArgs>(turn.args.clone()) {
                    Ok(f) => {
                        brief.summary = f.summary;
                        if f.issues_noted > brief.issues_noted {
                            brief.issues_noted = f.issues_noted;
                        }
                    }
                    Err(e) => {
                        history.push(json!({
                            "tool": "finish", "args": turn.args,
                            "result": {"error": format!("结构不符: {e}——请重发 finish")}
                        }));
                        continue;
                    }
                }
                break;
            }
            other => {
                json!({ "error": format!("未知工具 {other}——可用 list_docs/get_doc/note_issue/finish") })
            }
        };
        history.push(json!({ "tool": turn.tool, "args": turn.args, "result": result }));
    }

    if brief.summary.is_empty() {
        brief.summary = format!("项目整理 Agent 达到步数上限（{MAX_STEPS}），按已执行留痕收尾");
    }
    Ok(brief)
}

/// 维护注记留痕：追加「## 维护注记」段（已有则追加条目）——幂等安全，人审可见。
async fn note_issue(
    pool: &PgPool,
    project_id: Uuid,
    doc_id: Uuid,
    issue: &str,
    suggestion: &str,
) -> Result<(), String> {
    let today = chrono::Local::now().format("%Y-%m-%d");
    let mut line = format!("- **{today}** {issue}");
    if !suggestion.is_empty() {
        line.push_str(&format!(" → 建议：{suggestion}"));
    }
    line.push('\n');
    let r = sqlx::query(
        "UPDATE project_docs SET content = \
         CASE WHEN content LIKE '%## 维护注记%' \
              THEN content || $3 \
              ELSE content || E'\\n\\n## 维护注记\\n\\n> 由项目整理 Agent 巡逻留痕\\n\\n' || $3 \
         END \
         WHERE id = $1 AND project_id = $2 RETURNING id",
    )
    .bind(doc_id)
    .bind(project_id)
    .bind(&line)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    if r.is_none() {
        return Err("文档不存在（或非本项目）".into());
    }
    Ok(())
}

// ---------- 编排 + 节律 ----------

/// 单飞守卫：该项目是否已有 running 的整理任务。
pub async fn count_running_for_project(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE kind = 'maintain_project' AND status = 'running' \
         AND payload->>'project_id' = $1",
    )
    .bind(project_id.to_string())
    .fetch_one(pool)
    .await
}

/// job handler 主体：跑 Agent → Markdown 报告落 progress。
pub async fn maintain_project_job(
    ctx: &engram_jobs::JobContext,
    llm: &LlmRef,
) -> Result<Value, JobError> {
    let pool = ctx.pool();
    let payload = &ctx.job.payload.0;
    let project_id: Uuid = payload
        .get("project_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| JobError::Permanent("payload 缺 project_id".into()))?;
    let project_name: String = sqlx::query_scalar("SELECT name FROM projects WHERE id = $1")
        .bind(project_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?
        .ok_or_else(|| JobError::Permanent("项目不存在".into()))?;

    let agent = run(ctx, llm, pool, project_id).await;
    let brief = match agent {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("项目整理 Agent 失败（project={project_name}）: {e}");
            return Err(e);
        }
    };

    let report = json!({
        "project_id": project_id,
        "project_name": project_name,
        "summary": brief.summary,
        "issues_noted": brief.issues_noted,
        "steps": brief.steps,
        "markdown": render_markdown(&project_name, &brief),
    });
    ctx.emit(
        &format!(
            "项目整理完成：{} —— {}（留痕 {} 条）",
            project_name, brief.summary, brief.issues_noted
        ),
        Some(report.clone()),
    )
    .await
    .ok();
    Ok(report)
}

/// 整理报告 → Markdown 存档。
fn render_markdown(project_name: &str, brief: &ProjectBrief) -> String {
    let mut md = format!("# 项目整理报告 · {project_name}\n\n");
    md.push_str(&format!(
        "- 巡逻 {} 步，留痕 **{}** 条维护注记\n\n## 纪要\n\n{}\n",
        brief.steps, brief.issues_noted, brief.summary
    ));
    md.push_str(
        "\n> 维护注记已写进对应文档（「## 维护注记」段）——人审时逐条处理，处理后可删除该条。\n",
    );
    md
}

fn project_bucket_key(day: chrono::NaiveDate) -> String {
    format!("rhythm-maintain-project-{}", day.format("%Y%m%d"))
}

/// 注册 handler：maintain_project（整理）+ rhythm_maintain_project（节律壳）。
/// 注意：两个 kind 都不进 WORKFLOW_KINDS——多项目并发靠 worker 池。
pub fn register_maintain_project(runner: engram_jobs::Runner, llm: LlmRef) -> engram_jobs::Runner {
    let llm_rhythm = llm.clone();
    runner
        .register(KIND_MAINTAIN_PROJECT, move |ctx| {
            let llm = llm.clone();
            async move { maintain_project_job(&ctx, &llm).await }
        })
        .register(KIND_MAINTAIN_PROJECT_RHYTHM, move |ctx| {
            let llm = llm_rhythm.clone();
            async move {
                let _ = llm; // 节律壳不直接用 LLM——投递子任务由 worker 消费时用
                let pool = ctx.pool();
                // 先自续明日桶（cron 语义：本轮失败不影响下期）
                enqueue_tomorrow_rhythm(&engram_jobs::queue::JobQueue::new(pool.clone()))
                    .await
                    .map_err(JobError::Retryable)?;
                // 扫全部项目逐个投递（单飞守卫在入队前过滤 running）
                let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM projects ORDER BY name")
                    .fetch_all(pool)
                    .await
                    .map_err(|e| JobError::Retryable(e.to_string()))?
                    .into_iter()
                    .collect();
                let mut enqueued = 0usize;
                let mut skipped = 0usize;
                for pid in ids {
                    match count_running_for_project(pool, pid).await {
                        Ok(0) => {}
                        Ok(_) => {
                            skipped += 1;
                            continue;
                        }
                        Err(e) => return Err(JobError::Retryable(e.to_string())),
                    }
                    crate::project_maintain::enqueue_maintain_project(pool, pid)
                        .await
                        .map_err(|e| JobError::Retryable(e.to_string()))?;
                    enqueued += 1;
                }
                Ok(json!({ "enqueued": enqueued, "skipped_running": skipped }))
            }
        })
}

/// 入队单项目整理任务。
pub async fn enqueue_maintain_project(pool: &PgPool, project_id: Uuid) -> Result<Uuid, String> {
    engram_jobs::queue::JobQueue::new(pool.clone())
        .enqueue(
            engram_jobs::types::JobTemplate::new(KIND_MAINTAIN_PROJECT)
                .with_payload(json!({ "project_id": project_id })),
        )
        .await
        .map(|j| j.id)
        .map_err(|e| e.to_string())
}

/// 启动补建：今天的节律桶不在队则建（幂等键命中即复用）。
pub async fn bootstrap_maintain_project(
    queue: &engram_jobs::queue::JobQueue,
) -> Result<(), String> {
    let today = chrono::Local::now().date_naive();
    queue
        .enqueue(
            engram_jobs::types::JobTemplate::new(KIND_MAINTAIN_PROJECT_RHYTHM)
                .with_idempotency_key(project_bucket_key(today))
                .with_payload(json!({ "reason": "bootstrap" })),
        )
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// 节律自续明日桶。
pub async fn enqueue_tomorrow_rhythm(queue: &engram_jobs::queue::JobQueue) -> Result<(), String> {
    let tomorrow = chrono::Local::now()
        .date_naive()
        .succ_opt()
        .ok_or("日期溢出")?;
    queue
        .enqueue(
            engram_jobs::types::JobTemplate::new(KIND_MAINTAIN_PROJECT_RHYTHM)
                .with_idempotency_key(project_bucket_key(tomorrow))
                .with_payload(json!({ "reason": "rhythm" }))
                .with_due(chrono::Utc::now() + chrono::Duration::hours(24)),
        )
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

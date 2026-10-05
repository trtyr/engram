//! wiki 维护 Agent（P016 objective ③）：复用 JSON 协议工具循环（maintain_agent 同款模式），
//! 在确定性巡逻（lint→repair→duplicates）之上做语义裁决与维护纪要。
//!
//! 职责边界：确定性阶段已自动执行结构修复；Agent 只做「语义裁决 + 纪要」——
//! 工具面只读（get_page / list_pages）+ finish，重复页合并等语义动作默认只报告不自动执行。

use engram_jobs::types::JobError;
use engram_llm::types::Purpose;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use engram_distill::llm_port::chat_json_retrying;

/// 步数闸门（预算闸门：超出按已执行裁决收尾，不报错）。
const MAX_STEPS: usize = 12;
/// 单页正文截断（字符）——裁决只需要声明级语义。
const PAGE_SNIPPET: usize = 1600;

/// 维护纪要：Agent 的语义裁决产出。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PatrolBrief {
    /// 一句话纪要（本轮巡逻语义结论）
    pub summary: String,
    /// 建议人工处理的动作清单（合并建议/补写建议等）
    pub manual_actions: Vec<String>,
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
    manual_actions: Vec<String>,
}

fn system_prompt() -> String {
    "你是 Wiki 维护官。你会收到一轮自动巡逻的证据（结构 lint 剩余问题、修复摘要、重复页候选）。\
     你的职责：①对证据做语义裁决——需要时翻看页面原文确认问题是否真实；\
     ②产出维护纪要：一句话总结本轮健康状态 + 建议人工处理的动作清单（重复页合并方向、\
     内容补写方向等）。你只能报告建议，不能修改任何页面。\
     可用工具（每轮只调一个，输出 JSON）：\n\
     {\"tool\":\"list_pages\",\"args\":{}} — 列出全部页面（slug+标题）；\n\
     {\"tool\":\"get_page\",\"args\":{\"slug\":\"...\"}} — 读页面原文（截 1600 字）；\n\
     {\"tool\":\"finish\",\"args\":{\"summary\":\"...\",\"manual_actions\":[\"...\"]}} — 收尾交纪要。\
     没有值得裁决的问题就直接 finish，不要为了用工具而用工具。"
        .to_string()
}

/// 维护 Agent 巡逻循环：读确定性巡逻证据 → 按需翻页裁决 → finish 产出维护纪要。
pub async fn run(
    ctx: &engram_jobs::JobContext,
    llm: &crate::service::LlmRef,
    wiki: &crate::service::WikiService,
    lib: Uuid,
    evidence: Value,
) -> Result<PatrolBrief, JobError> {
    let system = system_prompt();
    let mut history: Vec<Value> = Vec::new();
    let mut brief = PatrolBrief::default();

    for step in 0..MAX_STEPS {
        brief.steps = step + 1;
        let mut user = format!(
            "本轮巡逻证据（确定性阶段产出）：\n{}\n\n工具调用历史：{}",
            serde_json::to_string(&evidence).unwrap_or_default(),
            serde_json::to_string(&history).unwrap_or_default(),
        );
        user.push_str("\n\n请输出下一个工具调用 JSON。");
        let out = chat_json_retrying(
            ctx,
            llm.as_ref(),
            Purpose::WikiLint,
            &system,
            &user,
            ctx.job.id,
        )
        .await?;
        let turn: AgentTurn = serde_json::from_value(out)
            .map_err(|e| JobError::Retryable(format!("维护 Agent 输出结构不符: {e}")))?;

        let result: Value = match turn.tool.as_str() {
            "list_pages" => match wiki.list_pages(lib, None, None, Some(200), None).await {
                Ok(pages) => json!({
                    "pages": pages.iter().map(|p| json!({"slug": p.slug, "title": p.title})).collect::<Vec<_>>()
                }),
                Err(e) => json!({"error": e.to_string()}),
            },
            "get_page" => {
                let slug = turn.args.get("slug").and_then(|v| v.as_str()).unwrap_or("");
                match wiki.get_page(lib, slug).await {
                    Ok(p) => {
                        let mut body: String = p.content.chars().take(PAGE_SNIPPET).collect();
                        if p.content.chars().count() > PAGE_SNIPPET {
                            body.push('…');
                        }
                        json!({"slug": p.slug, "title": p.title, "content": body})
                    }
                    Err(e) => json!({"error": e.to_string()}),
                }
            }
            "finish" => {
                match serde_json::from_value::<FinishArgs>(turn.args.clone()) {
                    Ok(f) => {
                        brief.summary = f.summary;
                        brief.manual_actions = f.manual_actions;
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
            other => json!({"error": format!("未知工具 {other}——可用 list_pages/get_page/finish")}),
        };
        history.push(json!({ "tool": turn.tool, "args": turn.args, "result": result }));
    }

    if brief.summary.is_empty() {
        brief.summary = format!("维护 Agent 达到步数上限（{MAX_STEPS}），按已执行裁决收尾");
    }
    Ok(brief)
}

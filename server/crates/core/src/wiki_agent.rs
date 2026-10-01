//! Wiki 维护 Agent Harness（P004-T010）——engram 侧的 LLM 工具循环。
//!
//! 拍板（2026-10-02）：①Purpose::WikiAgent 独立档位；②破坏性工具全审计；
//! ③document_add ready 后自动接力 harness；④jobs 体系 per-kind 串行队列。
//! 工具面与外部 Agent 的 MCP wiki 工具能力等价（同一 service 层）；
//! 跨任务状态不存循环记忆，存 wiki 本身（Karpathy 模式）。

use std::time::{Duration, Instant};
use std::sync::Arc;

use serde_json::{json, Value};
use uuid::Uuid;

use engram_jobs::{JobContext, JobError, JobTemplate};
use engram_llm::types::{ChatMessage, ChatRequest, ToolDef};
use engram_llm::provider::LlmProvider;
use engram_llm::types::{LlmError, Purpose};
use engram_storage::PgPool;
use engram_wiki_engine::service::WikiService;

use crate::wiki_docs::pipeline::IngestSource;
use crate::wiki_docs::WikiDocumentService;

/// 预算（首版默认：20 轮 / 60 次 LLM 调用 / 总超时 10 分钟）。
#[derive(Debug, Clone)]
pub struct AgentBudget {
    pub max_rounds: usize,
    pub max_llm_calls: usize,
    pub timeout: Duration,
}

impl Default for AgentBudget {
    fn default() -> Self {
        Self {
            max_rounds: 20,
            max_llm_calls: 60,
            timeout: Duration::from_secs(600),
        }
    }
}

/// 一次 harness 任务的输入。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentTask {
    pub lib: Uuid,
    pub instruction: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
}

/// harness 运行报告（回执/审计面）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentReport {
    pub rounds: usize,
    pub llm_calls: usize,
    pub tool_calls: usize,
    pub pages_touched: Vec<String>,
    pub degraded: bool,
    pub degraded_reason: Option<String>,
    pub summary: String,
}

/// harness 错误（工具失败不属此类——工具错误回填给 LLM 自纠）。
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("LLM: {0}")]
    Llm(#[from] LlmError),
    #[error("预算耗尽：{0}")]
    Budget(String),
    #[error("存储: {0}")]
    Storage(String),
}

impl AgentError {
    /// 是否值得重试（仅 LLM 瞬态）。
    pub fn retryable(&self) -> bool {
        matches!(self, AgentError::Llm(LlmError::Transient(_)))
    }
}

/// 依赖装配（job handler 与测试共用）。
pub struct AgentDeps {
    pub pool: PgPool,
    pub registry: engram_llm::ProviderRegistry,
    pub data_dir: std::path::PathBuf,
}

impl AgentDeps {
    pub fn wiki(&self) -> WikiService {
        WikiService::new(self.pool.clone(), self.registry.clone())
    }
    pub fn docs(&self) -> WikiDocumentService {
        WikiDocumentService::new(
            self.pool.clone(),
            self.registry.clone(),
            self.data_dir.clone(),
        )
    }
}

/// 工具定义（与 MCP wiki 域能力等价的子集；name 与参数 schema 在此单点维护）。
fn tool_defs() -> Vec<ToolDef> {
    fn def(name: &str, desc: &str, props: Value, required: &[&str]) -> ToolDef {
        ToolDef {
            name: name.into(),
            description: desc.into(),
            parameters: json!({
                "type": "object",
                "properties": props,
                "required": required,
            }),
        }
    }
    vec![
        def("web_reader", "抓取网页正文（markdown，含标题/链接）。SPA/JS 渲染页可用。",
            json!({"url": {"type": "string", "description": "完整 URL"}}), &["url"]),
        def("wiki_search", "按关键词检索 wiki 页面（FTS+向量混合），返回 slug/标题/摘要。",
            json!({"query": {"type": "string"}, "limit": {"type": "integer", "description": "默认 8"}}),
            &["query"]),
        def("get_page", "读一个 wiki 页的完整内容。",
            json!({"slug": {"type": "string"}}), &["slug"]),
        def("write_page", "创建或覆盖一个 wiki 页（markdown，互链用 [[slug]]）。覆盖前可先 get_page 保留有价值内容。",
            json!({
                "slug": {"type": "string", "description": "字母/数字/-/_/·，≤80 字符"},
                "title": {"type": "string"},
                "content": {"type": "string"},
                "folder": {"type": "string", "description": "可选目录"}
            }),
            &["slug", "title", "content"]),
        def("wiki_graph", "全库页面互链图（节点=页面+边=链接），用于了解现有结构、避免重复建页。",
            json!({}), &[]),
        def("wiki_lint", "质量体检报告（孤儿页/缺互链/矛盾提示）。",
            json!({}), &[]),
        def("document_add", "把原文资料（URL 或文本）入库为文档（后台分块嵌入，供 documents_search 检索）。",
            json!({
                "name": {"type": "string"},
                "url": {"type": "string", "description": "与 text 二选一"},
                "text": {"type": "string", "description": "与 url 二选一"}
            }),
            &["name"]),
        def("documents_search", "检索已入库文档的原文分块（FTS+向量），带文档出处。",
            json!({"query": {"type": "string"}, "limit": {"type": "integer", "description": "默认 6"}}),
            &["query"]),
        def("delete_page", "【破坏性】删除一个 wiki 页。仅在确凿重复/错误时使用。",
            json!({"slug": {"type": "string"}}), &["slug"]),
        def("merge_pages", "【破坏性】把 duplicate 页并入 primary 页（内容拼接+重定向），删除 duplicate。",
            json!({"primary": {"type": "string"}, "duplicate": {"type": "string"}}),
            &["primary", "duplicate"]),
    ]
}

/// 工具结果截断（防上下文爆炸）。
const TOOL_RESULT_MAX: usize = 16_000;

fn truncate(s: String) -> String {
    if s.chars().count() <= TOOL_RESULT_MAX {
        s
    } else {
        let head: String = s.chars().take(TOOL_RESULT_MAX).collect();
        format!("{head}\n…[截断，原文过长]")
    }
}

/// 执行一次工具调用。Err = 工具层失败（回填给 LLM，不终止循环）。
async fn execute_tool(
    deps: &AgentDeps,
    lib: Uuid,
    name: &str,
    args: &Value,
    pages_touched: &mut Vec<String>,
) -> Result<String, String> {
    match name {
        "web_reader" => {
            let url = args["url"].as_str().ok_or("缺 url")?;
            let client = crate::wiki_docs::web_reader::WebReaderClient::from_pool(&deps.pool)
                .await
                .ok_or("web-reader 未配置（缺 zhipu/web_reader_key 凭据）")?;
            let page = client
                .read_url(url)
                .await
                .map_err(|e| format!("抓取失败: {e}"))?;
            Ok(truncate(
                json!({"title": page.title, "url": url, "content": page.content_markdown})
                    .to_string(),
            ))
        }
        "wiki_search" => {
            let q = args["query"].as_str().ok_or("缺 query")?;
            let limit = args["limit"].as_i64().unwrap_or(8).clamp(1, 30);
            let pages = deps
                .wiki()
                .search(lib, q, limit)
                .await
                .map_err(|e| e.to_string())?;
            Ok(truncate(
                pages
                    .iter()
                    .map(|p| json!({"slug": p.slug, "title": p.title}))
                    .collect::<Value>()
                    .to_string(),
            ))
        }
        "get_page" => {
            let slug = args["slug"].as_str().ok_or("缺 slug")?;
            let page = deps
                .wiki()
                .get_page(lib, slug)
                .await
                .map_err(|e| e.to_string())?;
            Ok(truncate(json!({"slug": page.slug, "title": page.title, "content": page.content}).to_string()))
        }
        "write_page" => {
            let slug = args["slug"].as_str().ok_or("缺 slug")?;
            let title = args["title"].as_str().ok_or("缺 title")?;
            let content = args["content"].as_str().ok_or("缺 content")?;
            let folder = args["folder"].as_str();
            let page = deps
                .wiki()
                .put_page(lib, slug, title, content, folder, Some("agent"))
                .await
                .map_err(|e| e.to_string())?;
            if !pages_touched.iter().any(|s| s == &page.slug) {
                pages_touched.push(page.slug.clone());
            }
            Ok(json!({"ok": true, "slug": page.slug, "title": page.title}).to_string())
        }
        "wiki_graph" => {
            let g = deps.wiki().graph(lib).await.map_err(|e| e.to_string())?;
            Ok(truncate(serde_json::to_string(&g).unwrap_or_default()))
        }
        "wiki_lint" => {
            let r = deps.wiki().lint(lib).await.map_err(|e| e.to_string())?;
            Ok(truncate(serde_json::to_string(&r).unwrap_or_default()))
        }
        "document_add" => {
            let doc_name = args["name"].as_str().ok_or("缺 name")?;
            let source = if let Some(url) = args["url"].as_str() {
                IngestSource::Url(url.to_string())
            } else if let Some(text) = args["text"].as_str() {
                IngestSource::Bytes {
                    name: doc_name.to_string(),
                    content: text.as_bytes().to_vec(),
                    content_type: Some("text/markdown".into()),
                }
            } else {
                return Err("url 与 text 必须二选一".into());
            };
            let (id, deduped) = deps
                .docs()
                .submit(lib, source)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({"document_id": id.to_string(), "deduped": deduped}).to_string())
        }
        "documents_search" => {
            let q = args["query"].as_str().ok_or("缺 query")?;
            let limit = args["limit"].as_i64().unwrap_or(6).clamp(1, 20);
            let chunks = deps
                .docs()
                .search(lib, q, limit)
                .await
                .map_err(|e| e.to_string())?;
            Ok(truncate(serde_json::to_string(&chunks).unwrap_or_default()))
        }
        "delete_page" => {
            let slug = args["slug"].as_str().ok_or("缺 slug")?;
            let deleted = deps
                .wiki()
                .delete_page(lib, slug)
                .await
                .map_err(|e| e.to_string())?;
            if deleted {
                pages_touched.retain(|s| s != slug);
            }
            Ok(json!({"deleted": deleted}).to_string())
        }
        "merge_pages" => {
            let primary = args["primary"].as_str().ok_or("缺 primary")?;
            let dup = args["duplicate"].as_str().ok_or("缺 duplicate")?;
            let result = deps
                .wiki()
                .merge_pages(lib, primary, dup)
                .await
                .map_err(|e| e.to_string())?;
            if let Some(pos) = pages_touched.iter().position(|s| s == dup) {
                pages_touched.remove(pos);
            }
            Ok(json!({"merged_into": primary, "detail": result}).to_string())
        }
        other => Err(format!("未知工具 {other}")),
    }
}

/// system 提示词：身份 + purpose + 工具纪律。
async fn build_system(deps: &AgentDeps, lib: Uuid) -> String {
    let purpose_text = match deps.wiki().get_purpose(lib).await {
        Ok(p) => serde_json::to_string_pretty(&p).unwrap_or_default(),
        Err(_) => "（未设置）".into(),
    };
    format!(
        "你是 engram wiki 库的维护 Agent。你通过工具读取/检索/写入 wiki 页面，\
把知识整理成结构化、互链的页面网络（Karpathy LLM Wiki 模式：wiki 即你的记忆）。\n\n\
## 本库方向意图（purpose）\n{purpose_text}\n\n\
## 工作纪律\n\
- 动笔前先 wiki_search / wiki_graph 查现状，避免重复建页；已有相关页则更新而非重建。\n\
- 页面内容用 markdown；跨页引用一律 [[slug]] 互链；一页一个概念，标题即概念名。\n\
- 原始资料用 document_add 入库，提炼后的知识写成 wiki 页；引用原文时用 documents_search 查证。\n\
- delete_page / merge_pages 是破坏性操作，仅在确凿重复或错误时使用。\n\
- 遇到工具报错，调整参数重试或换路径，不要重复同一失败调用。\n\
- 任务完成后不再调用工具，直接输出最终总结（做了什么、建/改了哪些页、遗留问题）。"
    )
}

/// 用户任务消息。
fn build_user_msg(task: &AgentTask) -> String {
    let mut msg = format!("## 任务\n{}\n", task.instruction);
    if let Some(url) = &task.source_url {
        msg.push_str(&format!("\n## 原料（URL）\n请先用 web_reader 抓取：<{url}>\n"));
    }
    if let Some(text) = &task.source_text {
        let name = task.source_name.as_deref().unwrap_or("未命名");
        msg.push_str(&format!(
            "\n## 原料（文本「{name}」）\n```\n{}\n```\n（可按需 document_add 入库原文）\n",
            truncate(text.clone())
        ));
    }
    msg
}

/// harness 主循环。
pub async fn run_agent<P: LlmProvider + 'static>(
    provider: Arc<P>,
    model: String,
    deps: &AgentDeps,
    task: &AgentTask,
    budget: AgentBudget,
    job_id: Option<Uuid>,
) -> Result<AgentReport, AgentError> {
    let started = Instant::now();
    let lib = task.lib;
    let defs = tool_defs();
    let mut messages = vec![
        ChatMessage::system(build_system(deps, lib).await),
        ChatMessage::user(build_user_msg(task)),
    ];
    let mut report = AgentReport {
        rounds: 0,
        llm_calls: 0,
        tool_calls: 0,
        pages_touched: Vec::new(),
        degraded: false,
        degraded_reason: None,
        summary: String::new(),
    };

    loop {
        // 预算闸门
        if report.rounds >= budget.max_rounds {
            report.degraded = true;
            report.degraded_reason = Some(format!("达到最大轮数 {}", budget.max_rounds));
            report.summary = "任务因达到最大轮数而中断，已完成部分保留。".into();
            return Ok(report);
        }
        if report.llm_calls >= budget.max_llm_calls {
            report.degraded = true;
            report.degraded_reason = Some(format!("达到最大 LLM 调用数 {}", budget.max_llm_calls));
            report.summary = "任务因 LLM 调用预算耗尽而中断，已完成部分保留。".into();
            return Ok(report);
        }
        if started.elapsed() > budget.timeout {
            report.degraded = true;
            report.degraded_reason = Some("总超时".into());
            report.summary = "任务因总超时而中断，已完成部分保留。".into();
            return Ok(report);
        }
        report.rounds += 1;

        let req = ChatRequest {
            model: model.clone(),
            messages: messages.clone(),
            temperature: Some(0.3),
            json_mode: false,
            max_tokens: None,
            tools: Some(defs.clone()),
        };
        let t0 = Instant::now();
        let resp = provider.chat(req).await.map_err(AgentError::Llm)?;
        report.llm_calls += 1;
        // 记账（与 P005 LLM 调用日志口径一致；绕过 registry.resolve 的直接调用在此补账）
        deps.registry
            .record_usage(&engram_llm::types::UsageMeta {
            provider: provider.name().to_string(),
            model: model.clone(),
            purpose: Purpose::WikiAgent.as_str().to_string(),
            input_tokens: resp.input_tokens,
            output_tokens: resp.output_tokens,
            latency_ms: t0.elapsed().as_millis() as i64,
            job_id,
        })
        .await;

        match resp.tool_calls {
            Some(calls) if !calls.is_empty() => {
                messages.push(ChatMessage::assistant_with_tool_calls(
                    resp.content.clone(),
                    calls.clone(),
                ));
                for call in calls {
                    let t1 = Instant::now();
                    let destructive = matches!(call.name.as_str(), "delete_page" | "merge_pages");
                    let result = execute_tool(deps, lib, &call.name, &parse_args(&call.arguments), &mut report.pages_touched).await;
                    report.tool_calls += 1;
                    let ok = result.is_ok();
                    // 审计（拍板②）：全工具留痕，破坏性标记 destructive
                    tracing::info!(
                        target: "wiki_agent",
                        audit = true,
                        tool = %call.name,
                        destructive,
                        ok,
                        elapsed_ms = t1.elapsed().as_millis() as i64,
                        job_id = job_id.map(|u| u.to_string()).unwrap_or_default(),
                        "harness 工具调用"
                    );
                    let content = match result {
                        Ok(s) => s,
                        Err(e) => format!("{{\"error\": {}}}", serde_json::to_string(&e).unwrap_or_default()),
                    };
                    messages.push(ChatMessage::tool_result(call.id.clone(), content));
                }
            }
            _ => {
                report.summary = resp.content;
                return Ok(report);
            }
        }
    }
}

fn parse_args(arguments: &str) -> Value {
    serde_json::from_str(arguments).unwrap_or_else(|_| json!({}))
}

/// 外部入队（MCP/HTTP 入口用）。
pub async fn enqueue_agent_task(
    queue: &engram_jobs::JobQueue,
    task: &AgentTask,
) -> Result<Uuid, JobError> {
    let job_id = Uuid::now_v7();
    queue
        .enqueue(
            JobTemplate::new("wiki_agent").with_payload(json!({"task": task})),
        )
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(job_id)
}

/// document_add → harness 自动接力（拍板③；ENGRAM_WIKI_AGENT_RELAY=0 关闭）。
/// sample = 文档正文取样（extracted 前 1500 字符），供 agent 判断知识价值。
pub async fn enqueue_relay_after_ready(ctx: &JobContext, lib: Uuid, doc_id: Uuid, sample: &str) {
    if std::env::var("ENGRAM_WIKI_AGENT_RELAY").ok().as_deref() == Some("0") {
        return;
    }
    let task = AgentTask {
        lib,
        instruction: format!(
            "刚有一篇新文档（id {doc_id}）完成入库（分块嵌入就绪）。请阅读下方正文取样，\
判断它值得沉淀哪些 wiki 知识页：有价值的概念建页（或并入已有页，先 wiki_search 查现状），\
并与库内相关页互链；纯参考资料则不强行建页。完成后总结你的取舍。"
        ),
        source_url: None,
        source_text: Some(sample.to_string()),
        source_name: Some(format!("doc-{doc_id}")),
    };
    if ctx
        .enqueue_next(JobTemplate::new("wiki_agent").with_payload(json!({"task": task})))
        .await
        .is_err()
    {
        tracing::warn!("wiki_agent 接力入队失败（不阻塞文档就绪）");
    }
}

/// AgentError → JobError（仅 LLM 瞬态可重试）。
fn to_job_err(e: AgentError) -> JobError {
    if e.retryable() {
        JobError::Retryable(e.to_string())
    } else {
        JobError::Permanent(e.to_string())
    }
}

/// 注册 wiki_agent handler（main 装配用；追加在 register_handlers 之后）。
pub fn register_agent_handler(
    runner: engram_jobs::Runner,
    registry: engram_llm::ProviderRegistry,
    data_dir: std::path::PathBuf,
) -> engram_jobs::Runner {
    runner.register("wiki_agent", move |ctx: JobContext| {
        let registry = registry.clone();
        let data_dir = data_dir.clone();
        async move {
            let task: AgentTask = ctx
                .job
                .payload
                .0
                .get("task")
                .cloned()
                .ok_or_else(|| JobError::Permanent("payload 缺 task".into()))
                .and_then(|v| serde_json::from_value(v).map_err(|e| JobError::Permanent(format!("task 解析失败: {e}"))))?;
            let job_id = ctx.job.id;
            let deps = AgentDeps {
                pool: ctx.pool().clone(),
                registry,
                data_dir,
            };
            // WikiAgent 独立档位解析（无配置 → Permanent：修配置才有意义）
            let (provider, model) = deps
                .registry
                .resolve(Purpose::WikiAgent)
                .await
                .map_err(|e| to_job_err(AgentError::Llm(e)))?;

            match run_agent(provider, model, &deps, &task, AgentBudget::default(), Some(job_id)).await {
                Ok(report) => {
                    let _ = ctx
                        .emit(
                            &format!(
                                "harness 完成：{} 轮 / {} 次 LLM / {} 次工具，触页 {:?}{}",
                                report.rounds,
                                report.llm_calls,
                                report.tool_calls,
                                report.pages_touched,
                                report
                                    .degraded_reason
                                    .as_ref()
                                    .map(|r| format!("（降级：{r}）"))
                                    .unwrap_or_default()
                            ),
                            None,
                        )
                        .await;
                    Ok(serde_json::to_value(&report).unwrap_or_else(|_| json!({})))
                }
                Err(e) => Err(to_job_err(e)),
            }
        }
    })
}

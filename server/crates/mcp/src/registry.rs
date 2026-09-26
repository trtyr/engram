//! registry 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

/// 工具名 → 所需 scope（域名前缀即 scope 名；管理台按同一前缀分域）。
/// 渐进式发现后常规工具就是 9 个域工具 + 跨域 search_all（scope 检查在 list_tools/
/// handler 内按"任一域"特判，不进本表；projects 的 scope 叫 project）；
/// 下面的平铺分支保留兜底（防御未来再加非域工具）。
pub(crate) fn tool_scope(name: &str) -> &'static str {
    match name {
        "projects" => "project",
        "assets" => "assets", // 资产台账域：独立一等对象（2026-09-21 新增）
        "credentials" => "credentials", // 凭据域：机密值一等台账（EN-234，2026-09-26 新增）
        "circles" => "memory", // 圈子域：实体坐标系读写（EN-229）——底座 memory 同表，scope 不分家（todos/tickets 先例）
        "memory" => "memory",
        "wiki" => "wiki",
        "todos" | "tickets" => "todos", // 工单域与待办同 scope（同表同底座，权限不分家）
        "codegraph" => "codegraph",
        other => flat_tool_scope(other),
    }
}

/// 平铺名兜底（防御未来再加非域工具）：按前缀归域。
pub(crate) fn flat_tool_scope(other: &str) -> &'static str {
    match other.split('_').next() {
        Some("project") => "project",
        Some("credentials") => "credentials",
        Some("circles") => "memory",
        Some("wiki") => "wiki",
        Some("codegraph") => "codegraph",
        Some("llm") => "llm",
        Some("todos") | Some("todo") => "todos",
        _ => "memory",
    }
}

/// MCP instructions：initialize 时返回给调用方 AI 的顶层使用说明。
pub(crate) const SERVER_INSTRUCTIONS: &str = "\
Engram —— 单用户 AI 长期记忆平台。MCP 工具面采用渐进式发现：十个领域各一个入口工具\
（memory 用户记忆 / projects 工作线 / assets 资产台账 / credentials 机密凭据 / circles 圈子实体图 / wiki 知识库 / todos 待办 / tickets 工单 / codegraph 代码图谱 / jobs 任务），\
外加跨域全局检索 search_all（一次查询并发四域，各回 top-k 摘要；机密域 credentials 不进全局检索）。\
域工具调用形态 {\"action\":\"<操作名>\", ...参数}；每个工具的描述里带操作目录（常驻可见），\
参数细节用 {\"action\":\"help\"} 一轮取回全域操作手册。

用户记忆分四层蒸馏：L0 原始会话 →（蒸馏）→ L1 原子事实 → L2 场景模式 → L3 用户画像；\
另有实体坐标系（人物/项目/主题/群组/地点）横向串联记忆。全部记忆可溯源、可遗忘（void 可 restore 撤销）。

memory 域用法（六动词，2026-09-26 收敛：存/找/翻/改/审/忘；旧动作名仍可用作别名）：
1. 会话开始：{\"action\":\"recall\",\"mode\":\"context\"} 装载用户画像与近期记忆，再开始对话；
2. 对话中需要背景：{\"action\":\"recall\",\"mode\":\"search\",\"query\":\"…\"} 定向回忆，或 {\"action\":\"recall\",\"mode\":\"entities\"} 按人/项目/主题查档案；翻清单用 {\"action\":\"browse\"}；
3. 记一句话事实：{\"action\":\"remember\",\"text\":\"…\"}；成段对话收尾 {\"action\":\"remember\",\"mode\":\"session\",\"turns\":[…]}（蒸馏自动沉淀），长对话分段 {\"action\":\"remember\",\"mode\":\"session_append\"}；
4. 纠错与画像：{\"action\":\"revise\",\"mode\":\"correct\"}（取代链留痕）/ {\"action\":\"revise\",\"mode\":\"persona\"}（分面编辑后蒸馏不覆盖）；蒸馏复核 {\"action\":\"review\"}；
5. 精确值逐字保存：{\"action\":\"remember\",\"mode\":\"kv\",\"key\":\"…\",\"value\":\"…\"}（序列号/端口/路径等，蒸馏零介入）；**机密凭据（API Key/Token/密码）不进 memory——走 credentials 域**（加密落库+取用审计）；
6. 用户明确表达遗忘：「别记住这个」→ {\"action\":\"forget\"}（void 会话作废且蒸馏产物级联归档；误作废用 mode=\"restore\" 撤销）。

projects 域用法：项目 = 一件有明确目标、一次干不完、跨多次会话推进的工作。
1. 开工：{\"action\":\"list\"} / {\"action\":\"get\"} 找到这件事的锚点接上上下文；没有就 {\"action\":\"create\"}；
2. 找内容：get 默认索引模式（文档只给 id/分类/标题/字符数）；{\"action\":\"doc_search\"} 定位到哪篇哪行，
   {\"action\":\"doc_get\",\"doc_id\":\"…\",\"start_line\":…,\"end_line\":…} 区间精读——按需取用，无截断；
3. 干活中：{\"action\":\"doc_add\"} / {\"action\":\"doc_update\"} 沉淀进展与结论；
   小修一段用 {\"action\":\"doc_patch\"}（行级 replace/insert/delete，不必取全文重发）；
4. 收尾：{\"action\":\"update\"} 改状态、写总结文档，下次会话从 get 接上。

skills 域已裁撤（2026-09-26，EN-252）：技能触发回归调用方本地目录；方法论沉淀在 wiki（skill-* 页）、engram 自身口径在 projects（[skills 迁入] 篇）。

wiki 域用法：单库知识库（Markdown 页面 + [[wikilink]] + 混合检索）。单库终局——无 library 参数，一切读写恒定在 main 主库。
1. 查证事实性知识 → {\"action\":\"search\"}（命中带片段，全文 get_page）；浏览结构 → {\"action\":\"list_pages\"} / {\"action\":\"graph\"}；
2. 沉淀：单条结论 {\"action\":\"archive_query\"}，整篇文档 {\"action\":\"ingest\"}（异步，产物落同库），明确要页面 {\"action\":\"write_page\"}（覆盖前先 get_page，旧文自动留版本）；
3. 版本与原料：{\"action\":\"versions\"}/{\"action\":\"restore_version\"} 查历史与回滚（误删页可重建）；{\"action\":\"sources\"}/{\"action\":\"delete_source\"} 清理织入原料（lint 报 stale_source 时用）。

todos 域用法：不绑定项目的快速待办（灵感/学习计划/系统操作——做完勾掉）。
{\"action\":\"add\",\"title\":\"…\"} 秒记；{\"action\":\"list\"} 看进行中；{\"action\":\"done\",\"id\":\"…\"} 完成。

tickets 域用法：工单 = 结构化问题跟踪（与待办同表不同心智，2026-09-18 拆域互不可见）。
{\"action\":\"add\",\"title\":\"…\",\"severity\":\"P1\",\"symptom\":\"…\",\"acceptance\":\"…\"} 开单；
{\"action\":\"list\"} 看工单（status 六态 open/confirmed/in_progress/resolved/verified/archived）；
{\"action\":\"update\",\"id\":\"…\",\"status\":\"resolved\",\"resolution\":\"…\"} 流转；链接关系（blocked_by/relates_to/parent）两域通用。

codegraph 域用法：注册代码库 → index/sync → {\"action\":\"query\"}（search/explore/node/callers/callees/impact）读懂调用关系。\
explore 默认返回符号大纲（不带源码）；单符号源码用 node，整份源码 explore 传 include_source=true。

域的选择：回忆「用户本人是谁、偏好什么、经历过什么」用 memory；实体关系/圈子视图用 circles；
查证「客观知识」用 wiki；跨会话的工作线用 projects；随手记行动项用 todos；开工单跟踪问题用 tickets；
API Key/Token 等机密存取用 credentials；不确定在哪域就 search_all。
LLM 供应商/模型的配置与排障是管理员专属，走 Web 控制台「设置 → AI 功能」——MCP 工具面
不提供 provider 配置工具（AI 报 LLM 未配置时，引导用户去设置页，不要尝试自行配置）。

分权规则（务必遵守）：
- 用户记忆的写入通道只有「写会话」：事实抽取、画像更新、实体维护全部由蒸馏完成；
- 直接改写用户记忆语义内容（原子内容、画像分面、实体档案）是用户专属权限，MCP 工具面不提供；
- 纠错也走会话：把正确的表述写成对话（correction 语义），蒸馏会自动生成取代链；
- 敏感对话（医疗/感情/财务等）写入时可置 sensitive=true——敏感是标记不是隐身，检索与上下文默认可见（2026-09-12 口径放开）；**机密凭据（账号/密码/密钥/token）一律走 credentials 域**（静态加密+按名取用+取用留痕，2026-09-26 EN-234 口径——不再走 kv_put/kv_get）；
- 破坏性操作（各域 delete/forget 类，目录里有【破坏性】标注）不可逆，只对用户明确请求使用。\
";

// ---------- 动态工具描述（发现能力长在工具面上） ----------
//
// 云端资产（技能/项目）的清单随库内容即时变化，AI 的发现通道只有 tools/list——
// 把「当前有什么可用」直接织进工具描述（Claude Code Skill 工具同款思路），
// 用户不需要在提示词里维护路由清单。tools/list 与控制台 /settings/mcp 同源拼装，
// 管理台看到的描述就是 AI 实际收到的描述。

/// 单工具动态清单条数上限（防清单膨胀无限挤占 AI 上下文；超出部分提示用工具查全量）。
pub(crate) const CATALOG_ITEM_CAP: usize = 40;

/// 按本次工具面实际包含的工具惰性取动态清单（工具不在面内就不查库）。
pub(crate) struct ToolCatalogs {
    projects: Option<String>,
    codegraph: Option<String>,
}

impl ToolCatalogs {
    pub(crate) async fn for_tools(pool: &engram_storage::PgPool, names: &[&str]) -> Self {
        let projects = if names.contains(&"projects") {
            projects_catalog(pool).await
        } else {
            None
        };
        let codegraph = if names.contains(&"codegraph") {
            codegraph_catalog(pool).await
        } else {
            None
        };
        Self {
            projects,
            codegraph,
        }
    }

    pub(crate) fn extra_for(&self, name: &str) -> Option<&str> {
        match name {
            "projects" => self.projects.as_deref(),
            "codegraph" => self.codegraph.as_deref(),
            _ => None,
        }
    }
}

/// 把动态清单段拼到工具描述尾部（tools/list 与控制台共用）。
pub(crate) fn with_dynamic_description(
    mut t: rmcp::model::Tool,
    extra: Option<&str>,
) -> rmcp::model::Tool {
    if let Some(extra) = extra {
        let base = t
            .description
            .as_ref()
            .map(|d| d.to_string())
            .unwrap_or_default();
        t.description = Some(std::borrow::Cow::Owned(format!("{base}\n\n{extra}")));
    }
    t
}

// ---------- MCP 配置（settings KV 单行：服务开关 + 工具粒度开关） ----------

/// MCP 配置（settings 表 key=`mcp`，jsonb）。缺省 = 全开。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpConfig {
    /// 服务总开关：false 时 /mcp 整体 503（AI 客户端连接/调用一律拒绝）
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 停用清单：域工具名（tools/list 不出现、call 报错）或 `域.action`（单操作停用，
    /// 从目录隐身 + call 拒绝）。历史扁平工具名的残留条目惰性忽略。
    #[serde(default)]
    pub disabled_tools: Vec<String>,
}
pub(crate) fn default_true() -> bool {
    true
}
impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            disabled_tools: Vec::new(),
        }
    }
}

pub(crate) const MCP_SETTINGS_KEY: &str = "mcp";

/// 读配置（行缺失 → 全开缺省；坏 JSON 按缺省处理，不让配置损坏打死端点）。
pub async fn load_config(pool: &engram_storage::PgPool) -> McpConfig {
    engram_storage::repo::settings::get_json::<McpConfig>(pool, MCP_SETTINGS_KEY)
        .await
        .unwrap_or_default()
}

/// 写配置（settings KV upsert）。
pub async fn save_config(
    pool: &engram_storage::PgPool,
    cfg: &McpConfig,
) -> engram_storage::StoreResult<()> {
    engram_storage::repo::settings::put_json(pool, MCP_SETTINGS_KEY, cfg).await?;
    Ok(())
}

/// 服务总开关拦截：关闭时 /mcp 一律 503（在 Bearer 之内、MCP 层之前——
/// 关闭是对外的，已认证客户端也拿不到 initialize）。
pub async fn gate(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if !load_config(&state.pool).await.enabled {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "error": {
                    "code": "mcp_disabled",
                    "message": "MCP 服务已关闭——控制台「MCP」页可重新开启",
                    "retryable": true,
                }
            })),
        )
            .into_response();
    }
    next.run(req).await
}

// ---------- 服务信息（Web 控制台「MCP」页用；HTTP handler 在 api 层） ----------

/// 域内操作条目（渐进式发现：域工具下的 action 清单，控制台两级展示 + 单操作开关）。
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct McpActionInfo {
    /// 操作名（与调用时 {"action":"…"} 一致）
    pub action: String,
    /// 一行摘要（与 AI 看到的 L0 目录同源）
    pub summary: String,
    /// 破坏性操作（不可逆/删除类）
    pub destructive: bool,
    /// 参数 JSON Schema（help 手册同源；控制台详情展示用）
    #[schema(value_type = Object)]
    pub parameters: serde_json::Value,
    /// 是否已被停用（disabled_tools 里的 `域.action`）
    pub disabled: bool,
}

/// MCP 工具条目（控制台工具清单；域工具下挂 actions）。
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct McpToolInfo {
    pub name: String,
    /// 所属资产域（渐进式发现后工具名即域名：memory/projects/skills/wiki/todos/codegraph）
    pub domain: String,
    pub description: String,
    pub read_only: Option<bool>,
    pub destructive: Option<bool>,
    /// 参数 JSON Schema（tools/list 的 inputSchema 同源；控制台详情展示用）
    #[schema(value_type = Object)]
    pub parameters: serde_json::Value,
    /// 域内操作（非域工具为空表）
    pub actions: Vec<McpActionInfo>,
}

/// MCP 服务信息。
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct McpInfo {
    /// MCP 端点路径（相对服务根）
    pub endpoint: String,
    /// MCP 协议版本
    pub protocol_version: String,
    pub server_name: String,
    pub server_version: String,
    /// 服务总开关（false = /mcp 整体 503）
    pub enabled: bool,
    /// 停用清单：域工具名（整个工具隐身）或 `域.action`（单操作停用）
    pub disabled_tools: Vec<String>,
    /// initialize 时下发给调用方 AI 的使用说明（与工具面同源展示）
    pub instructions: String,
    pub tools: Vec<McpToolInfo>,
}

/// 已知工具名 + 操作键清单（控制台配置校验用）。
pub fn tool_catalog() -> Vec<String> {
    let mut keys: Vec<String> = EngramMcpServer::build_tool_router()
        .list_all()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    for domain in dispatch::DOMAIN_TOOLS {
        if let Some(docs) = dispatch::action_docs(domain) {
            for d in docs {
                keys.push(dispatch::action_key(domain, d.action));
            }
        }
    }
    keys
}

/// 汇总服务信息（开关状态 + 工具清单）。与 tools/list 同源：控制台看到的描述
/// 就是 AI 实际收到的描述（含动态资产清单段）。
pub async fn build_info(pool: &engram_storage::PgPool) -> McpInfo {
    let cfg = load_config(pool).await;
    let router = EngramMcpServer::build_tool_router();
    let all = router.list_all();
    let names: Vec<&str> = all.iter().map(|t| t.name.as_ref()).collect();
    let catalogs = ToolCatalogs::for_tools(pool, &names).await;
    let tools = all
        .into_iter()
        .map(|t| {
            let name = t.name.as_ref().to_string();
            let asset = catalogs.extra_for(&name);
            let catalog = dispatch::render_catalog(&name, &cfg.disabled_tools);
            let extra = match (catalog, asset) {
                (Some(c), Some(a)) => Some(format!("{c}\n\n{a}")),
                (Some(c), None) => Some(c),
                (None, Some(a)) => Some(a.to_string()),
                (None, None) => None,
            };
            let t = with_dynamic_description(t, extra.as_deref());
            let actions = dispatch::action_docs(&name)
                .map(|docs| {
                    docs.iter()
                        .map(|d| McpActionInfo {
                            action: d.action.to_string(),
                            summary: d.summary.to_string(),
                            destructive: d.destructive,
                            parameters: (d.schema)(),
                            disabled: cfg
                                .disabled_tools
                                .iter()
                                .any(|x| x == &dispatch::action_key(&name, d.action)),
                        })
                        .collect()
                })
                .unwrap_or_default();
            McpToolInfo {
                name,
                domain: t.name.split('_').next().unwrap_or("other").to_string(),
                description: t.description.as_deref().unwrap_or("").to_string(),
                read_only: t.annotations.as_ref().and_then(|a| a.read_only_hint),
                destructive: t.annotations.as_ref().and_then(|a| a.destructive_hint),
                parameters: serde_json::to_value(&*t.input_schema).unwrap_or(serde_json::json!({})),
                actions,
            }
        })
        .collect();
    McpInfo {
        endpoint: "/mcp".into(),
        protocol_version: rmcp::model::ProtocolVersion::default().to_string(),
        server_name: "engram".into(),
        server_version: env!("CARGO_PKG_VERSION").into(),
        enabled: cfg.enabled,
        disabled_tools: cfg.disabled_tools,
        instructions: SERVER_INSTRUCTIONS.into(),
        tools,
    }
}

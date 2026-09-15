//! MCP（Model Context Protocol）适配器：渐进式发现工具面（六域 + 跨域 search_all）。
//!
//! 官方 Rust SDK（rmcp）Streamable HTTP 传输，由 api 装配到 engram-server 的 `/mcp` 端点。
//! 与 HTTP 路由平级的第二适配器：同一套 core 服务与 scope 分权，独立成 crate。
//! 渐进式发现（progressive disclosure）：六个领域各一个入口工具
//! （memory/projects/skills/wiki/todos/codegraph），域内操作经 action 分发
//! （历史 53 个扁平工具全部收编，R 测试报告后又扩至 60+ 个：remember/doc_patch/
//! versions/restore/sources 等；action 表在 dispatch 模块，三级发现同源）。
//! 各域工具在调用时检查各自 scope。鉴权复用 Bearer 中间件（amk_ key / ams_ 会话）：
//! 每个工具调用请求都过 `bearer_auth`，Principal 已注入 request extensions；
//! rmcp 把 HTTP request Parts 注入工具上下文，工具实现从这里取 Principal
//! 做与 HTTP API 同一套的 scope / 编辑分权检查。
//! key 吊销即刻生效（每个请求独立认证，会话保活也不能豁免）。

use rmcp::ServerHandler;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, ErrorCode, Implementation, ServerCapabilities, ServerInfo,
};
use rmcp::schemars::JsonSchema;
use rmcp::service::RequestContext;
use rmcp::service::RoleServer;
use rmcp::tool;
use rmcp::tool_handler;
use rmcp::tool_router;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::tower::{
    StreamableHttpServerConfig, StreamableHttpService,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

pub mod dispatch;
pub mod wiki;

use axum::response::IntoResponse;
use engram_core::auth::Principal;
use engram_core::state::AppState;
use engram_core::unified::UnifiedHit;

/// 错误桥：把本模块的错误统一成 MCP ErrorData。
pub(crate) fn mcp_err(code: ErrorCode, msg: impl Into<String>) -> rmcp::ErrorData {
    rmcp::ErrorData::new(code, msg.into(), None)
}

/// MemoryError → MCP 错误码（与 HTTP API 的 me() 同语义）。
fn from_memory(e: engram_core::memory::MemoryError) -> rmcp::ErrorData {
    use engram_core::memory::MemoryError;
    match e {
        MemoryError::NotFound(m) => rmcp::ErrorData::resource_not_found(m, None),
        MemoryError::BadRequest(m) => rmcp::ErrorData::invalid_params(m, None),
        MemoryError::Storage(m) => rmcp::ErrorData::internal_error(m, None),
        MemoryError::LlmNotConfigured(m) => rmcp::ErrorData::internal_error(m, None),
    }
}

/// ProjectError → MCP 错误码（与 HTTP API 的 pe() 同语义：NotFound/Conflict/BadRequest/Storage）。
fn from_project(e: engram_core::project::ProjectError) -> rmcp::ErrorData {
    use engram_core::project::ProjectError;
    match e {
        ProjectError::NotFound(m) => rmcp::ErrorData::resource_not_found(m, None),
        ProjectError::Conflict(m) => rmcp::ErrorData::new(ErrorCode::INVALID_REQUEST, m, None),
        ProjectError::BadRequest(m) => rmcp::ErrorData::invalid_params(m, None),
        ProjectError::Storage(m) => rmcp::ErrorData::internal_error(m, None),
    }
}

fn require_memory(principal: &Principal) -> Result<(), rmcp::ErrorData> {
    if principal.has_scope("memory") {
        Ok(())
    } else {
        Err(mcp_err(
            ErrorCode::INVALID_REQUEST,
            "缺少 memory scope——请用带 memory scope 的 amk_ key 连接 MCP",
        ))
    }
}

fn require_project(principal: &Principal) -> Result<(), rmcp::ErrorData> {
    if principal.has_scope("project") {
        Ok(())
    } else {
        Err(mcp_err(
            ErrorCode::INVALID_REQUEST,
            "缺少 project scope——请用带 project scope 的 amk_ key 连接 MCP",
        ))
    }
}

/// 工具名 → 所需 scope（域名前缀即 scope 名；管理台按同一前缀分域）。
/// 渐进式发现后常规工具就是 6 个域工具 + 跨域 search_all（scope 检查在 list_tools/
/// handler 内按"任一域"特判，不进本表；projects 的 scope 叫 project）；
/// 下面的平铺分支保留兜底（防御未来再加非域工具）。
fn tool_scope(name: &str) -> &'static str {
    match name {
        // 渐进式发现后的 6 个域工具：名即域（唯一例外 projects → project scope）
        "projects" => "project",
        "memory" => "memory",
        "skills" => "skills",
        "wiki" => "wiki",
        "todos" => "todos",
        "codegraph" => "codegraph",
        // 平铺名兜底（防御未来再加非域工具）
        other => match other.split('_').next() {
            Some("project") => "project",
            Some("skills") => "skills",
            Some("wiki") => "wiki",
            Some("codegraph") => "codegraph",
            Some("llm") => "llm",
            Some("todos") | Some("todo") => "todos",
            _ => "memory",
        },
    }
}

fn require_erase(principal: &Principal) -> Result<(), rmcp::ErrorData> {
    if principal.has_scope("erase") {
        Ok(())
    } else {
        Err(mcp_err(
            ErrorCode::INVALID_REQUEST,
            "擦除需要 erase scope（不可逆操作，与读写分权）——void 模式无需 erase",
        ))
    }
}

fn require_skills(principal: &Principal) -> Result<(), rmcp::ErrorData> {
    if principal.has_scope("skills") {
        Ok(())
    } else {
        Err(mcp_err(
            ErrorCode::INVALID_REQUEST,
            "缺少 skills scope——请用带 skills scope 的 amk_ key 连接 MCP",
        ))
    }
}

/// SkillsError → MCP 错误码（与 HTTP API 的 se() 同语义）。
fn from_skills(e: engram_core::skills::SkillsError) -> rmcp::ErrorData {
    use engram_core::skills::SkillsError;
    match e {
        SkillsError::NotFound(m) => rmcp::ErrorData::resource_not_found(m, None),
        SkillsError::Conflict(m) => rmcp::ErrorData::invalid_params(m, None),
        SkillsError::BadRequest(m) => rmcp::ErrorData::invalid_params(m, None),
        SkillsError::Storage(m) => rmcp::ErrorData::internal_error(m, None),
    }
}

fn require_todos(principal: &Principal) -> Result<(), rmcp::ErrorData> {
    if principal.has_scope("todos") {
        Ok(())
    } else {
        Err(mcp_err(
            ErrorCode::INVALID_REQUEST,
            "缺少 todos scope——请用带 todos scope 的 amk_ key 连接 MCP",
        ))
    }
}

fn todo_svc(state: &AppState) -> engram_core::todos::TodoService {
    engram_core::todos::TodoService::new(state.pool.clone())
}

/// TodoError → MCP 错误码。
fn from_todo(e: engram_core::todos::TodoError) -> rmcp::ErrorData {
    use engram_core::todos::TodoError;
    match e {
        TodoError::NotFound(m) => rmcp::ErrorData::resource_not_found(m, None),
        TodoError::BadRequest(m) => rmcp::ErrorData::invalid_params(m, None),
        TodoError::Storage(m) => rmcp::ErrorData::internal_error(m, None),
    }
}

/// CgError → MCP 错误码。
fn from_cg(e: engram_cg_bridge::CgError) -> rmcp::ErrorData {
    use engram_cg_bridge::CgError;
    match e {
        CgError::NotFound(m) => rmcp::ErrorData::resource_not_found(m, None),
        CgError::BadRequest(m) | CgError::VersionMismatch { need: m, got: _ } => {
            rmcp::ErrorData::invalid_params(m, None)
        }
        other => rmcp::ErrorData::internal_error(other.to_string(), None),
    }
}

/// CodeGraph 桥（root 与 api 层同一约定：data_dir/codegraph）。
fn cg_svc(state: &AppState) -> engram_cg_bridge::CgBridge {
    engram_cg_bridge::CgBridge::new(state.pool.clone(), state.data_dir.join("codegraph"))
}

fn require_codegraph(principal: &Principal) -> Result<(), rmcp::ErrorData> {
    if principal.has_scope("codegraph") {
        Ok(())
    } else {
        Err(mcp_err(
            ErrorCode::INVALID_REQUEST,
            "缺少 codegraph scope——请用带 codegraph scope 的 amk_ key 连接 MCP",
        ))
    }
}

/// 项目寻址：名字优先（AI 友好），uuid 亦可。
async fn cg_resolve(state: &AppState, project: &str) -> Result<uuid::Uuid, rmcp::ErrorData> {
    let bridge = cg_svc(state);
    if let Ok(id) = uuid::Uuid::parse_str(project) {
        bridge.get(id).await.map_err(from_cg)?;
        return Ok(id);
    }
    let rows = bridge.list().await.map_err(from_cg)?;
    rows.iter()
        .find(|r| r.name == project)
        .map(|r| r.id)
        .ok_or_else(|| {
            let known: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
            mcp_err(
                ErrorCode::RESOURCE_NOT_FOUND,
                format!("项目 {project:?} 不存在——已注册：{}", known.join("、")),
            )
        })
}

/// 从工具调用上下文取 HTTP 请求里的 Principal（bearer_auth 已认证并注入）。
fn principal_of(ctx: &RequestContext<RoleServer>) -> Result<Principal, rmcp::ErrorData> {
    let parts = ctx
        .extensions
        .get::<axum::http::request::Parts>()
        .ok_or_else(|| mcp_err(ErrorCode::INTERNAL_ERROR, "内部错误：缺少 HTTP 请求上下文"))?;
    parts
        .extensions
        .get::<Principal>()
        .cloned()
        .ok_or_else(|| mcp_err(ErrorCode::INTERNAL_ERROR, "内部错误：缺少认证主体"))
}

/// 宽容时间解析：RFC3339 全形态或 date-only（与 HTTP API opt_flex_dt 同一套语义）。
fn parse_flex_datetime(s: &str) -> Result<chrono::DateTime<chrono::Utc>, rmcp::ErrorData> {
    let t = s.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t) {
        return Ok(dt.into());
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d")
        && let Some(ndt) = d.and_hms_opt(0, 0, 0)
    {
        return Ok(chrono::DateTime::from_naive_utc_and_offset(
            ndt,
            chrono::Utc,
        ));
    }
    Err(mcp_err(
        ErrorCode::INVALID_PARAMS,
        format!("无法解析时间 {s:?}：期望 ISO8601（2026-09-02 或 2026-09-02T00:00:00Z）"),
    ))
}

fn ok_json(v: serde_json::Value) -> Result<CallToolResult, rmcp::ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(
        serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string()),
    )]))
}

// ---------- MCP 面返回瘦身（R 报告 P0-1：写操作不回显正文） ----------
//
// 调用方刚发送的正文原样返回是纯浪费（正文已在调用方上下文里）。
// HTTP API 保持全量（Web UI 依赖），MCP 面统一只回元数据——渐进式「列表层」字段集。

/// 通用瘦身：删 body/content 等正文键，补 content_chars。
fn slim_content(v: serde_json::Value, content_keys: &[&str]) -> serde_json::Value {
    let mut v = v;
    let mut chars = 0i64;
    if let Some(obj) = v.as_object_mut() {
        for key in content_keys {
            if let Some(s) = obj.remove(*key).and_then(|x| x.as_str().map(String::from)) {
                chars = s.chars().count() as i64;
            }
        }
        obj.insert("content_chars".into(), json!(chars));
        obj.insert("content_omitted".into(), json!(true));
    }
    v
}

/// 会话写入瘦身：turns 数组换轮次数（全文走 get_session）。
fn slim_session(s: serde_json::Value) -> serde_json::Value {
    let mut v = s;
    if let Some(obj) = v.as_object_mut() {
        let turns = obj.remove("content");
        let n = turns.as_ref().and_then(|t| t.as_array()).map(|a| a.len());
        obj.insert("turns".into(), json!(n.unwrap_or(0)));
        obj.insert(
            "hint".into(),
            json!("已入库（轮次数见 turns）——原文用 get_session 回读；蒸馏产物几分钟后可 search/list_atoms 看到"),
        );
    }
    v
}

/// 技能写操作瘦身：正文换 content_chars。
fn slim_skill(s: serde_json::Value) -> serde_json::Value {
    slim_content(s, &["content"])
}

/// 项目文档瘦身：正文换 content_chars。
fn slim_doc(d: serde_json::Value) -> serde_json::Value {
    slim_content(d, &["content"])
}

/// 待办瘦身：正文换 content_chars（title/状态/时间全保留）。
fn slim_todo(t: serde_json::Value) -> serde_json::Value {
    slim_content(t, &["body"])
}

/// 递归删除对象键（context include_evidence=false 时剥溯源字段）。
fn strip_keys(v: &mut serde_json::Value, keys: &[&str]) {
    match v {
        serde_json::Value::Object(m) => {
            for k in keys {
                m.remove(*k);
            }
            for (_, child) in m.iter_mut() {
                strip_keys(child, keys);
            }
        }
        serde_json::Value::Array(a) => {
            for child in a.iter_mut() {
                strip_keys(child, keys);
            }
        }
        _ => {}
    }
}

// ---------- 工具参数 ----------

/// memory_context / memory_search 公共可选参数里的时间串直接用 String（ISO8601）。

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TodoAddParams {
    /// 一句话标题（必填）
    #[schemars(description = "一句话标题（必填，≤200 字）。")]
    pub title: String,
    /// 详情（可选 markdown）
    #[schemars(description = "详情（可选 markdown）。")]
    pub body: Option<String>,
    /// low | normal | high（缺省 normal）
    #[schemars(description = "优先级：low/normal/high，缺省 normal。")]
    pub priority: Option<String>,
    /// 自由标签
    #[schemars(description = "自由标签（如 学习/系统操作/问题排查）。")]
    pub tags: Option<Vec<String>>,
    /// 截止时间（ISO8601，可选）
    #[schemars(description = "可选截止时间（ISO8601）。")]
    pub due_at: Option<String>,
    /// 相关项目名提示（纯文本备注，不绑定）
    #[schemars(description = "可选：相关项目名提示（纯文本备注，不绑定项目）。")]
    pub project_hint: Option<String>,
    /// 形态：todo（行动项，默认）/ ticket（工单）
    #[schemars(
        description = "可选形态：todo（行动项，默认）/ ticket（工单——结构化问题跟踪，建议填 severity/symptom/acceptance）。"
    )]
    pub kind: Option<String>,
    /// 工单严重度 P0-P3（仅 kind=ticket）
    #[schemars(description = "可选：工单严重度 P0/P1/P2/P3（仅 kind=ticket）。")]
    pub severity: Option<String>,
    /// 工单症状（仅 kind=ticket）
    #[schemars(description = "工单症状/现象描述（仅 kind=ticket）。")]
    pub symptom: Option<String>,
    /// 工单复现路径（仅 kind=ticket）
    #[schemars(description = "工单复现路径（仅 kind=ticket）。")]
    pub reproduce: Option<String>,
    /// 工单验收标准（仅 kind=ticket）
    #[schemars(description = "工单验收标准（仅 kind=ticket）。")]
    pub acceptance: Option<String>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct TodoLinkParams {
    /// 源条目（UUID 或 EN-<短号>）
    #[schemars(description = "源条目（UUID 或 EN-<短号>）。")]
    pub from: String,
    /// 目标条目（UUID 或 EN-<短号>）
    #[schemars(description = "目标条目（UUID 或 EN-<短号>）。")]
    pub to: String,
    /// 关联类型：blocked_by（from 被 to 阻塞）/ relates_to（相关）/ parent（to 是 from 的父项）
    #[schemars(description = "关联类型：blocked_by / relates_to / parent。")]
    pub kind: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct TodoUnlinkParams {
    /// 源条目
    #[schemars(description = "源条目（UUID 或 EN-<短号>）。")]
    pub from: String,
    /// 目标条目
    #[schemars(description = "目标条目（UUID 或 EN-<短号>）。")]
    pub to: String,
    /// 关联类型
    #[schemars(description = "关联类型：blocked_by / relates_to / parent。")]
    pub kind: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct TodoLinksParams {
    /// 目标条目（UUID 或 EN-<短号>）
    #[schemars(description = "目标条目（UUID 或 EN-<短号>）。")]
    pub id: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct TodoListParams {
    /// open | done | archived（缺省全部，open 优先展示）
    #[schemars(description = "可选状态过滤：open/done/archived。缺省全部（open 优先）。")]
    pub status: Option<String>,
    /// 可选：todo / ticket
    #[schemars(description = "可选形态过滤：todo / ticket。")]
    pub kind: Option<String>,
    /// low | normal | high
    #[schemars(description = "可选优先级过滤。")]
    pub priority: Option<String>,
    /// 标签过滤
    #[schemars(description = "可选标签过滤。")]
    pub tag: Option<String>,
    /// 标题/正文子串
    #[schemars(description = "可选子串过滤（标题或正文）。")]
    pub q: Option<String>,
    /// 摘要模式（MCP 默认 true）：只返回 短号/标题/形态/状态/分级/标签/关联计数
    #[schemars(
        description = "可选：摘要模式，默认 true——只返回 short_no/标题/形态/状态/分级/标签/关联计数（不含 body/symptom 等长字段）。brief=false 返回全量。"
    )]
    pub brief: Option<bool>,
    /// keyset 分页游标（D29）：上一页最后一条的 {1|0}|{updated_at ISO8601}|{id}——
    /// 1 表示该条 status=open。首查不传；结果恰为 limit 条时继续传游标取下一页
    #[schemars(
        description = "可选：keyset 分页游标。取上一页最后一条构造：{1|0}|{updated_at ISO8601}|{id}（1=该条 status 为 open，否则 0）。首查不传；返回条数恰等于 limit 时说明可能还有下一页。"
    )]
    pub cursor: Option<String>,
    /// 条数上限（缺省 50，单页上限 500——更多结果用 cursor 翻页）
    #[schemars(description = "条数上限（缺省 50，单页上限 500——更多结果用 cursor 翻页）。")]
    pub limit: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TodoIdParams {
    /// 待办 id（todo_list 返回）
    #[schemars(description = "待办 id（todo_list 返回）。")]
    pub id: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TodoUpdateParams {
    /// 待办 id（todo_list 返回）
    #[schemars(description = "待办 id（todo_list 返回）。")]
    pub id: String,
    /// 可选：形态转换 todo ↔ ticket
    #[schemars(description = "可选：形态转换 todo ↔ ticket（转换后工单字段生效）。")]
    pub kind: Option<String>,
    /// 新标题（可选）
    #[schemars(description = "可选新标题。")]
    pub title: Option<String>,
    /// 新详情（可选）
    #[schemars(description = "可选新详情。")]
    pub body: Option<String>,
    /// low | normal | high
    #[schemars(description = "可选优先级：low/normal/high。")]
    pub priority: Option<String>,
    /// todo: open/done/archived；ticket: open/confirmed/in_progress/resolved/verified/archived
    #[schemars(
        description = "可选状态——todo: open/done/archived；ticket（工单）: open/confirmed/in_progress/resolved/verified/archived。"
    )]
    pub status: Option<String>,
    /// 工单严重度 P0-P3（仅 kind=ticket）
    #[schemars(description = "可选：工单严重度 P0/P1/P2/P3（仅 kind=ticket）。")]
    pub severity: Option<String>,
    /// 工单症状（仅 kind=ticket）
    #[schemars(description = "工单症状/现象描述（仅 kind=ticket）。")]
    pub symptom: Option<String>,
    /// 工单复现路径（仅 kind=ticket）
    #[schemars(description = "工单复现路径（仅 kind=ticket）。")]
    pub reproduce: Option<String>,
    /// 工单验收标准（仅 kind=ticket）
    #[schemars(description = "工单验收标准（仅 kind=ticket）。")]
    pub acceptance: Option<String>,
    /// 工单解决记录（resolved 前必填）
    #[schemars(description = "工单解决记录——状态转 resolved 前必填（写了什么方案/修了什么）。")]
    pub resolution: Option<String>,
    /// 截止时间（ISO8601，可选）
    #[schemars(description = "可选截止时间（ISO8601）。")]
    pub due_at: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ContextParams {
    /// 可选相关性查询（缺省按最近 + 30 天半衰期重排）；会话开始时通常不传
    #[schemars(
        description = "可选：相关性查询词。会话冷启动时不传（按最近+新鲜度），带着具体问题回忆时传。"
    )]
    pub query: Option<String>,
    /// 各层条数预算（默认 20）
    #[schemars(description = "各层返回条数预算，默认 20。通常不需要调。")]
    pub budget_items: Option<usize>,
    /// 总字符预算（默认 8000）
    #[schemars(description = "总字符数预算，默认 8000。通常不需要调。")]
    pub budget_chars: Option<usize>,
    /// 证据溯源（默认关）
    #[schemars(
        description = "可选：true = 携带证据溯源字段（persona.evidence_refs / atom.source_refs / scenario.atom_refs 的 ID 数组）。默认 false——溯源 ID 客户端几乎不消费，省上下文（R 报告 P1-5）；审计需要时再开。"
    )]
    pub include_evidence: Option<bool>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct MemoryKvPutParams {
    /// 键（唯一；同 key 再写 = 就地覆盖更新，不产生历史链）
    #[schemars(description = "键（唯一，≤200 字符）。同 key 再次写入 = 覆盖更新（就地改值）。")]
    pub key: String,
    /// 值（逐字保存——蒸馏永不加工；序列号/UUID/IP/端口等精确值原样存）
    #[schemars(
        description = "值（逐字保存，零加工）。精确值如序列号/UUID/IP:PORT 原样存，禁概括。"
    )]
    pub value: String,
    /// 可选：说明（这是哪台机器的什么值、怎么探到的）
    #[schemars(description = "可选：上下文说明。")]
    pub context: Option<String>,
    /// 可选：标签
    #[schemars(description = "可选：标签数组。")]
    pub tags: Option<Vec<String>>,
    /// 可选：来源 user_stated/verified_probe/agent_inferred/doc（缺省 user_stated）
    #[schemars(
        description = "可选：值来源。user_stated=用户明示, verified_probe=实测探得, agent_inferred=推断, doc=文档。"
    )]
    pub source: Option<String>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct MemoryDistillResultParams {
    /// 会话 id（write_session 返回的 id）
    #[schemars(description = "会话 id（write_session 返回的 id）。")]
    pub session_id: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct MemoryKvGetParams {
    /// 键
    #[schemars(description = "要读的键。")]
    pub key: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct MemoryKvListParams {
    /// 返回上限（默认 50，≤500）
    #[schemars(description = "可选：返回上限（按 updated_at 倒序）。默认 50。")]
    pub limit: Option<i64>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct MemoryKvSearchParams {
    /// 字面量（≥3 字符；key/value/context ILIKE 直查——精确值不依赖分词）
    #[schemars(
        description = "字面量检索词（≥3 字符）。对 key/value/context 做 ILIKE 直查——序列号等精确值用这个，不依赖分词。"
    )]
    pub query: String,
    /// 返回上限（默认 20，≤100）
    #[schemars(description = "可选：返回上限。默认 20。")]
    pub limit: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SearchParams {
    /// 检索词
    #[schemars(description = "检索词。中英文均可，混合检索（全文+向量）。")]
    pub query: String,
    /// 层过滤：["l1","l2","l3","entities"]，空 = 全部
    #[schemars(
        description = "可选：限定检索层。l1=原子事实, l2=场景, l3=画像, entities=人物/项目/主题/群组/地点。空 = 全部层。"
    )]
    pub layers: Option<Vec<String>>,
    /// 每层最大命中数（默认 20）
    #[schemars(description = "每层最大命中数，默认 20。")]
    pub max_items: Option<i64>,
    /// 时间范围起点（ISO8601；occurred_at 优先，NULL 回退 created_at）
    #[schemars(
        description = "可选：时间范围起点，ISO8601（如 2026-09-01 或 2026-09-01T00:00:00Z）。"
    )]
    pub from: Option<String>,
    /// 时间范围终点
    #[schemars(description = "可选：时间范围终点，ISO8601。")]
    pub to: Option<String>,
    /// 证据溯源（默认关）
    #[schemars(
        description = "可选：true = L3 画像命中携带 evidence_refs（溯源 ID 数组）。默认 false——检索消费者不用溯源 ID，与 context 同口径（验收遗留 #1）；审计需要时再开。"
    )]
    pub include_evidence: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ListAtomsParams {
    /// 原子类型：preference/fact/decision/event/insight/correction/failure/convention
    #[schemars(
        description = "可选：按类型过滤。preference=偏好, fact=事实, decision=决策, event=事件, insight=洞察, correction=纠正, failure=失败, convention=惯例。"
    )]
    pub kind: Option<String>,
    /// active（默认）/ superseded / archived / candidate / all
    #[schemars(
        description = "可选：按状态过滤，默认 \"active\"（只看有效记忆）。superseded=被取代, archived=归档, candidate=候选；\"all\" = 全部状态（巡检历史时用）。"
    )]
    pub status: Option<String>,
    /// true = 只看待审（低置信度）；false = 只看已审
    #[schemars(
        description = "可选：按待审标记过滤（needs_review=true 是低置信度、建议用户复核的条目）。"
    )]
    pub needs_review: Option<bool>,
    /// keyset 分页游标（上一页最后一条的 created_at，ISO8601）
    #[schemars(description = "可选：分页游标。传上一页最后一条的 created_at（ISO8601）取下一页。")]
    pub cursor: Option<String>,
    /// 每页条数（默认 100）
    #[schemars(description = "每页条数，默认 100。")]
    pub limit: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ListSessionsParams {
    /// 按 agent 名过滤（写入时的客户端/助手标识）
    #[schemars(description = "可选：按 agent 名过滤（写入会话时的客户端标识，如 claude-code）。")]
    pub agent: Option<String>,
    /// keyset 分页游标（上一页最后一条的 created_at，ISO8601）
    #[schemars(description = "可选：分页游标。传上一页最后一条的 created_at（ISO8601）取下一页。")]
    pub cursor: Option<String>,
    /// 每页条数（默认 50）
    #[schemars(description = "每页条数，默认 50。")]
    pub limit: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct GetSessionParams {
    /// 会话 id（UUID）
    #[schemars(description = "会话 id（UUID，来自 memory_list_sessions 的返回）。")]
    pub session_id: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct Turn {
    /// 说话方：user 或 assistant
    #[schemars(description = "说话方：\"user\" 或 \"assistant\"。")]
    pub speaker: String,
    /// 该轮内容
    #[schemars(description = "该轮对话内容原文。")]
    pub text: String,
    /// 可选：发生时间（ISO8601）
    #[schemars(description = "可选：该轮发生时间（ISO8601）。不传由服务端记写入时间。")]
    pub ts: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WriteSessionParams {
    /// 对话轮次
    #[schemars(description = "对话轮次数组，按时间顺序。只写值得记忆的对话即可。")]
    pub turns: Vec<Turn>,
    /// auto（默认，防抖自动蒸馏）| manual（立即蒸馏）| off
    #[schemars(
        description = "蒸馏模式：\"auto\"（默认，写入后 ~30 秒窗口合并蒸馏）/ \"manual\"（立即触发蒸馏）/ \"off\"（**永久豁免**——该会话不会被任何自动或手动蒸馏扫到，适合只归档不提炼的内容）。一般用默认。"
    )]
    pub distill: Option<String>,
    /// 会话级敏感标记（医疗/感情/财务等隐私）：蒸馏产物继承标记。2026-09-12 口径放开——敏感是标记不是隐身，检索/上下文默认可见。
    #[schemars(
        description = "整段对话含用户隐私（医疗/感情/财务等）时置 true：蒸馏产物自动继承敏感标记。2026-09-12 口径放开——sensitive 是标记不是隐身，检索与上下文默认可见（DTO 带 sensitive=true 供识别）。"
    )]
    pub sensitive: Option<bool>,
    /// agent 归因（缺省用连接本服务的 API key 名）
    #[schemars(
        description = "可选：agent 归因名（标识是哪个客户端写的）。缺省用连接本服务的 API key 名。"
    )]
    pub agent: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct AppendSessionParams {
    /// 会话 id（UUID）
    #[schemars(description = "会话 id（UUID）。仅未蒸馏的会话可追加。")]
    pub session_id: String,
    /// 追加的对话轮次
    #[schemars(description = "追加的对话轮次数组（同 memory_write_session 的 turns）。")]
    pub turns: Vec<Turn>,
    /// auto（默认，防抖）| off
    #[schemars(
        description = "蒸馏模式：\"auto\"（默认，合并进 ~30 秒防抖窗）/ \"off\"（永久豁免蒸馏）。"
    )]
    pub distill: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ForgetParams {
    /// 会话 id（UUID）
    #[schemars(description = "会话 id（UUID）。restore 模式 = 要恢复的会话 id。")]
    pub session_id: String,
    /// void（默认：蒸馏跳过，记录保留）| erase（需 erase scope：物理删除）| restore（撤销 void）
    #[schemars(
        description = "遗忘力度：\"void\"（默认，推荐——会话作废、原文保留，已蒸馏产物自动级联归档）/ \"erase\"（物理删除，需要 erase scope 的 key）/ \"restore\"（撤销 void：恢复会话与被级联归档的原子——误作废的后悔药）。"
    )]
    pub mode: Option<String>,
}

/// 一句话记忆（R 报告 P1-9）：记条小事实不必手搓 turns 数组。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct RememberParams {
    /// 要记住的一句话（字段名 text，≤120 字）
    #[schemars(
        description = "要记住的事实/偏好/事件，一句话 ≤120 字（如「用户的猫叫墨鱼，喜欢趴键盘上睡觉」）。字段名是 text。等价于单轮 write_session + auto 蒸馏；成段内容请走 write_session。"
    )]
    pub text: Option<String>,
    /// 会话级敏感标记
    #[schemars(
        description = "可选：敏感内容（医疗/感情/财务）置 true——敏感是标记不是隐身，产物默认可见并带 sensitive 标记。"
    )]
    pub sensitive: Option<bool>,
    /// agent 归因
    #[schemars(description = "可选：agent 归因名。缺省用连接本服务的 API key 名。")]
    pub agent: Option<String>,
    /// 显式断言强度：仅支持 "fact"——用户亲口明示的事实用这个（直写落库不走蒸馏，保原话）；
    /// 缺省走蒸馏（产物默认 inference，保守不升格）
    #[schemars(
        description = "可选：仅支持 fact——用户亲口明示的事实直写落库（不走蒸馏，原话保真）。缺省走蒸馏（默认 inference）。"
    )]
    pub strength: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct EntitiesParams {
    /// 检索词（人名/项目名/主题词）
    #[schemars(description = "检索词：人名、项目名、主题词等。名字命中权重最高。")]
    pub q: String,
    /// 返回条数（默认 20）
    #[schemars(description = "返回条数，默认 20。")]
    pub limit: Option<i64>,
}

/// 跨域全局检索（R 报告 P1-8：把 6 次单域搜索并成 1 次）。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SearchAllParams {
    /// 检索词（各域同词并发检索）
    #[schemars(
        description = "检索词。对 key 有 scope 的域并发检索（memory/wiki/skills/todos/projects）。"
    )]
    pub query: String,
    /// R6：可选 LLM 精排（默认关）——开时五域命中合并 top-10 交 LLM 重排，响应附 reranked 视图
    #[schemars(
        description = "可选：LLM 精排（默认关）。开时五域命中合并 top-10 交 LLM 重排，响应附 reranked 视图（LLM 失败降级原分组）。"
    )]
    pub rerank: Option<bool>,
    /// 每域返回条数（默认 3）
    #[schemars(description = "每域返回条数上限，默认 3。结果只有摘要——精确检索请用单域工具。")]
    pub max_per_domain: Option<i64>,
}

// ---------- 项目记忆工具参数 ----------

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectTypesParams {}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectListParams {
    /// 可选：按类型过滤（dev=开发 / research=调研）
    #[schemars(description = "可选：按类型过滤。dev=开发, research=调研。")]
    #[serde(rename = "type")]
    pub type_: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectGetParams {
    /// 项目 id（UUID，来自 project_list / project_get）
    #[schemars(description = "项目 id（UUID，来自 project_list / project_get 的返回）。")]
    pub project_id: Option<String>,
    /// 项目名（项目名唯一，可代替 id 定位）
    #[schemars(description = "项目名（唯一，可代替 project_id 定位）。与 project_id 至少给一个。")]
    pub project_name: Option<String>,
    /// 索引模式（默认）：docs 只给 id/分类/标题/content_chars，不带正文
    #[schemars(
        description = "默认 false（索引模式）：docs 只含 id/分类/标题/content_chars（正文字符数），不带正文——先看结构，再 project_doc_search 定位或 project_doc_get 精读。true = 全量带上每篇正文（项目文档很少时可用，无损）。"
    )]
    pub include_content: Option<bool>,
    /// 只看某个分类下的文档
    #[schemars(description = "可选：只返回该分类下的文档（分类名须与项目 categories 一致）。")]
    pub category: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectCreateParams {
    /// 项目名（唯一）
    #[schemars(description = "项目名（唯一）。起一个能认出「这是哪件事」的名字。")]
    pub name: String,
    /// dev=开发 / research=调研
    #[schemars(
        description = "项目类型：\"dev\"（开发，预置分类 后端/前端/测试/规划）或 \"research\"（调研，预置 待查/线索/资料/结论/疑点/证伪）。类型只决定初始分类，建后可自由增删。"
    )]
    #[serde(rename = "type")]
    pub type_: String,
    /// 项目描述（一句话说清目标）
    #[schemars(description = "可选：项目描述，一句话说清目标和范围。")]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectUpdateParams {
    /// 定位：项目 id（UUID）
    #[schemars(description = "项目 id（UUID）。与 project_name 至少给一个。")]
    pub project_id: Option<String>,
    /// 定位：项目名
    #[schemars(description = "项目名（唯一）。与 project_id 至少给一个。")]
    pub project_name: Option<String>,
    /// 新名字（改名用）
    #[schemars(description = "可选：改成的新项目名（项目名唯一，撞名会报错）。不传不改名。")]
    pub new_name: Option<String>,
    /// 新状态：active / paused / done / abandoned
    #[schemars(
        description = "可选：新状态。active=进行中, paused=暂停, done=完成, abandoned=放弃。不传不改。"
    )]
    pub status: Option<String>,
    /// 新描述（传空串清除）
    #[schemars(description = "可选：新描述。传空串清除描述。不传不改。")]
    pub description: Option<String>,
    /// 分类列表（替换式；删分类不删该分类下的文档）
    #[schemars(
        description = "可选：替换整个分类列表（如 [\"后端\",\"前端\",\"规划\"]）。注意是替换不是追加；删分类不删该分类下的文档。不传不改。"
    )]
    pub categories: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectDeleteParams {
    /// 项目 id（UUID）
    #[schemars(description = "项目 id（UUID）。")]
    pub project_id: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectBatchDeleteParams {
    /// 要删除的项目 id 列表
    #[schemars(description = "要删除的项目 id 列表。返回 deleted 条数与不存在的 id。")]
    pub ids: Vec<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectLocationAddParams {
    /// 定位项目：项目 id（UUID）
    #[schemars(description = "项目 id（UUID）。与 project_name 至少给一个。")]
    pub project_id: Option<String>,
    /// 定位项目：项目名
    #[schemars(description = "项目名（唯一）。与 project_id 至少给一个。")]
    pub project_name: Option<String>,
    /// 主机 IP（内网/公网/IPv6 均可，仅登记不校验格式）
    #[schemars(
        description = "主机 IP（内网/公网/IPv6 均可，仅登记不校验格式；本机可填 127.0.0.1）。"
    )]
    pub ip: String,
    /// 主机名
    #[schemars(description = "主机名（如 MacBook Pro / tencent-beijing）。")]
    pub host: String,
    /// 操作系统
    #[schemars(description = "操作系统（macOS / Ubuntu / Windows…）。")]
    pub os: String,
    /// 项目在主机上的文件夹路径
    #[schemars(description = "项目在该主机上的文件夹路径（纯登记，服务端不会读取该路径）。")]
    pub path: String,
    /// 用途（开发 / 部署 / …）
    #[schemars(description = "可选：该位置的用途（开发 / 部署 / 测试机…）。")]
    pub purpose: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectLocationUpdateParams {
    /// 位置 id（UUID，来自 project_get 的 locations 列表）
    #[schemars(description = "位置 id（UUID，来自 project_get 返回的 locations 列表）。")]
    pub location_id: String,
    /// 新 IP
    #[schemars(description = "可选：新 IP。不传不改。")]
    pub ip: Option<String>,
    /// 新主机名
    #[schemars(description = "可选：新主机名。不传不改。")]
    pub host: Option<String>,
    /// 新操作系统
    #[schemars(description = "可选：新操作系统。不传不改。")]
    pub os: Option<String>,
    /// 新路径
    #[schemars(description = "可选：新路径。不传不改。")]
    pub path: Option<String>,
    /// 新用途
    #[schemars(description = "可选：新用途。不传不改。")]
    pub purpose: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectLocationDeleteParams {
    /// 位置 id（UUID）
    #[schemars(description = "位置 id（UUID，来自 project_get 返回的 locations 列表）。")]
    pub location_id: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectDocAddParams {
    /// 定位项目：项目 id（UUID）
    #[schemars(description = "项目 id（UUID）。与 project_name 至少给一个。")]
    pub project_id: Option<String>,
    /// 定位项目：项目名
    #[schemars(description = "项目名（唯一）。与 project_id 至少给一个。")]
    pub project_name: Option<String>,
    /// 分类名（须是项目已有分类；新分类先 project_update 追加）
    #[schemars(
        description = "分类名，必须是项目已有分类（先 project_get 看 categories）。要新分类就先 project_update 把它加进 categories。同项目同分类下 title 唯一。"
    )]
    pub category: String,
    /// 文档标题
    #[schemars(description = "文档标题（同项目同分类同 folder 下唯一）。")]
    pub title: String,
    /// 可选：子文件夹相对路径
    #[schemars(
        description = "可选：子文件夹相对路径（/ 分隔多级，如 审计、归档/ai-permissions；'' = 分类根下）。树形呈现：分类 → folder → 文档。"
    )]
    pub folder: Option<String>,
    /// Markdown 正文
    #[schemars(description = "Markdown 正文。沉淀进展、结论、决策时写清楚背景与结果。")]
    pub content: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectDocGetParams {
    /// 文档 id（UUID）
    #[schemars(description = "文档 id（UUID，来自 project_get 返回的 docs 列表）。")]
    pub doc_id: String,
    /// 起始行（1-based；与 end_line 搭配精确读一个区间）
    #[schemars(
        description = "可选：起始行号（1-based，含该行）。与 end_line 搭配做区间精读；行号来自 project_doc_search 的命中或 with_line_numbers 的全文。不传 = 从头。"
    )]
    pub start_line: Option<i64>,
    /// 结束行（1-based，含该行）
    #[schemars(description = "可选：结束行号（1-based，含该行）。不传 = 到末尾。")]
    pub end_line: Option<i64>,
    /// 输出加行号前缀（区间模式恒带行号；全文默认不加）
    #[schemars(
        description = "可选：true = 全文每行加「行号: 」前缀，便于后续按行寻址。不传或 false = 原文。区间读取（传了 start_line/end_line）恒带行号。"
    )]
    pub with_line_numbers: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectDocSearchParams {
    /// 定位项目：项目 id（UUID）
    #[schemars(description = "项目 id（UUID）。与 project_name 至少给一个。")]
    pub project_id: Option<String>,
    /// 定位项目：项目名
    #[schemars(description = "项目名（唯一）。与 project_id 至少给一个。")]
    pub project_name: Option<String>,
    /// 检索词（按行大小写不敏感子串匹配）
    #[schemars(description = "检索词：按行大小写不敏感子串匹配（grep 式）。")]
    pub query: String,
    /// 命中上限（默认 50）
    #[schemars(description = "命中上限，默认 50。命中含 doc_id/title/category/line/text。")]
    pub limit: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectDocUpdateParams {
    /// 文档 id（UUID）
    #[schemars(description = "文档 id（UUID，来自 project_get 返回的 docs 列表）。")]
    pub doc_id: String,
    /// 新分类
    #[schemars(description = "可选：移到新分类（须是项目已有分类）。不传不改。")]
    pub category: Option<String>,
    /// 新子文件夹
    #[schemars(
        description = "可选：改子文件夹相对路径（/ 分隔多级；'' = 移到分类根下）。不传不改。"
    )]
    pub folder: Option<String>,
    /// 新标题
    #[schemars(description = "可选：新标题。不传不改。")]
    pub title: Option<String>,
    /// 新正文（替换式；先 project_doc_get 取原文再追加修改）
    #[schemars(
        description = "可选：替换整个 Markdown 正文（是替换不是追加——改长文档先 project_doc_get 取原文）。不传不改。"
    )]
    pub content: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectDocDeleteParams {
    /// 文档 id（UUID）
    #[schemars(description = "文档 id（UUID，来自 project_get 返回的 docs 列表）。")]
    pub doc_id: String,
}

/// 行级补丁（R 报告 P1-10）：改长文档不再「doc_get 取全文→doc_update 重发全文」。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectDocPatchParams {
    /// 文档 id（UUID）
    #[schemars(description = "文档 id（UUID，来自 project_get 返回的 docs 列表）。")]
    pub doc_id: String,
    /// 起始行（1-based）
    #[schemars(
        description = "行号（1-based）。replace/delete = 区间起点；insert = 插入位置（在该行之前，total+1 = 追加到末尾）。"
    )]
    pub start_line: i64,
    /// 结束行（1-based，含该行）
    #[schemars(
        description = "行号（1-based，含该行）。replace/delete = 区间终点；insert 忽略此参数。"
    )]
    pub end_line: i64,
    /// replace（默认）| insert | delete
    #[schemars(
        description = "补丁模式：\"replace\"（默认，[start_line,end_line] 替换为 content）/ \"insert\"（在 start_line 前插入 content，可传 start_line=total+1 追加）/ \"delete\"（删除 [start_line,end_line]，忽略 content）。"
    )]
    pub mode: Option<String>,
    /// 替换/插入的文本（可多行；delete 忽略）
    #[schemars(description = "替换或插入的文本（可多行）。mode=delete 时不需要。")]
    pub content: Option<String>,
}

// ---------- 技能域工具参数 ----------

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsListParams {
    /// 可选关键词（搜名称与描述）
    #[schemars(description = "可选：关键词，模糊匹配技能名称与描述。")]
    pub q: Option<String>,
    /// 可选标签过滤
    #[schemars(description = "可选：按标签过滤（含该标签即命中）。")]
    pub tag: Option<String>,
    /// 可选启用过滤：true=只看启用 / false=只看停用 / 缺省=全部
    #[schemars(description = "可选：true 只看启用，false 只看停用，缺省全部。")]
    pub enabled: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CgRegisterParams {
    /// 项目名（唯一，如 engram-server）
    #[schemars(description = "项目名（唯一，如 engram-server）。")]
    pub name: String,
    /// 本地绝对路径或 git URL
    #[schemars(description = "本地绝对路径（如 D:\\Code\\Rust\\engram）或 git URL。")]
    pub source_uri: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CgNameParams {
    /// 项目名（codegraph_list 里的 name；也接受 id）
    #[schemars(description = "项目名（codegraph_list 返回的 name；也接受 uuid）。")]
    pub project: String,
}

/// 无参操作（codegraph list）占位：inputSchema 根类型须为 object（同 wiki::WikiNoParams）。
#[derive(Serialize, Deserialize, JsonSchema, Default)]
pub struct CgNoParams {}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CgQueryParams {
    /// 项目名（codegraph_list 里的 name；也接受 id）
    #[schemars(description = "项目名（codegraph_list 返回的 name；也接受 uuid）。")]
    pub project: String,
    /// explore | search | node | callers | callees | impact
    #[schemars(
        description = "查询类型：search=搜符号, explore=区域符号大纲(默认不带源码), node=符号详情含源码, callers=谁调用它, callees=它调用谁, impact=改动影响面。"
    )]
    pub kind: String,
    /// 查询文本或符号名
    #[schemars(
        description = "查询文本（search/explore）或符号名（node/callers/callees/impact）。注意 explore 按目录名或符号定位，不支持按单文件文件名查询。"
    )]
    pub target: String,
    /// explore→max-files；impact→depth
    #[schemars(
        description = "可选：explore 的 max-files（仅 include_source=true 生效）或 impact 的 depth。"
    )]
    pub depth: Option<u32>,
    /// explore 是否带完整源码（默认 false）
    #[schemars(
        description = "可选，仅 explore 生效：true = CLI 原生输出（含完整源码，体积大）；默认 false = 符号大纲（name/kind/行号/签名，无源码——单个符号的源码用 kind=node 取）。"
    )]
    pub include_source: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsGetParams {
    /// 技能 slug（kebab-case 标识）
    #[schemars(
        description = "技能 slug（来自 skills_list 的返回，如 review-pr）。也接受技能名 name 精确匹配。"
    )]
    pub slug: String,
}

/// 版本列表（R 报告建议 #5：skills 快照已在存，MCP 此前未暴露）。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsVersionsParams {
    /// 技能 slug（或技能名）
    #[schemars(description = "技能 slug（或技能名）。")]
    pub slug: String,
}

/// 回滚到历史版本。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsRestoreParams {
    /// 技能 slug（或技能名）
    #[schemars(description = "技能 slug（或技能名）。")]
    pub slug: String,
    /// 目标版本 id（versions 列表返回的 id，非 rev 序号）
    #[schemars(
        description = "目标版本 id（来自 versions 列表的 id 字段）。回滚本身也留版本快照，可再滚回来。"
    )]
    pub revision_id: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsFileGetParams {
    /// 技能 slug（来自 skills_list 的返回）
    #[schemars(description = "技能 slug（来自 skills_list 的返回）。")]
    pub slug: String,
    /// 附属文件相对路径（来自 skills_get 返回的 files 索引）
    #[schemars(description = "附属文件相对路径（/ 分隔，如 scripts/run.py、references/api.md）。")]
    pub path: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsFilePutParams {
    /// 技能 slug（来自 skills_list 的返回）
    #[schemars(description = "技能 slug（来自 skills_list 的返回）。")]
    pub slug: String,
    /// 附属文件相对路径（/ 分隔；禁止 .. 与绝对路径；SKILL.md 本体走 skills_update）
    #[schemars(
        description = "附属文件相对路径（/ 分隔，如 scripts/run.py）。禁止 .. 与绝对路径；SKILL.md 本体走 skills_update。"
    )]
    pub path: String,
    /// 文件文本内容（脚本/参考资料/模板）
    #[schemars(description = "文件文本内容（脚本/参考资料/模板）。同路径重复写 = 覆盖更新。")]
    pub content: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsCreateParams {
    /// 技能名
    #[schemars(description = "技能名（简短、可辨认，如「PR 审查」）。")]
    pub name: String,
    /// markdown 正文（技能指令本体）
    #[schemars(description = "技能正文，markdown。写清这个技能做什么、怎么做、何时用。")]
    pub content: String,
    /// 可选 slug（缺省从 name 推导；中文/非 ASCII 名必须显式给）
    #[schemars(
        description = "可选：slug（kebab-case 标识，如 review-pr）。缺省从 name 推导；name 非 ASCII 时必须显式给。"
    )]
    pub slug: Option<String>,
    /// 可选一句话描述
    #[schemars(description = "可选：一句话描述这个技能做什么、何时该用（列表与选择时的依据）。")]
    pub description: Option<String>,
    /// 可选标签
    #[schemars(description = "可选：标签列表，便于分类过滤。")]
    pub tags: Option<Vec<String>>,
    /// 可选：存储形态（text/script）
    #[schemars(
        description = "可选：text=整体入库（默认，纯文本技能，依赖走 npm/cargo 全局二进制时也用这个）；script=带 .py/.sh 等真脚本的技能——真身存本地文件夹（SKILL.md+scripts/），库中只存指针，正文不入库（get 现读），file/versions 操作不可用。"
    )]
    pub kind: Option<String>,
    /// 可选：来源（self/github/both）
    #[schemars(
        description = "可选：self=自建未发布（默认）/ github=源自 GitHub / both=自建且已发布。"
    )]
    pub origin: Option<String>,
    /// script 型必填：本地技能文件夹路径
    #[schemars(
        description = "kind=script 时必填：本地技能文件夹绝对路径（含 SKILL.md），系统只存指针。kind=text 时不要传。"
    )]
    pub local_path: Option<String>,
    /// 可选：GitHub 仓库地址（元数据）
    #[schemars(
        description = "可选：GitHub 仓库地址（origin=github 或 both 时填）。纯元数据，不做远端拉取。"
    )]
    pub repo_url: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsUpdateParams {
    /// 目标技能 slug
    #[schemars(description = "要更新的技能 slug。")]
    pub slug: String,
    /// 可选：改技能名
    #[schemars(description = "可选：新技能名。不传不动。")]
    pub name: Option<String>,
    /// 可选：改描述
    #[schemars(description = "可选：新描述。不传不动。")]
    pub description: Option<String>,
    /// 可选：改正文
    #[schemars(description = "可选：新正文（markdown，整体替换）。不传不动。")]
    pub content: Option<String>,
    /// 可选：改标签
    #[schemars(description = "可选：新标签列表（整体替换）。不传不动。")]
    pub tags: Option<Vec<String>>,
    /// 可选：启用/停用
    #[schemars(
        description = "可选：true=启用 / false=停用。停用后不再出现在 enabled=true 过滤里；缺省列表是管理视角仍会显示（停用技能不被删除）。"
    )]
    pub enabled: Option<bool>,
    /// 可选：改来源（self/github/both）
    #[schemars(
        description = "可选：改来源。self=自建未发布 / github=源自 GitHub / both=自建且已发布。origin 改回 self 时 repo_url 自动清空。"
    )]
    pub origin: Option<String>,
    /// 可选：改仓库地址
    #[schemars(description = "可选：改 GitHub 仓库地址（origin 含 github 时有意义）。")]
    pub repo_url: Option<String>,
    /// 可选：script 型指针改址
    #[schemars(description = "可选：script 型技能改本地路径（指针搬家）。text 型不可用。")]
    pub local_path: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsDeleteParams {
    /// 目标技能 slug
    #[schemars(description = "要删除的技能 slug（级联删版本快照，不可逆）。")]
    pub slug: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsImportParams {
    /// SKILL.md 全文
    #[schemars(
        description = "SKILL.md 全文：可选 --- frontmatter（name/description/slug/tags 键）+ markdown 正文。没有 frontmatter 时需传 filename 或 name 兜底。"
    )]
    pub content: String,
    /// 可选文件名（无 frontmatter name 时兜底命名）
    #[schemars(
        description = "可选：来源文件名（如 review-pr.md），无 frontmatter name 时用来兜底命名。"
    )]
    pub filename: Option<String>,
    /// 可选：技能名（frontmatter 与 filename 都没有时兜底）
    #[schemars(description = "可选：技能名兜底（frontmatter name 与 filename 都缺时必须给）。")]
    pub name: Option<String>,
    /// 命中已有 slug 时覆盖更新（默认 false）
    #[schemars(description = "可选：slug 已存在时是否覆盖更新，默认 false（该条报错）。")]
    pub overwrite: Option<bool>,
}

// ---------- MCP 服务器 ----------

/// Engram MCP 服务器（用户记忆域 + Wiki 域）。工具实现进程内直调各域 service（不走 HTTP 回环）。
#[derive(Clone)]
pub struct EngramMcpServer {
    state: AppState,
    tool_router: ToolRouter<Self>,
}

impl EngramMcpServer {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    fn svc(&self) -> engram_core::memory::MemoryService {
        engram_core::memory::MemoryService::new(self.state.pool.clone(), self.state.registry())
    }

    fn svc_project(&self) -> engram_core::project::ProjectService {
        engram_core::project::ProjectService::new(self.state.pool.clone())
    }

    fn skills_svc(&self) -> engram_core::skills::SkillsService {
        engram_core::skills::SkillsService::new(self.state.pool.clone())
    }

    /// wiki 库 slug → 库 id（缺省 main）。多库（2026-09-08）：所有 wiki 操作按库隔离。
    async fn resolve_wiki_lib(&self, library: Option<&str>) -> Result<Uuid, rmcp::ErrorData> {
        engram_core::wiki::libraries::resolve(&self.state.pool, library)
            .await
            .map_err(wiki::from_wiki)
    }

    /// project_id / project_name 二选一定位项目 id（项目名唯一，可寻址）。
    async fn resolve_project(
        &self,
        id: &Option<String>,
        name: &Option<String>,
    ) -> Result<Uuid, rmcp::ErrorData> {
        match (id, name) {
            (Some(id), _) => Uuid::parse_str(id)
                .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "project_id 不是合法 UUID")),
            (None, Some(name)) => self
                .svc_project()
                .project_id_by_name(name)
                .await
                .map_err(from_project),
            (None, None) => Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                "需要 project_id 或 project_name 之一来定位项目",
            )),
        }
    }
}

#[tool_router]
impl EngramMcpServer {
    /// 装载用户记忆上下文包（L3 画像 + L2 场景 + L1 原子事实 + 实体，按预算裁剪）。
    ///
    /// 何时用：会话开始时调用一次，冷启动装载「这个用户是谁、在忙什么、有什么偏好与约束」。
    /// 何时不用：需要回忆某个具体细节时用 memory_search（更省 token）；本工具是全景而非定向检索。
    /// 返回：persona（画像分面）、scenarios（场景）、atoms（原子事实）、entities（实体）、
    /// pending_review（待用户复核的低置信度条目，可顺带提醒用户）。
    async fn memory_context(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ContextParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let pack = self
            .svc()
            .context_pack(
                params.0.query.as_deref(),
                params.0.budget_items.unwrap_or(20),
                params.0.budget_chars.unwrap_or(8000),
                false,
            )
            .await
            .map_err(from_memory)?;
        let mut v = serde_json::to_value(&pack).unwrap_or(serde_json::json!({}));
        // P1-5：证据溯源 ID 默认不携带（审计时显式 include_evidence=true）
        if !params.0.include_evidence.unwrap_or(false) {
            strip_keys(&mut v, &["evidence_refs", "source_refs", "atom_refs"]);
        }
        ok_json(v)
    }

    /// 定向检索用户记忆（全文 + 向量混合，跨 L1/L2/L3/实体四层）。
    ///
    /// 何时用：对话中需要回忆与当前话题相关的用户背景、既往决策、偏好、历史事件时。
    /// 何时不用：会话开场的全景装载用 memory_context；浏览全量列表用 memory_list_atoms。
    /// 命中会回写热度（hit_count），常被检索的内容会在整理中获得更高权重。
    /// sensitive 条目默认可见（2026-09-12 口径放开——标记保留不隐身）；返回 {entities, l1, l2, l3}，各元素含 score/title/snippet；
    /// L3 画像默认不带 evidence_refs（与 context 同口径，include_evidence=true 开）。
    async fn memory_search(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SearchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let sp = params.0;
        let from = sp.from.as_deref().map(parse_flex_datetime).transpose()?;
        let to = sp.to.as_deref().map(parse_flex_datetime).transpose()?;
        let layers: Vec<&str> = sp.layers.iter().flatten().map(|s| s.as_str()).collect();
        let resp = self
            .svc()
            .search(
                &sp.query,
                &layers,
                sp.max_items.unwrap_or(20),
                false,
                from,
                to,
            )
            .await
            .map_err(from_memory)?;
        let mut v = serde_json::to_value(&resp).unwrap_or(serde_json::json!({}));
        // 与 context 同口径（验收遗留 #1）：L3 画像命中的 evidence_refs 默认不携带
        if !sp.include_evidence.unwrap_or(false) {
            strip_keys(&mut v, &["evidence_refs", "source_refs", "atom_refs"]);
        }
        ok_json(v)
    }

    /// 浏览 L1 原子事实列表（keyset 分页，可按类型/状态/待审过滤）。
    ///
    /// 何时用：需要系统性浏览用户的事实条目（而非定向检索）时；或巡检 needs_review 条目。
    /// 何时不用：有明确主题的回忆用 memory_search；开场装载用 memory_context。
    async fn memory_list_atoms(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ListAtomsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let lp = params.0;
        let cursor = lp.cursor.as_deref().map(parse_flex_datetime).transpose()?;
        // P1-6：默认 active——文档一直写着「默认查这个」，行为此前却是全量。
        // "all" 是显式出口（巡检历史）。
        let status = match lp.status.as_deref() {
            None | Some("") | Some("active") => Some("active"),
            Some("all") => None,
            Some(other) => {
                return Err(mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    format!(
                        "status 只支持 active/superseded/archived/candidate/all（收到 {other:?}）"
                    ),
                ));
            }
        };
        let atoms = self
            .svc()
            .list_atoms(
                lp.kind.as_deref(),
                status,
                lp.needs_review,
                cursor,
                lp.limit.unwrap_or(100),
            )
            .await
            .map_err(from_memory)?;
        ok_json(serde_json::to_value(&atoms).unwrap_or(serde_json::json!([])))
    }

    // ---------- KV 值保值通道（蒸馏零介入——精确值原样透传） ----------

    /// 写入/更新结构化值（kv_put）：同 key 就地覆盖——序列号/UUID/IP:PORT 等精确值的正道。
    async fn memory_kv_put(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<MemoryKvPutParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let kp = params.0;
        let row = self
            .svc()
            .kv_put(
                &kp.key,
                &kp.value,
                kp.context.as_deref(),
                kp.tags,
                kp.source.as_deref(),
            )
            .await
            .map_err(from_memory)?;
        ok_json(serde_json::to_value(&row).unwrap_or(serde_json::json!({})))
    }

    /// 蒸馏回执（distill_result）：查一次会话蒸馏产出了哪些原子（id/内容/强度/状态）。
    async fn memory_distill_result(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<MemoryDistillResultParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let sid = uuid::Uuid::parse_str(params.0.session_id.trim()).map_err(|_| {
            rmcp::ErrorData::invalid_params(
                format!("session_id 不是合法 UUID: {}", params.0.session_id),
                None,
            )
        })?;
        let v = self.svc().distill_result(sid).await.map_err(from_memory)?;
        ok_json(v)
    }

    /// 读取结构化值（kv_get）。
    async fn memory_kv_get(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<MemoryKvGetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let row = self
            .svc()
            .kv_get(&params.0.key)
            .await
            .map_err(from_memory)?;
        match row {
            Some(r) => ok_json(serde_json::to_value(&r).unwrap_or(serde_json::json!({}))),
            None => Err(rmcp::ErrorData::invalid_params(
                format!("kv 键不存在: {}", params.0.key),
                None,
            )),
        }
    }

    /// 列出全部 KV（kv_list，按 updated_at 倒序）。
    async fn memory_kv_list(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<MemoryKvListParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let rows = self
            .svc()
            .kv_list(params.0.limit.unwrap_or(50))
            .await
            .map_err(from_memory)?;
        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
    }

    /// 字面量直查 KV（kv_search）——精确值不依赖分词，ILIKE 全字段。
    async fn memory_kv_search(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<MemoryKvSearchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let rows = self
            .svc()
            .kv_search(&params.0.query, params.0.limit.unwrap_or(20))
            .await
            .map_err(from_memory)?;
        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
    }

    /// 列出 L0 原始会话（keyset 分页，可按 agent 过滤）。
    ///
    /// 何时用：找某段对话的原文入口时（拿到 session_id 后用 memory_get_session 看全文）。
    async fn memory_list_sessions(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ListSessionsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let lp = params.0;
        let cursor = lp.cursor.as_deref().map(parse_flex_datetime).transpose()?;
        // D6（重构）：列表是浏览/定位场景——返回轻量元数据（轮次数 + 首条预览），
        // 不截断不裁剪正文；定位到目标后用 memory_get_session 取完整原文
        let sessions = self
            .svc()
            .list_sessions_meta(lp.agent.as_deref(), cursor, lp.limit.unwrap_or(50))
            .await
            .map_err(from_memory)?;
        ok_json(serde_json::to_value(&sessions).unwrap_or(serde_json::json!([])))
    }

    /// 读取一个 L0 原始会话全文（逐轮对话原文）。
    ///
    /// 何时用：memory_search / memory_list_sessions 定位到会话后，需要核对原文细节时。
    async fn memory_get_session(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<GetSessionParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let id = Uuid::parse_str(&params.0.session_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "session_id 不是合法 UUID"))?;
        let s = self.svc().get_session(id).await.map_err(from_memory)?;
        ok_json(serde_json::to_value(&s).unwrap_or(serde_json::json!({})))
    }

    /// 写入一段对话到 L0 会话（AI 记忆的主写入口；蒸馏自动抽取事实/场景/画像/实体）。
    ///
    /// 何时用：会话收尾或告一段落时，把值得记忆的对话内容写入。只写有长期价值的部分
    /// （用户的事实、偏好、决策、事件），不要逐字搬运全程闲聊。
    /// 蒸馏（distill="auto"）会把原文抽取为 L1 原子事实并沉淀 L2/L3，全程带 prompt 溯源。
    /// 注意：不要用本工具「纠正」既有记忆——把纠正后的内容写成对话（可含 correction 语义），
    /// 蒸馏会自动生成取代链；直接改写语义内容是用户（Web 登录态）专属权限。
    async fn memory_write_session(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<WriteSessionParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let principal = principal_of(&ctx)?;
        require_memory(&principal)?;
        let wp = params.0;
        let turns = serde_json::to_value(&wp.turns).unwrap_or(serde_json::json!([]));
        let agent = wp.agent.unwrap_or_else(|| match &principal {
            Principal::ApiKey { name, .. } => name.clone(),
            Principal::Admin => "admin".into(),
        });
        let s = self
            .svc()
            .write_session(
                &agent,
                turns,
                wp.distill.as_deref().unwrap_or("auto"),
                wp.sensitive.unwrap_or(false),
            )
            .await
            .map_err(from_memory)?;
        // P0-1：turns 原文不回显（调用方刚发过——回显是纯浪费）
        ok_json(slim_session(
            serde_json::to_value(&s).unwrap_or(serde_json::json!({})),
        ))
    }

    /// 向一个未蒸馏的会话追加轮次（长对话分片落库，不必等收尾一次性写）。
    ///
    /// 何时用：同一会话持续进行、已用 memory_write_session 开头后，后续内容追加进来。
    /// 已蒸馏的会话不可追加（会报错）——那就新开一个会话。
    async fn memory_append_session(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<AppendSessionParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let principal = principal_of(&ctx)?;
        require_memory(&principal)?;
        let ap = params.0;
        let id = Uuid::parse_str(&ap.session_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "session_id 不是合法 UUID"))?;
        let turns = serde_json::to_value(&ap.turns).unwrap_or(serde_json::json!([]));
        let s = self
            .svc()
            .append_session(id, turns, None, ap.distill.as_deref().unwrap_or("auto"))
            .await
            .map_err(from_memory)?;
        ok_json(slim_session(
            serde_json::to_value(&s).unwrap_or(serde_json::json!({})),
        ))
    }

    /// 一句话记忆（R 报告 P1-9）：记条小事实不必手搓 turns 数组。
    ///
    /// 何时用：用户说了值得长期记住的一句话事实/偏好/事件时（「记住我的猫叫墨鱼」）。
    /// 何时不用：成段对话收尾用 write_session（上下文更完整，蒸馏质量更高）。
    async fn memory_remember(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<RememberParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let principal = principal_of(&ctx)?;
        require_memory(&principal)?;
        let rp = params.0;
        // R（参数摩擦收口）：缺字段/为空/超长统一给一条完整约束提示——
        // 此前 serde missing field 报错不带字段名与上限，AI 每次冷启动要试错多轮
        let Some(raw) = rp.text else {
            return Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "remember 需要正文字段 text（上限 {} 字）——注意字段名是 text 不是 content；strength=fact 直写原话时内容需 ≤120 字；成段内容走默认蒸馏路径或 write_session",
                    engram_core::memory::TURN_TEXT_MAX_CHARS
                ),
            ));
        };
        let text = raw.trim().to_string();
        if text.is_empty() {
            return Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "text 不能为空——要记住的内容一句话写清楚（上限 {} 字）；成段内容请走 write_session",
                    engram_core::memory::TURN_TEXT_MAX_CHARS
                ),
            ));
        }
        if text.chars().count() > engram_core::memory::TURN_TEXT_MAX_CHARS {
            return Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "text 超长（当前 {} 字，上限 {} 字）——remember text 自身可到上限；但 strength=fact 直写原话限 120 字，超长请去掉 strength=fact 走默认蒸馏或用 write_session",
                    text.chars().count(),
                    engram_core::memory::TURN_TEXT_MAX_CHARS
                ),
            ));
        }
        let agent = rp.agent.unwrap_or_else(|| match &principal {
            Principal::ApiKey { name, .. } => name.clone(),
            Principal::Admin => "admin".into(),
        });
        // 显式 fact：用户亲口明示的事实直写落库（不走蒸馏——原话保真，不走概括）。
        // 这是用户授权的提格入口；蒸馏默认 inference 的保守原则只约束无声明路径。
        if rp.strength.as_deref() == Some("fact") {
            if rp.sensitive.unwrap_or(false) {
                return Err(mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    "sensitive 与 strength=fact 互斥——敏感内容走会话蒸馏通道",
                ));
            }
            let a = self
                .svc()
                .create_atom(
                    "fact",
                    &text,
                    0.9,
                    None,
                    None,
                    false,
                    Some("fact"),
                    Some("user_stated"),
                )
                .await
                .map_err(from_memory)?;
            let mut v = serde_json::to_value(&a).unwrap_or(serde_json::json!({}));
            v["hint"] = json!("已记住（显式 fact 直写——原话保真，不走蒸馏）");
            return ok_json(v);
        }
        let turns = serde_json::json!([{ "speaker": "user", "text": text }]);
        let s = self
            .svc()
            .write_session(&agent, turns, "auto", rp.sensitive.unwrap_or(false))
            .await
            .map_err(from_memory)?;
        let mut v = slim_session(serde_json::to_value(&s).unwrap_or(serde_json::json!({})));
        v["hint"] = json!("已记住（auto 蒸馏，几分钟内可 search 命中）");
        ok_json(v)
    }

    /// 遗忘：用户说「别记住这段/把这事忘了」时使用，对任何会话都有效。
    ///
    /// mode="void"（默认）：该会话作废、原文保留可审计；若会话已蒸馏，其产出的
    /// 原子（active 与 superseded）会**级联归档**——检索与上下文包立即不再返回它们。
    /// mode="erase"：物理删除该会话及其派生原子的检索可见性（不可逆，需要 erase scope 的 key）。
    /// mode="restore"：撤销 void——误作废的后悔药，恢复会话与被归档的原子。
    /// 注意：只对用户明确表达的遗忘请求使用，不要自行判断「这段不重要」就遗忘。
    async fn memory_forget(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ForgetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let principal = principal_of(&ctx)?;
        require_memory(&principal)?;
        let fp = params.0;
        let id = Uuid::parse_str(&fp.session_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "session_id 不是合法 UUID"))?;
        match fp.mode.as_deref().unwrap_or("void") {
            "void" => {
                let s = self.svc().void_session(id).await.map_err(from_memory)?;
                ok_json(serde_json::json!({ "mode": "void", "session": s }))
            }
            "erase" => {
                require_erase(&principal)?;
                self.svc().erase_session(id).await.map_err(from_memory)?;
                ok_json(serde_json::json!({ "mode": "erase", "erased": fp.session_id }))
            }
            "restore" => {
                let (s, restored) = self.svc().unvoid_session(id).await.map_err(from_memory)?;
                ok_json(serde_json::json!({
                    "mode": "restore",
                    "session": s,
                    "restored_atoms": restored,
                    "message": "已撤销作废——会话与被级联归档的原子均已恢复",
                }))
            }
            other => Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                format!("mode 只支持 void / erase / restore，收到 {other:?}"),
            )),
        }
    }

    /// 检索实体（用户记忆的横向透镜：人物/项目/主题/群组/地点）。
    ///
    /// 何时用：想按「某个具体的人/项目/主题」横向拉出相关记忆线索时；
    /// 或对话中出现新人物/项目，先查一下是否已有档案。
    /// 实体由蒸馏从会话中自动抽取维护——发现信息更新请写会话，不要要求直接改实体。
    async fn memory_entities(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<EntitiesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let hits = engram_search::search_entities(
            &self.state.pool,
            &params.0.q,
            params.0.limit.unwrap_or(20),
        )
        .await
        .map_err(|e| mcp_err(ErrorCode::INTERNAL_ERROR, e.to_string()))?;
        ok_json(serde_json::to_value(&hits).unwrap_or(serde_json::json!([])))
    }

    // ---------- 项目记忆工具（跨会话工作线：项目 / 位置 / 分类文档） ----------

    /// 列出项目类型模板（建项目选类型用）。
    ///
    /// 何时用：project_create 前不知道给什么 type 时。返回 dev（开发，预置 后端/前端/测试/规划）
    /// 与 research（调研，预置 待查/线索/资料/结论/疑点/证伪）两类及其默认分类。
    async fn project_types(
        &self,
        ctx: RequestContext<RoleServer>,
        _params: Parameters<ProjectTypesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        ok_json(
            serde_json::to_value(engram_core::project::ProjectService::list_types())
                .unwrap_or(serde_json::json!([])),
        )
    }

    /// 列出项目（可按类型过滤）。
    ///
    /// 何时用：开工前找「这件事」的项目锚点，或确认某个项目名是否已存在。
    /// 返回 id / name / type / status / description / categories，按创建时间倒序。
    async fn project_list(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectListParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let rows = self
            .svc_project()
            .list_projects(params.0.type_.as_deref())
            .await
            .map_err(from_project)?;
        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
    }

    /// 项目详情：本体 + 多主机位置 + 文档索引。
    ///
    /// 何时用：开工拉上下文——目标（description）、代码在哪（locations）、
    /// 有哪些文档（docs 索引：id/分类/标题/正文字符数）一次拿全。
    /// 默认索引模式不带正文（文档多时省 token 也无信息损失）：
    /// 用 project_doc_search 定位关键词行号，project_doc_get 区间精读；
    /// 小项目想一次全量就 include_content=true。可用 project_id 或 project_name（唯一）定位。
    async fn project_get(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectGetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let gp = params.0;
        let id = self
            .resolve_project(&gp.project_id, &gp.project_name)
            .await?;
        let detail = self
            .svc_project()
            .get_project(id)
            .await
            .map_err(from_project)?;
        let mut v = serde_json::to_value(&detail).unwrap_or(serde_json::json!({}));
        if let Some(category) = &gp.category
            && let Some(docs) = v["docs"].as_array_mut()
        {
            docs.retain(|d| d["category"] == json!(category));
        }
        let include_content = gp.include_content.unwrap_or(false);
        if let Some(docs) = v["docs"].as_array_mut() {
            for (d, orig) in docs.iter_mut().zip(&detail.docs) {
                let chars = orig.content.chars().count() as i64;
                let obj = d.as_object_mut().expect("doc 是对象");
                if !include_content {
                    obj.remove("content");
                }
                obj.insert("content_chars".into(), json!(chars));
            }
        }
        ok_json(v)
    }

    /// 新建项目（type 决定初始分类，之后可增删）。
    ///
    /// 何时用：接到一件一次干不完、要跨会话推进的工作，先建项目做锚点，后续进展沉淀为文档。
    /// 项目名唯一，撞名会报错——先 project_list 确认没有可复用的同名项目。
    async fn project_create(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectCreateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let cp = params.0;
        let created = self
            .svc_project()
            .create_project(&cp.name, &cp.type_, cp.description.as_deref())
            .await
            .map_err(from_project)?;
        ok_json(serde_json::to_value(&created).unwrap_or(serde_json::json!({})))
    }

    /// 编辑项目（改名 / 状态 / 描述 / 分类列表；都是可选补丁式，不传不改）。
    ///
    /// 何时用：收尾改状态（active/paused/done/abandoned）、追加新分类、补描述。
    /// categories 是替换式——追加分类请把现有分类带上（先 project_get 看 categories）。
    async fn project_update(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectUpdateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let up = params.0;
        let id = self
            .resolve_project(&up.project_id, &up.project_name)
            .await?;
        // 补丁式：先取现值，未传字段保持原样
        let current = self
            .svc_project()
            .get_project(id)
            .await
            .map_err(from_project)?;
        let name = up.new_name.unwrap_or(current.name);
        let status = up.status.unwrap_or(current.status);
        let description = up.description.or(current.description);
        let categories = up.categories.unwrap_or(current.categories);
        let updated = self
            .svc_project()
            .update_project(id, &name, &status, description.as_deref(), &categories)
            .await
            .map_err(from_project)?;
        ok_json(serde_json::to_value(&updated).unwrap_or(serde_json::json!({})))
    }

    /// 删除项目（级联删除其位置与文档，不可逆）。
    ///
    /// 何时用：项目彻底作废时。只对用户明确表达「删掉这个项目」的请求使用。
    async fn project_delete(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectDeleteParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let id = Uuid::parse_str(&params.0.project_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "project_id 不是合法 UUID"))?;
        self.svc_project()
            .delete_project(id)
            .await
            .map_err(from_project)?;
        ok_json(serde_json::json!({ "deleted": params.0.project_id }))
    }

    /// 批量删除项目（返回删除条数与不存在的 id）。
    ///
    /// 何时用：一次清理多个作废项目。不可逆——删除前最好和用户确认过清单。
    async fn project_batch_delete(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectBatchDeleteParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let mut ids = Vec::with_capacity(params.0.ids.len());
        for raw in &params.0.ids {
            ids.push(Uuid::parse_str(raw).map_err(|_| {
                mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    format!("ids 里有非法 UUID：{raw}"),
                )
            })?);
        }
        let (deleted, failed) = self
            .svc_project()
            .batch_delete_projects(&ids)
            .await
            .map_err(from_project)?;
        ok_json(serde_json::json!({ "deleted": deleted, "failed": failed }))
    }

    /// 登记项目位置（多主机：ip / host / os / path / 用途）。
    ///
    /// 何时用：项目代码在某个主机上有了新副本/部署时登记一条。纯元数据登记制，
    /// 服务端不会读取该路径。
    async fn project_location_add(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectLocationAddParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let lp = params.0;
        let id = self
            .resolve_project(&lp.project_id, &lp.project_name)
            .await?;
        let loc = self
            .svc_project()
            .add_location(
                id,
                &lp.ip,
                &lp.host,
                &lp.os,
                &lp.path,
                lp.purpose.as_deref(),
            )
            .await
            .map_err(from_project)?;
        ok_json(serde_json::to_value(&loc).unwrap_or(serde_json::json!({})))
    }

    /// 编辑项目位置（补丁式，不传不改）。
    ///
    /// 何时用：代码挪了目录、换了机器，更新已登记的位置。
    async fn project_location_update(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectLocationUpdateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let lp = params.0;
        let id = Uuid::parse_str(&lp.location_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "location_id 不是合法 UUID"))?;
        let current = self
            .svc_project()
            .get_location(id)
            .await
            .map_err(from_project)?;
        let loc = self
            .svc_project()
            .update_location(
                id,
                &lp.ip.unwrap_or(current.ip),
                &lp.host.unwrap_or(current.host),
                &lp.os.unwrap_or(current.os),
                &lp.path.unwrap_or(current.path),
                lp.purpose.or(current.purpose).as_deref(),
            )
            .await
            .map_err(from_project)?;
        ok_json(serde_json::to_value(&loc).unwrap_or(serde_json::json!({})))
    }

    /// 删除一条项目位置登记（不动项目本体）。
    ///
    /// 何时用：某个主机上的副本不再属于这个项目。
    async fn project_location_delete(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectLocationDeleteParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let id = Uuid::parse_str(&params.0.location_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "location_id 不是合法 UUID"))?;
        self.svc_project()
            .delete_location(id)
            .await
            .map_err(from_project)?;
        ok_json(serde_json::json!({ "deleted": params.0.location_id }))
    }

    /// 项目下新增分类文档（Markdown）。
    ///
    /// 何时用：沉淀进展/结论/决策——干活中的发现写成文档，收尾写总结。
    /// category 必须是项目已有分类（先 project_get 看 categories）；同项目同分类下标题唯一。
    async fn project_doc_add(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectDocAddParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let dp = params.0;
        let id = self
            .resolve_project(&dp.project_id, &dp.project_name)
            .await?;
        // 分类归属由 service 校验（防孤儿分类，报错列出现有分类）
        let doc = self
            .svc_project()
            .add_doc(
                id,
                &dp.category,
                dp.folder.as_deref().unwrap_or(""),
                &dp.title,
                &dp.content,
            )
            .await
            .map_err(from_project)?;
        // P0-1：刚发送的正文不回显
        ok_json(slim_doc(
            serde_json::to_value(&doc).unwrap_or(serde_json::json!({})),
        ))
    }

    /// 读取项目文档：全文或按行区间精读（1-based，含两端）。
    ///
    /// 何时用：project_get 索引或 project_doc_search 命中之后精确读内容。
    /// 传 start_line/end_line 只读该区间（输出恒带「行号: 」前缀，便于连环寻址）；
    /// 不传读全文（无损，不截断），with_line_numbers=true 可给全文加行号。
    /// 行号基于文档当前版本——改文档后需重取。
    async fn project_doc_get(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectDocGetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let dp = params.0;
        let id = Uuid::parse_str(&dp.doc_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "doc_id 不是合法 UUID"))?;
        let ranged = dp.start_line.is_some() || dp.end_line.is_some();
        let numbered = |line: i64, text: &str| format!("{line}: {text}");
        if ranged {
            let (total, lines) = self
                .svc_project()
                .read_doc_lines(id, dp.start_line, dp.end_line)
                .await
                .map_err(from_project)?;
            let start = dp.start_line.unwrap_or(1);
            let end = dp.end_line.unwrap_or(total).min(total);
            let content = lines
                .iter()
                .map(|(l, t)| numbered(*l, t))
                .collect::<Vec<_>>()
                .join("\n");
            return ok_json(json!({
                "doc_id": dp.doc_id,
                "total_lines": total,
                "start_line": start,
                "end_line": end,
                "content": content,
            }));
        }
        let doc = self.svc_project().get_doc(id).await.map_err(from_project)?;
        let total = doc.content.lines().count() as i64;
        let mut v = serde_json::to_value(&doc).unwrap_or(serde_json::json!({}));
        if let Some(obj) = v.as_object_mut() {
            obj.insert("total_lines".into(), json!(total));
            if dp.with_line_numbers.unwrap_or(false) {
                let numbered_content = doc
                    .content
                    .lines()
                    .enumerate()
                    .map(|(i, t)| numbered(i as i64 + 1, t))
                    .collect::<Vec<_>>()
                    .join("\n");
                obj.insert("content".into(), json!(numbered_content));
            }
        }
        ok_json(v)
    }

    /// grep 式跨文档按行检索项目文档（大小写不敏感子串）。
    ///
    /// 何时用：索引模式下想找「某句话/某个结论在哪篇文档哪一行」。
    /// 返回命中 {doc_id, title, category, line, text}；拿到行号后用
    /// project_doc_get 的 start_line/end_line 区间精读上下文。
    async fn project_doc_search(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectDocSearchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let sp = params.0;
        let id = self
            .resolve_project(&sp.project_id, &sp.project_name)
            .await?;
        let hits = self
            .svc_project()
            .search_doc_lines(id, &sp.query, sp.limit.unwrap_or(50))
            .await
            .map_err(from_project)?;
        ok_json(serde_json::to_value(&hits).unwrap_or(serde_json::json!([])))
    }

    /// 编辑项目文档（补丁式：只传要改的字段）。
    ///
    /// 何时用：追加进展、更新结论。content 是替换式——改长文档先 project_doc_get
    /// 取原文改好再整体传回。移分类时 category 须是项目已有分类。
    async fn project_doc_update(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectDocUpdateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let dp = params.0;
        let id = Uuid::parse_str(&dp.doc_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "doc_id 不是合法 UUID"))?;
        // 部分更新直传 Option：SQL 层 COALESCE——并发各字段互不覆盖（D2 修复）
        let doc = self
            .svc_project()
            .update_doc(
                id,
                dp.category.as_deref(),
                dp.folder.as_deref(),
                dp.title.as_deref(),
                dp.content.as_deref(),
            )
            .await
            .map_err(from_project)?;
        ok_json(slim_doc(
            serde_json::to_value(&doc).unwrap_or(serde_json::json!({})),
        ))
    }

    /// 行级补丁（R 报告 P1-10）：改长文档的一行/一段，不再取全文重发全文。
    ///
    /// 何时用：doc_search 命中行号后的小修正——replace 换一段、insert 插一段、delete 删一段。
    /// 何时不用：结构性重写还是 doc_update 整体替换省事。
    async fn project_doc_patch(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectDocPatchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let dp = params.0;
        let id = Uuid::parse_str(&dp.doc_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "doc_id 不是合法 UUID"))?;
        let doc = self
            .svc_project()
            .patch_doc(
                id,
                dp.start_line,
                dp.end_line,
                dp.mode.as_deref().unwrap_or("replace"),
                dp.content.as_deref(),
            )
            .await
            .map_err(from_project)?;
        let mut v = slim_doc(serde_json::to_value(&doc).unwrap_or(serde_json::json!({})));
        v["total_lines"] = json!(doc.content.lines().count());
        v["patched"] = json!({
            "mode": dp.mode.as_deref().unwrap_or("replace"),
            "start_line": dp.start_line,
            "end_line": dp.end_line,
        });
        v["hint"] = json!("行号基于新版本——继续补丁前先重新定位（行区间读 doc_get）");
        ok_json(v)
    }

    /// 删除项目文档（不可逆）。
    ///
    /// 何时用：文档写废或彻底过时。只对明确表达的删除请求使用。
    async fn project_doc_delete(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectDocDeleteParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let id = Uuid::parse_str(&params.0.doc_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "doc_id 不是合法 UUID"))?;
        self.svc_project()
            .delete_doc(id)
            .await
            .map_err(from_project)?;
        ok_json(serde_json::json!({ "deleted": params.0.doc_id }))
    }
    // ---------- 技能域工具（可复用指令包：SKILL.md 形态） ----------

    /// 列出技能（AI 技能库的浏览入口；q/tag/enabled 过滤，不含正文）。
    ///
    /// 何时用：开始需要某种可复用能力前，先看库里有没有现成技能；或按标签浏览技能面。
    /// 命中候选后用 skills_get 取正文照做；没有合适的再用 skills_create 沉淀新技能。
    async fn skills_list(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsListParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        let lp = params.0;
        let rows = self
            .skills_svc()
            .list_skills(lp.q.as_deref(), lp.tag.as_deref(), lp.enabled)
            .await
            .map_err(from_skills)?;
        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
    }

    /// 读取一个技能全文（正文即指令——照做即可复用该技能）。
    ///
    /// 何时用：skills_list 命中候选后，取全文执行；或用户点名某个技能时。
    /// 消费形态指南（按需选通道，不要一股脑拉全量）：
    /// ① 纯文本技能 → 本工具读正文即完事；② 需要某个附属文件（脚本/参考资料）→
    /// skills_file_get 看内容，要落盘就 HTTP 直下：curl -s -H "Authorization: Bearer $KEY"
    /// "{本服务origin}/skills/{slug}/file?path=…&raw=1" -o 文件名；
    /// ③ 需要整个文件夹（SKILL.md+scripts+references）→
    /// curl -s -H "Authorization: Bearer $KEY" "{origin}/skills/{slug}/bundle" -o s.zip && tar -xf s.zip。
    async fn skills_get(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsGetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        let s = self
            .skills_svc()
            .get_skill(&params.0.slug)
            .await
            .map_err(from_skills)?;
        // folder 形态：附属文件索引随详情下发（AI 据此用 skills_file_get 取脚本/参考资料）
        let files = self
            .skills_svc()
            .list_files(&params.0.slug)
            .await
            .map_err(from_skills)?;
        let mut v = serde_json::to_value(&s).unwrap_or(serde_json::json!({}));
        v["files"] = serde_json::to_value(&files).unwrap_or(serde_json::json!([]));
        ok_json(v)
    }

    /// 读取技能附属文件（scripts/ / references/ 等按路径寻址的文件）。
    ///
    /// 何时用：SKILL.md（skills_get 的 content）里引用了 scripts/xxx.py、
    /// references/api.md 等相对路径时——取到的是文件内容，脚本由客户端本地执行
    /// （服务端只存内容，永不代执行）。
    /// 只需要一个文件且要落盘时，HTTP 直下比逐文件调本工具更快：
    /// curl -s -H "Authorization: Bearer $KEY" "{本服务origin}/skills/{slug}/file?path=…&raw=1" -o 文件名
    /// （需要整个文件夹时用整包：curl … "{origin}/skills/{slug}/bundle" -o s.zip && tar -xf s.zip）
    async fn skills_file_get(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsFileGetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        let content = self
            .skills_svc()
            .get_file(&params.0.slug, &params.0.path)
            .await
            .map_err(from_skills)?;
        ok_json(serde_json::json!({
            "slug": params.0.slug,
            "path": params.0.path,
            "content": content,
        }))
    }

    /// 写（upsert）技能附属文件：沉淀技能时把脚本/参考资料一并入库。
    ///
    /// 何时用：skills_create 之后补充 scripts/、references/ 等文件；
    /// 同路径重复写 = 覆盖更新。SKILL.md 本体走 skills_update 的 content。
    async fn skills_file_put(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsFilePutParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        let (path, size) = self
            .skills_svc()
            .put_file(&params.0.slug, &params.0.path, &params.0.content)
            .await
            .map_err(from_skills)?;
        ok_json(serde_json::json!({
            "slug": params.0.slug,
            "path": path,
            "size": size,
        }))
    }

    // ---------- 代码图谱域工具（codegraph scope） ----------

    /// 列出代码图谱项目（注册状态、索引统计）。
    ///
    /// 何时用：想探索/查询某个代码库前，先看注册了哪些项目、哪个 ready；
    /// 没有 → codegraph_register 注册（本地路径或 git URL）再 codegraph_index。
    async fn codegraph_list(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_codegraph(&p)?;
        let rows = cg_svc(&self.state).list().await.map_err(from_cg)?;
        // 新鲜度口径（工单「索引生命周期无口径」）：HEAD 对比 last_indexed，陈旧显式提示
        let mut out = Vec::new();
        for r in &rows {
            let mut item = serde_json::to_value(r).unwrap_or(serde_json::json!({}));
            item["freshness"] = cg_svc(&self.state).freshness_for(r).await;
            out.push(item);
        }
        ok_json(serde_json::Value::Array(out))
    }

    /// 注册代码图谱项目（本地绝对路径或 git URL）。
    ///
    /// 何时用：想让 AI 理解某个代码库的结构与调用关系时。注册后须 codegraph_index
    /// 建索引（异步 job，稍等片刻再 codegraph_list 确认 ready）。
    async fn codegraph_register(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<CgRegisterParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_codegraph(&p)?;
        let row = cg_svc(&self.state)
            .register(&params.0.name, &params.0.source_uri)
            .await
            .map_err(from_cg)?;
        ok_json(serde_json::to_value(&row).unwrap_or(serde_json::json!({})))
    }

    /// 建索引/重建索引（异步 job——返回 job_id，稍后 codegraph_list 看状态）。
    ///
    /// 何时用：注册后首次建索引；或代码大改后需要重建。小改动用 codegraph_sync 即可。
    async fn codegraph_index(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<CgNameParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_codegraph(&p)?;
        let id = cg_resolve(&self.state, &params.0.project).await?;
        let job = engram_jobs::JobQueue::new(self.state.pool.clone())
            .enqueue(
                engram_jobs::JobTemplate::new("cg_index")
                    .with_payload(serde_json::json!({"project_id": id})),
            )
            .await
            .map_err(|e| mcp_err(ErrorCode::INTERNAL_ERROR, format!("入队失败: {e}")))?;
        ok_json(serde_json::json!({
            "project": params.0.project, "job_id": job.id, "status": "queued",
            "hint": "索引异步执行（首次可能数分钟）——稍后 codegraph_list 确认 ready。\
                     ready 后 files/symbols 为 0 通常说明仓库没有可识别的源码文件（纯 README/文档仓库索引不出符号，属正常行为）",
        }))
    }

    /// 增量同步索引（代码小改动后刷新；异步 job）。
    ///
    /// 何时用：项目 ready 后代码有小改动，不想全量重建时。
    async fn codegraph_sync(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<CgNameParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_codegraph(&p)?;
        let id = cg_resolve(&self.state, &params.0.project).await?;
        let job = engram_jobs::JobQueue::new(self.state.pool.clone())
            .enqueue(
                engram_jobs::JobTemplate::new("cg_sync")
                    .with_payload(serde_json::json!({"project_id": id})),
            )
            .await
            .map_err(|e| mcp_err(ErrorCode::INTERNAL_ERROR, format!("入队失败: {e}")))?;
        ok_json(serde_json::json!({
            "project": params.0.project, "job_id": job.id, "status": "queued",
        }))
    }

    /// 代码图谱查询：search 符号 / explore 区域 / node 符号详情 /
    /// callers / callees / impact 影响面。
    ///
    /// 何时用：读陌生代码前先 search/explore；改代码前用 callers/impact 评估影响面；
    /// 深入一个函数用 node。
    async fn codegraph_query(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<CgQueryParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_codegraph(&p)?;
        if params.0.target.trim().is_empty() {
            return Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                "target 不能为空——先用 kind=search 搜符号，再对具体符号做 callers/impact",
            ));
        }
        let kind = engram_cg_bridge::QueryKind::from_str_opt(&params.0.kind).ok_or_else(|| {
            mcp_err(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "未知查询类型 {}——explore/search/node/callers/callees/impact",
                    params.0.kind
                ),
            )
        })?;
        let id = cg_resolve(&self.state, &params.0.project).await?;
        let mut v = cg_svc(&self.state)
            .query(
                id,
                kind,
                &params.0.target,
                params.0.depth,
                params.0.include_source.unwrap_or(false),
            )
            .await
            .map_err(from_cg)?;
        // 新鲜度提示：索引落后于 HEAD 时显式提醒（避免静默使用旧图）。
        // 仅 object 响应注入（kind=search 返回数组，不能带键——新鲜度看 codegraph list）
        if v.is_object()
            && let Ok(proj) = cg_svc(&self.state).get(id).await
            && proj.status == "ready"
        {
            v["_freshness"] = cg_svc(&self.state).freshness_for(&proj).await;
        }
        ok_json(v)
    }

    /// 注销代码图谱项目（删除注册与索引；不可逆——本地路径项目的源码不动）。
    ///
    /// 何时用：项目已完结/注册错了。按 name 或 id 注销。
    async fn codegraph_delete(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<CgNameParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_codegraph(&p)?;
        let id = cg_resolve(&self.state, &params.0.project).await?;
        let workdir_removed = cg_svc(&self.state).delete(id).await.map_err(from_cg)?;
        ok_json(serde_json::json!({
            "deleted": params.0.project,
            "workdir_removed": workdir_removed,
            "note": "git clone 的工作目录已一并删除；本地路径项目仅移除注册，源码未动",
        }))
    }

    /// 沉淀新技能：把本次对话中验证有效的做法固化成可复用指令包。
    ///
    /// 何时用：用户说「把这个做法存成技能/记成 SOP」，或一套流程已被验证有效且可复用时。
    /// 何时不用：一次性的操作细节不值得建技能；用户个人事实走记忆域（memory_write_session）。
    async fn skills_create(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsCreateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        let cp = params.0;
        let s = self
            .skills_svc()
            .create_skill(engram_core::skills::NewSkill {
                slug: cp.slug.as_deref(),
                name: &cp.name,
                description: cp.description.as_deref().unwrap_or(""),
                content: &cp.content,
                tags: &cp.tags.unwrap_or_default(),
                enabled: true,
                source: "mcp",
                kind: cp.kind.as_deref().unwrap_or("text"),
                origin: cp.origin.as_deref().unwrap_or("self"),
                local_path: cp.local_path.as_deref(),
                repo_url: cp.repo_url.as_deref(),
            })
            .await
            .map_err(from_skills)?;
        ok_json(slim_skill(
            serde_json::to_value(&s).unwrap_or(serde_json::json!({})),
        ))
    }

    /// 更新技能（正文/名称/描述/标签/启停；语义变更自动留版本快照，可回滚）。
    ///
    /// 何时用：技能做法需要修正或演进时（如用户指出了更好的步骤）。
    /// 注意：改坏可回滚（版本快照），但删除不可逆——拿不准就更新而不是删除。
    async fn skills_update(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsUpdateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        let up = params.0;
        let s = self
            .skills_svc()
            .update_skill(
                &up.slug,
                engram_core::skills::SkillPatch {
                    name: up.name,
                    description: up.description,
                    content: up.content,
                    tags: up.tags,
                    enabled: up.enabled,
                    origin: up.origin,
                    repo_url: up.repo_url,
                    local_path: up.local_path,
                },
            )
            .await
            .map_err(from_skills)?;
        ok_json(slim_skill(
            serde_json::to_value(&s).unwrap_or(serde_json::json!({})),
        ))
    }

    /// 版本列表（R 报告建议 #5）：语义变更自动留的快照，MCP 此前不可见。
    ///
    /// 何时用：改坏前先看有哪些版本；或挑 revision_id 给 restore 回滚。
    async fn skills_versions(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsVersionsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        let revs = self
            .skills_svc()
            .list_revisions(&params.0.slug)
            .await
            .map_err(from_skills)?;
        // 列表不带正文全文（content_chars 决策用）；回滚走 restore（revision_id）
        let rows: Vec<serde_json::Value> = revs
            .iter()
            .map(|r| {
                json!({
                    "id": r.id, "rev": r.rev, "name": r.name,
                    "description": r.description, "tags": r.tags,
                    "origin": r.origin, "created_at": r.created_at,
                    "content_chars": r.content.chars().count(),
                })
            })
            .collect();
        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
    }

    /// 回滚到历史版本（回滚本身也留快照，可再滚回）。
    ///
    /// 何时用：一次 update 改坏后恢复。revision_id 来自 versions 列表（id 字段）。
    async fn skills_restore(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsRestoreParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        let rp = params.0;
        let rid = Uuid::parse_str(&rp.revision_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "revision_id 不是合法 UUID"))?;
        let s = self
            .skills_svc()
            .restore_revision(&rp.slug, rid)
            .await
            .map_err(from_skills)?;
        let mut v = slim_skill(serde_json::to_value(&s).unwrap_or(serde_json::json!({})));
        v["restored_from"] = json!(rp.revision_id);
        v["hint"] = json!("已回滚（本次回滚前状态留了 restore 快照，可再滚回）");
        ok_json(v)
    }

    /// 删除技能（级联删版本快照，不可逆）。
    ///
    /// 何时用：仅当用户明确要求删除某个技能时。不要因「内容过时」自行删除——用 skills_update 修订。
    async fn skills_delete(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsDeleteParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        self.skills_svc()
            .delete_skill(&params.0.slug)
            .await
            .map_err(from_skills)?;
        ok_json(serde_json::json!({ "deleted": params.0.slug }))
    }

    /// 导入一个 SKILL.md（frontmatter 容错解析——迁移现有技能库零改写）。
    ///
    /// 何时用：用户给了现成的 SKILL.md 文件/文本要入库时。逐条导入用本工具，
    /// 批量走 HTTP API POST /skills/import（逐条报告，互不阻断）。
    async fn skills_import(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsImportParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        let ip = params.0;
        let (meta, body) = engram_core::skills::parse_frontmatter(&ip.content);
        let name = meta
            .name
            .or(ip.name)
            .or(ip.filename.clone())
            .unwrap_or_default();
        let name = name
            .trim_end_matches(".md")
            .trim_end_matches(".markdown")
            .to_string();
        if name.is_empty() {
            return Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                "无法确定技能名——请在 frontmatter 写 name，或传 filename/name 兜底",
            ));
        }
        // slug：frontmatter > 名字推导（中文名推导失败 → 显式报错，不让坏 slug 落库）
        let slug = match meta.slug.or_else(|| engram_core::skills::slugify(&name)) {
            Some(s) => s,
            None => {
                return Err(mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    format!("无法从技能名「{name}」推导 slug——请显式给 slug（kebab-case）"),
                ));
            }
        };
        let overwrite = ip.overwrite.unwrap_or(false);
        let result = if self.skills_svc().get_skill(&slug).await.is_ok() && overwrite {
            self.skills_svc()
                .update_skill(
                    &slug,
                    engram_core::skills::SkillPatch {
                        name: Some(name),
                        description: Some(meta.description.unwrap_or_default()),
                        content: Some(body),
                        tags: Some(meta.tags),
                        enabled: None,
                        origin: None,
                        repo_url: None,
                        local_path: None,
                    },
                )
                .await
                .map(|s| ("updated", s))
        } else {
            self.skills_svc()
                .create_skill(engram_core::skills::NewSkill {
                    slug: Some(&slug),
                    name: &name,
                    description: &meta.description.unwrap_or_default(),
                    content: &body,
                    tags: &meta.tags,
                    enabled: true,
                    source: "mcp",
                    kind: "text",
                    origin: "self",
                    local_path: None,
                    repo_url: None,
                })
                .await
                .map(|s| ("imported", s))
        };
        let (status, s) = result.map_err(from_skills)?;
        ok_json(serde_json::json!({
            "status": status,
            "skill": slim_skill(serde_json::to_value(&s).unwrap_or(serde_json::json!({}))),
        }))
    }

    // ---------- Wiki 域（wiki scope；实现细节见 mcp_wiki.rs） ----------

    /// Wiki 检索（FTS + 向量 RRF 融合，带 wiki 方向意图 purpose）。
    ///
    /// 何时用：需要查证「世界知识」（用户 Wiki 里沉淀的文档、概念、实体、问答）时。
    /// 何时不用：回忆「用户本人」的偏好/事实/经历用 memory_search——那是用户记忆域。
    /// 返回 {purpose, pages}：命中只带片段（命中词附近 ~160 字符）+ content_chars，
    /// 全文按需 wiki_get_page——检索可能拖回数万字符全文是 R 报告点名的上下文黑洞（P0-2）。
    async fn wiki_search(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiSearchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let wp = params.0;
        let lib = self.resolve_wiki_lib(wp.library.as_deref()).await?;
        let result = wiki::svc(&self.state)
            .search_with_purpose(lib, &wp.query, wp.max_items.unwrap_or(20))
            .await
            .map_err(wiki::from_wiki)?;
        let mut v = result;
        if let Some(pages) = v["pages"].as_array_mut() {
            *pages = pages
                .iter()
                .cloned()
                .map(|pg| wiki::snippet_page(pg, &wp.query))
                .collect();
        }
        v["hint"] = json!("命中只带片段——读全文用 get_page（slug 在每条命中里）");
        ok_json(v)
    }

    /// 浏览 Wiki 页面列表（可按页型过滤；列表不带正文）。
    ///
    /// 何时用：想系统性看看 Wiki 里有什么（而非定向检索）时；读全文用 wiki_get_page。
    async fn wiki_list_pages(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiListPagesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lp = params.0;
        let lib = self.resolve_wiki_lib(lp.library.as_deref()).await?;
        let pages = wiki::svc(&self.state)
            .list_pages(
                lib,
                lp.page_type.as_deref(),
                lp.limit.unwrap_or(100),
                lp.cursor.as_deref(),
            )
            .await
            .map_err(wiki::from_wiki)?;
        let values: Vec<serde_json::Value> = serde_json::to_value(&pages)
            .unwrap_or(serde_json::json!([]))
            .as_array()
            .map(|a| a.iter().cloned().map(wiki::trim_page).collect())
            .unwrap_or_default();
        ok_json(serde_json::to_value(&values).unwrap_or(serde_json::json!([])))
    }

    /// 读取一个 Wiki 页面全文（含 frontmatter 与版本）。
    ///
    /// 何时用：wiki_search / wiki_list_pages 定位到页面后需要读全文时。
    async fn wiki_get_page(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiGetPageParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib(params.0.library.as_deref()).await?;
        let page = wiki::svc(&self.state)
            .get_page(lib, &params.0.slug)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::to_value(&page).unwrap_or(serde_json::json!({})))
    }

    /// 写入 / 更新一个 Wiki 页面（AI 通道，frontmatter.via 落 "ai" 标记）。
    ///
    /// 何时用：用户明确要求「把……记到 Wiki / 写个页面」时，把结构化的知识沉淀成页面
    /// （Markdown + [[wikilink]]）。已存在同 slug 页面则整体覆盖更新（版本 +1）。
    /// 注意：这是覆盖式写入——更新既有页面前先用 wiki_get_page 读原文，别盲目覆盖；
    /// 临时性的问答结论更适合 wiki_archive_query 而非手搓页面。
    async fn wiki_write_page(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiWritePageParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let wp = params.0;
        let lib = self.resolve_wiki_lib(wp.library.as_deref()).await?;
        let page = wiki::svc(&self.state)
            .put_page(
                lib,
                &wp.slug,
                &wp.title,
                &wp.content,
                wp.folder.as_deref(),
                Some("ai"),
            )
            .await
            .map_err(wiki::from_wiki)?;
        // P0-1：刚发送的正文不回显；版本历史在覆盖时自动留快照
        let mut v = wiki::trim_page(serde_json::to_value(&page).unwrap_or(serde_json::json!({})));
        v["content_omitted"] = json!(true);
        // 写入即处理（2026-09-13）：AI 写页自动入队轻量再摄取（页面当原料吸收概念/互链；
        // 同 sha 去重；摄取失败不影响页面本身，失败会落 reviews via=ingest_failed 可见）
        match wiki::svc(&self.state)
            .auto_ingest_page(lib, &wp.title, &wp.content)
            .await
        {
            Ok(ingest) => v["auto_ingest"] = ingest,
            Err(e) => {
                v["auto_ingest"] = json!({"state": "failed", "hint": format!("织入入队失败（页面本身已保存）: {e}")});
            }
        }
        ok_json(v)
    }

    /// 把一段源文本织入 Wiki（异步：入队 LLM 流水线，自动抽取实体/概念并互链）。
    ///
    /// 何时用：有一篇完整文档 / 长文本值得沉淀进知识库时。内容相同（sha 命中）会跳过。
    /// 注意：织入是异步任务（前端「任务」页可见），立即返回 skipped 只代表入队/去重结果；
    /// 单条问答式的结论用 wiki_archive_query 更合适。
    async fn wiki_ingest(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiIngestParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let wp = params.0;
        let lib = self.resolve_wiki_lib(wp.library.as_deref()).await?;
        let outcome = wiki::svc(&self.state)
            .ingest(lib, &wp.title, &wp.text)
            .await
            .map_err(wiki::from_wiki)?;
        // D27 三态：已就绪（跳过）/ 在途（勿重提也非丢失）/ 新入队——此前三者不可分，
        // sha 去重封锁重试，在途窗口任务表现如「丢失」
        use engram_core::wiki::IngestOutcome;
        let job_id = outcome.job_id();
        let (skipped, status, message) = match &outcome {
            IngestOutcome::AlreadyReady(_) => (
                true,
                "ready",
                "内容已存在（sha 命中），本次跳过".to_string(),
            ),
            IngestOutcome::InFlight(_, _) => (
                false,
                "in_flight",
                "同内容任务正在处理中——无需重复提交（sha 去重会挡住），稍后可在 Wiki 页面看到产物"
                    .to_string(),
            ),
            IngestOutcome::Enqueued(_, _) => (
                false,
                "enqueued",
                "已入队织入任务——LLM 流水线异步处理，稍后可在 Wiki 页面看到产物".to_string(),
            ),
        };
        let mut out = serde_json::json!({
            "skipped": skipped,
            "status": status,
            "source_id": outcome.source_id(),
            "async": true,
            "message": message,
        });
        // R8 观察 3：进度通道落到具体 id——GET /jobs/{job_id}（任意 scope 的 key 可读）
        if let Some(j) = job_id {
            out["job_id"] = serde_json::json!(j);
            out["message"] =
                serde_json::json!(format!("{message}；进度：GET /jobs/{j}（或任务页）"));
        }
        ok_json(out)
    }

    /// 把一条问答（问 + 答）存档为 queries 页并自动再摄取。
    ///
    /// 何时用：一次检索/讨论得出值得长期保留的结论时，落成「查询」页沉淀。
    /// 同标题已存档 → 幂等跳过（skipped=true），不会重复烧 LLM。
    async fn wiki_archive_query(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiArchiveQueryParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let qp = params.0;
        let lib = self.resolve_wiki_lib(qp.library.as_deref()).await?;
        let skipped = wiki::svc(&self.state)
            .archive_query(lib, &qp.title, &qp.question, &qp.answer)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::json!({
            "skipped": skipped,
            "async": true,
            "message": if skipped {
                "同标题已存档，本次跳过"
            } else {
                "已落 queries 页并入队再摄取"
            }
        }))
    }

    /// Wiki 链接图全貌（节点 = 页面，边 = [[wikilink]]，含社区划分）。
    ///
    /// 何时用：写页面前了解现有结构、找相关页面、看知识网络长什么样。
    /// 页面很多时输出较大——粗看结构够用，定位具体页面用 wiki_search。
    async fn wiki_graph(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(libp): Parameters<wiki::WikiLibParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib(libp.library.as_deref()).await?;
        let graph = wiki::svc(&self.state)
            .graph(lib)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::to_value(&graph).unwrap_or(serde_json::json!({})))
    }

    /// Wiki 健康检查（死链、孤页、缺源等 lint 报告）。
    ///
    /// 何时用：怀疑 Wiki 结构有问题（失效双链、孤立页面）时体检；只报告不修改。
    async fn wiki_lint(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(libp): Parameters<wiki::WikiLibParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib(libp.library.as_deref()).await?;
        let report = wiki::svc(&self.state)
            .lint(lib)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::to_value(&report).unwrap_or(serde_json::json!({})))
    }

    /// 语义 lint（LLM 深度检查：页面间矛盾 / 过时声明 / 重要概念缺页）。
    ///
    /// 何时用：结构 lint（lint action）干净后的进阶健康检查——语义维度只有 LLM 能做。
    /// 异步任务：入队返回 job_id，产出写入人审队列（控制台 ReviewQueue 处理），
    /// 不自动改写页面。slugs 可限定范围控制 LLM 成本。
    async fn wiki_lint_deep(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(dp): Parameters<wiki::WikiLintDeepParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib(dp.library.as_deref()).await?;
        let job_id = wiki::svc(&self.state)
            .lint_deep_enqueue(lib, dp.slugs)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::json!({
            "job_id": job_id,
            "hint": "语义 lint 异步执行（LLM 逐批检查）——产出写入人审队列，稍后用 wiki action=reviews 查看发现、action=review_resolve 处置",
        }))
    }

    // 文档域错误映射（WikiDocumentError → rmcp）
    fn from_wiki_docs(e: engram_core::wiki_docs::WikiDocumentError) -> rmcp::ErrorData {
        use engram_core::wiki_docs::WikiDocumentError;
        match e {
            WikiDocumentError::BadRequest(m) => rmcp::ErrorData::invalid_params(m, None),
            WikiDocumentError::NotFound(m) => rmcp::ErrorData::resource_not_found(m, None),
            WikiDocumentError::Storage(m) => rmcp::ErrorData::internal_error(m, None),
        }
    }

    // ---------- 文档 RAG（wiki_documents）——MCP 对齐 HTTP 能力（工单「工具面不对齐」） ----------

    /// 入库文档（document_add）：text 或 url → 分块+嵌入进原文 RAG，并触发 LLM 织入。
    async fn wiki_document_add(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(dp): Parameters<wiki::WikiDocumentAddParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib(dp.library.as_deref()).await?;
        let svc = engram_core::wiki_docs::WikiDocumentService::new(
            self.state.pool.clone(),
            self.state.registry(),
            self.state.data_dir.clone(),
        );
        let source = if let Some(url) = dp.url.as_deref().filter(|u| !u.trim().is_empty()) {
            engram_core::wiki_docs::IngestSource::Url(url.trim().to_string())
        } else if let Some(text) = dp.text.as_deref().filter(|t| !t.trim().is_empty()) {
            engram_core::wiki_docs::IngestSource::Bytes {
                name: dp.name.clone().unwrap_or_else(|| "mcp-text".into()),
                content: text.as_bytes().to_vec(),
                content_type: Some("text/markdown".into()),
            }
        } else {
            return Err(rmcp::ErrorData::invalid_params(
                "text 与 url 二选一".to_string(),
                None,
            ));
        };
        let (id, deduped) = svc
            .submit(lib, source)
            .await
            .map_err(Self::from_wiki_docs)?;
        let doc = svc
            .get_document(lib, id)
            .await
            .map_err(Self::from_wiki_docs)?;
        ok_json(serde_json::json!({
            "id": doc.id,
            "title": doc.title,
            "status": doc.status,
            "deduped": deduped,
            "hint": "入库成功（异步分块/嵌入/织入）——用 document_get 看 status 进度；原文检索用 documents_search",
        }))
    }

    /// 文档状态（document_get）：看处理进度（status/error）。
    async fn wiki_document_get(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(dp): Parameters<wiki::WikiDocumentGetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib(dp.library.as_deref()).await?;
        let id = uuid::Uuid::parse_str(dp.id.trim()).map_err(|_| {
            rmcp::ErrorData::invalid_params(format!("id 不是合法 UUID: {}", dp.id), None)
        })?;
        let svc = engram_core::wiki_docs::WikiDocumentService::new(
            self.state.pool.clone(),
            self.state.registry(),
            self.state.data_dir.clone(),
        );
        let doc = svc
            .get_document(lib, id)
            .await
            .map_err(Self::from_wiki_docs)?;
        ok_json(serde_json::to_value(&doc).unwrap_or(serde_json::json!({})))
    }

    /// 原文检索（documents_search）：chunk 级 FTS+向量混合——搜原文分块，与页面级 wiki search 互补。
    async fn wiki_documents_search(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(dp): Parameters<wiki::WikiDocumentsSearchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib(dp.library.as_deref()).await?;
        let svc = engram_core::wiki_docs::WikiDocumentService::new(
            self.state.pool.clone(),
            self.state.registry(),
            self.state.data_dir.clone(),
        );
        let hits = svc
            .search(lib, &dp.query, dp.limit.unwrap_or(8).clamp(1, 50))
            .await
            .map_err(Self::from_wiki_docs)?;
        ok_json(serde_json::to_value(&hits).unwrap_or(serde_json::json!([])))
    }

    /// 人审队列（reviews）：列出待审提案（lint 深检/织入期 LLM 旗标）。
    ///
    /// 何时用：lint_deep 或 ingest 产出人审项后，读取发现内容（kind/payload/来源）再决定处置。
    async fn wiki_reviews(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(libp): Parameters<wiki::WikiReviewsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib(libp.library.as_deref()).await?;
        let items = wiki::svc(&self.state)
            .reviews(lib, libp.status.as_deref())
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::json!({
            "count": items.len(),
            "items": items,
        }))
    }

    /// 处置评审项（review_resolve）：标记已处理（resolved）或驳回作废（dismiss），可附动作标签。
    async fn wiki_review_resolve(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(dp): Parameters<wiki::WikiReviewResolveParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let _ = dp.library.as_deref(); // 库参数仅保留对称性——处置按全局 id 定位
        let id = uuid::Uuid::parse_str(&dp.id).map_err(|_| {
            rmcp::ErrorData::invalid_params(format!("评审项 id 不是合法 UUID: {}", dp.id), None)
        })?;
        wiki::svc(&self.state)
            .review_resolve(id, dp.action.as_deref(), dp.dismiss.unwrap_or(false))
            .await
            .map_err(wiki::from_wiki)?; // 不存在或已处理时 service 层返回 NotFound
        ok_json(serde_json::json!({
            "id": id,
            "status": if dp.dismiss.unwrap_or(false) { "dismissed" } else { "resolved" },
        }))
    }

    /// 内容目录（index）：按 page_type 分组的全库页面目录（slug/标题/入链数/首段摘要）。
    ///
    /// 何时用：回答「这个库里有什么」/为深入检索做导航——只读动态聚合，零 LLM 成本。
    async fn wiki_index(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(libp): Parameters<wiki::WikiLibParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib(libp.library.as_deref()).await?;
        let idx = wiki::svc(&self.state)
            .index(lib)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(idx)
    }

    /// 问答/分析产物归档（karpathy LLM Wiki：好答案不该消失在聊天记录里）。
    ///
    /// 何时用：一段有价值的分析/对比/结论值得长期沉淀时——以 analysis 类型（0040）
    /// 落页（复用版本快照），related 列表自动建双向 wikilinks 融入链接图。
    async fn wiki_archive(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(ap): Parameters<wiki::WikiArchiveParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib(ap.library.as_deref()).await?;
        let related = ap.related.unwrap_or_default();
        let page = wiki::svc(&self.state)
            .archive_answer(lib, &ap.slug, &ap.title, &ap.content, &related)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::to_value(&page).unwrap_or(serde_json::json!({})))
    }

    /// 删除 Wiki 页面（不可逆——连带清理双向 wikilinks；最后状态留版本快照可重建）。
    ///
    /// 何时用：页面作废/测试数据清理。只对明确表达的删除请求使用。
    async fn wiki_delete_page(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiDeletePageParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib(params.0.library.as_deref()).await?;
        let deleted = wiki::svc(&self.state)
            .delete_page(lib, &params.0.slug)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::json!({
            "deleted": params.0.slug, "ok": deleted,
            "message": "已删除（最后状态留有版本快照——误删可 restore_version 重建）",
        }))
    }

    /// 页面版本列表（新→旧；含已删除页的最后状态快照）。
    ///
    /// 何时用：覆盖更新前看看历史；或找某个版本的 version 号/预览正文。
    async fn wiki_versions(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiVersionsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib(params.0.library.as_deref()).await?;
        let rows = wiki::svc(&self.state)
            .page_versions(lib, &params.0.slug)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
    }

    /// 读取某版本快照的正文（回滚前预览用）。
    async fn wiki_version_content(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiVersionContentParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib(params.0.library.as_deref()).await?;
        let content = wiki::svc(&self.state)
            .page_version_content(lib, &params.0.slug, params.0.version)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::json!({
            "slug": params.0.slug,
            "version": params.0.version,
            "content": content,
        }))
    }

    /// 回滚页面到历史版本（回滚本身产生新版本，历史不丢；已删除的页面从快照重建）。
    ///
    /// 何时用：一次覆盖改坏内容时；或误删页面要找回。
    async fn wiki_restore_version(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiRestoreVersionParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib(params.0.library.as_deref()).await?;
        let page = wiki::svc(&self.state)
            .restore_page_version(lib, &params.0.slug, params.0.version)
            .await
            .map_err(wiki::from_wiki)?;
        let mut v = wiki::trim_page(serde_json::to_value(&page).unwrap_or(serde_json::json!({})));
        v["restored_from"] = json!(params.0.version);
        v["message"] = json!("已恢复（本次回滚前状态留了快照，可再滚回）");
        ok_json(v)
    }

    /// 列出织入原料（wiki_sources：ingest 的源文本及其状态）。
    ///
    /// 何时用：lint 报 stale_source 后找要清理的 source_id；或查某次 ingest 的状态。
    async fn wiki_sources(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(libp): Parameters<wiki::WikiLibParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib(libp.library.as_deref()).await?;
        let rows = wiki::svc(&self.state)
            .list_sources(lib)
            .await
            .map_err(wiki::from_wiki)?;
        let items: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|(id, title, status, sha)| {
                json!({"source_id": id, "title": title, "status": status, "sha256": sha})
            })
            .collect();
        ok_json(serde_json::to_value(&items).unwrap_or(serde_json::json!([])))
    }

    /// 删除一条织入原料及其全部产出（级联：源、任务、由它产出的页面；不可逆）。
    ///
    /// 何时用：lint 报 stale_source（页面已删但原料残留）或想整体撤销一次织入。
    async fn wiki_delete_source(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiDeleteSourceParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let id = Uuid::parse_str(&params.0.source_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "source_id 不是合法 UUID"))?;
        let lib = self.resolve_wiki_lib(params.0.library.as_deref()).await?;
        let report = wiki::svc(&self.state)
            .delete_source_cascade(lib, id)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(
            serde_json::to_value(&report)
                .unwrap_or(serde_json::json!({ "deleted": params.0.source_id })),
        )
    }

    /// 列出全部 wiki 库（多库；页面/原料计数一并返回）。建库/删库走 Web。
    async fn wiki_libraries(
        &self,
        ctx: RequestContext<RoleServer>,
        _params: Parameters<wiki::WikiLibrariesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let rows = engram_core::wiki::libraries::list(&self.state.pool).await;
        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
    }

    // ---------- 待办域工具（todos scope；第七域） ----------

    /// 快速记一条待办（灵感/学习计划/系统操作/问题排查——不绑定项目）。
    async fn todo_add(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TodoAddParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let tp = params.0;
        let dto = todo_svc(&self.state)
            .create(
                &tp.title,
                tp.body.as_deref().unwrap_or(""),
                tp.kind.as_deref().unwrap_or("todo"),
                // 分级合并：显式 priority 原样透传（ticket 传非空 → core 400 用 severity）；
                // 缺省按 kind 填（ticket→空串=缺省 normal；todo→normal）
                match tp.priority.as_deref() {
                    Some(p) => p,
                    None => {
                        if tp.kind.as_deref() == Some("ticket") {
                            ""
                        } else {
                            "normal"
                        }
                    }
                },
                tp.severity.as_deref(),
                tp.symptom.as_deref().unwrap_or(""),
                tp.reproduce.as_deref().unwrap_or(""),
                tp.acceptance.as_deref().unwrap_or(""),
                tp.tags.as_deref().unwrap_or(&[]),
                match tp.due_at.as_deref() {
                    // D18：显式传了 due_at 就必须可解析（此前垃圾值被静默吞成 None，
                    // 调用方以为设置了截止时间实际没生效）
                    Some(s) => Some(parse_flex_datetime(s)?),
                    None => None,
                },
                tp.project_hint.as_deref(),
            )
            .await
            .map_err(from_todo)?;
        ok_json(slim_todo(
            serde_json::to_value(&dto).unwrap_or(serde_json::json!({})),
        ))
    }

    /// 待办列表（open 优先；status/priority/tag/q 过滤）。
    async fn todo_list(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TodoListParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let lp = params.0;
        let rows = todo_svc(&self.state)
            .list(
                lp.status.as_deref(),
                lp.kind.as_deref(),
                lp.priority.as_deref(),
                lp.tag.as_deref(),
                lp.q.as_deref(),
                lp.cursor.as_deref(),
                lp.limit.unwrap_or(50),
            )
            .await
            .map_err(from_todo)?;
        // 摘要模式（工单「列表返回全量正文」）：MCP 默认 brief——只回轻字段+关联计数
        if lp.brief.unwrap_or(true) {
            let counts = todo_svc(&self.state)
                .link_count_map()
                .await
                .unwrap_or_default();
            let brief: Vec<serde_json::Value> = rows
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "id": t.id,
                        "short_no": t.short_no,
                        "ref": format!("EN-{}", t.short_no),
                        "title": t.title,
                        "kind": t.kind,
                        "status": t.status,
                        "severity": t.severity,
                        "priority": t.priority,
                        "tags": t.tags,
                        "links": counts.get(&t.id).copied().unwrap_or(0),
                        "body_omitted": true,
                    })
                })
                .collect();
            return ok_json(serde_json::json!({
                "brief": true,
                "count": brief.len(),
                "items": brief,
                "hint": "摘要模式（body 已省略）——brief=false 取全量；引用条目用 EN-<短号>",
            }));
        }
        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
    }

    /// todo 引用解析（EN-<短号> 或 UUID → id）。
    async fn todo_ref_id(&self, r: &str) -> Result<Uuid, rmcp::ErrorData> {
        let r = r.trim();
        if let Ok(id) = Uuid::parse_str(r) {
            return Ok(id);
        }
        let n = r
            .strip_prefix("EN-")
            .or_else(|| r.strip_prefix("en-"))
            .and_then(|n| n.parse::<i32>().ok())
            .ok_or_else(|| {
                mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    format!("引用格式非法：{r}（UUID 或 EN-<短号>）"),
                )
            })?;
        todo_svc(&self.state)
            .find_by_ref(r)
            .await
            .map_err(from_todo)?
            .map(|d| d.id)
            .ok_or_else(|| mcp_err(ErrorCode::INVALID_PARAMS, format!("待办不存在: EN-{n}")))
    }

    /// 建立关联（link）：blocked_by / relates_to / parent 三类，幂等。
    async fn todo_link(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TodoLinkParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let lp = params.0;
        let from = self.todo_ref_id(&lp.from).await?;
        let to = self.todo_ref_id(&lp.to).await?;
        let created = todo_svc(&self.state)
            .link(from, to, &lp.kind)
            .await
            .map_err(from_todo)?;
        ok_json(serde_json::json!({
            "from": from, "to": to, "kind": lp.kind,
            "created": created,
            "hint": if created { "已关联" } else { "关联已存在（幂等）" },
        }))
    }

    /// 解除关联（unlink）。
    async fn todo_unlink(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TodoUnlinkParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let lp = params.0;
        let from = self.todo_ref_id(&lp.from).await?;
        let to = self.todo_ref_id(&lp.to).await?;
        let removed = todo_svc(&self.state)
            .unlink(from, to, &lp.kind)
            .await
            .map_err(from_todo)?;
        if !removed {
            return Err(mcp_err(ErrorCode::INVALID_PARAMS, "关联不存在"));
        }
        ok_json(serde_json::json!({"from": from, "to": to, "kind": lp.kind, "removed": true}))
    }

    /// 关联列表（links）：双向列出某条目的全部关联（含 EN-短号与方向）——「谁阻塞我」反查入口。
    async fn todo_links(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TodoLinksParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let id = self.todo_ref_id(&params.0.id).await?;
        let raw = todo_svc(&self.state).links(id).await.map_err(from_todo)?;
        let counts = todo_svc(&self.state)
            .list(None, None, None, None, None, None, 500)
            .await
            .map_err(from_todo)?;
        let short_of: std::collections::HashMap<Uuid, i32> =
            counts.iter().map(|t| (t.id, t.short_no)).collect();
        let items: Vec<serde_json::Value> = raw
            .iter()
            .map(|(from, to, kind, dir)| {
                serde_json::json!({
                    "from": from,
                    "from_ref": short_of.get(from).map(|n| format!("EN-{n}")).unwrap_or_default(),
                    "to": to,
                    "to_ref": short_of.get(to).map(|n| format!("EN-{n}")).unwrap_or_default(),
                    "kind": kind,
                    "direction": dir,
                })
            })
            .collect();
        ok_json(serde_json::json!({"id": id, "count": items.len(), "links": items}))
    }

    /// 待办详情。
    async fn todo_get(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TodoIdParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let dto = todo_svc(&self.state)
            .find_by_ref(&params.0.id)
            .await
            .map_err(from_todo)?
            .ok_or_else(|| {
                mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    format!("待办不存在: {}", params.0.id),
                )
            })?;
        ok_json(serde_json::to_value(&dto).unwrap_or(serde_json::json!({})))
    }

    /// 标记待办完成（done_at 自动记录）。
    async fn todo_done(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TodoIdParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let id = self.todo_ref_id(&params.0.id).await?;
        let dto = todo_svc(&self.state)
            .update(
                id,
                None,
                None,
                None,
                None,
                Some("done"),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .map_err(from_todo)?;
        ok_json(slim_todo(
            serde_json::to_value(&dto).unwrap_or(serde_json::json!({})),
        ))
    }

    /// 更新待办（标题/详情/优先级/状态，部分字段 None 不动）。
    async fn todo_update(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TodoUpdateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let id = self.todo_ref_id(&params.0.id).await?;
        let dto = todo_svc(&self.state)
            .update(
                id,
                params.0.kind.as_deref(),
                params.0.title.as_deref(),
                params.0.body.as_deref(),
                params.0.priority.as_deref(),
                params.0.status.as_deref(),
                params.0.severity.as_ref().map(|o| Some(o.as_str())),
                params.0.symptom.as_deref(),
                params.0.reproduce.as_deref(),
                params.0.acceptance.as_deref(),
                params.0.resolution.as_deref(),
                match params.0.due_at.as_deref() {
                    Some(s) => Some(Some(parse_flex_datetime(s)?)),
                    None => None,
                },
                None,
                None,
            )
            .await
            .map_err(from_todo)?;
        ok_json(slim_todo(
            serde_json::to_value(&dto).unwrap_or(serde_json::json!({})),
        ))
    }

    /// 删除待办（物理删除；归档语义走 todo_update status=archived）。
    async fn todo_delete(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TodoIdParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let id = self.todo_ref_id(&params.0.id).await?;
        todo_svc(&self.state).delete(id).await.map_err(from_todo)?;
        ok_json(serde_json::json!({ "deleted": params.0.id }))
    }

    // ---------- 跨域全局检索（第七个常驻工具；不属单一 scope，按 key 实际 scope 分域执行） ----------

    /// 全局检索（R 报告 P1-8）：一次查询并发打 memory/wiki/skills/todos/projects 五域，
    /// 各返回 top-k 摘要（含命中域标注）——「6 次单域搜索」压成 1 次。
    ///
    /// 何时用：不确定信息在哪域、或要先扫一遍全库面时。
    /// 何时不用：已知域的精确检索直接用单域工具（省时省 token，且支持更多过滤参数）。
    /// 只检索本 key 有 scope 的域；命中只有摘要，全文按各域 get/read 通道按需取。
    #[tool(
        name = "search_all",
        annotations(
            title = "全局检索",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn search_all_tool(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SearchAllParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        let q = params.0.query.trim().to_string();
        if q.is_empty() {
            return Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                "query 不能为空——给出检索词，各域并发检索",
            ));
        }
        let max = params.0.max_per_domain.unwrap_or(3).clamp(1, 10);
        let scope_of = |s: &str| p.has_scope(s);
        if !["memory", "wiki", "skills", "todos", "project"]
            .iter()
            .any(|s| scope_of(s))
        {
            return Err(mcp_err(
                ErrorCode::INVALID_REQUEST,
                "本 key 没有任何可检索域的 scope——search_all 需要至少一个域的读权限",
            ));
        }

        // memory（l1/l2/实体）；检索失败（如 LLM 未配置）降级为错误标注而非整体失败
        let mem = scope_of("memory");
        let mem_fut = async {
            if !mem {
                return None;
            }
            let r = self
                .svc()
                .search(&q, &["l1", "l2", "entities"], max, false, None, None)
                .await;
            Some(match r {
                Ok(r) => json!({
                    "l1": r.l1.iter().map(|h| json!({"id": h.id, "score": h.score, "snippet": h.snippet})).collect::<Vec<_>>(),
                    "l2": r.l2.iter().map(|h| json!({"id": h.id, "title": h.title, "snippet": h.snippet})).collect::<Vec<_>>(),
                    "entities": r.entities.iter().map(|h| json!({"id": h.id, "title": h.title, "kind": h.kind})).collect::<Vec<_>>(),
                }),
                Err(e) => json!({ "error": e.to_string() }),
            })
        };
        // wiki（命中片段化，与 wiki_search 同口径）
        let wik = scope_of("wiki");
        let wiki_fut = async {
            if !wik {
                return None;
            }
            let lib = match engram_core::wiki::libraries::resolve(&self.state.pool, None).await {
                Ok(l) => l,
                Err(e) => return Some(json!({ "error": e.to_string() })),
            };
            let r = wiki::svc(&self.state).search(lib, &q, max).await;
            Some(match r {
                Ok(pages) => json!(
                    pages
                        .iter()
                        .map(|pg| {
                            wiki::snippet_page(serde_json::to_value(pg).unwrap_or(json!({})), &q)
                        })
                        .collect::<Vec<_>>()
                ),
                Err(e) => json!({ "error": e.to_string() }),
            })
        };
        // skills（名字/描述命中，标题 + 描述即可定位）
        let sk = scope_of("skills");
        let skills_fut = async {
            if !sk {
                return None;
            }
            let pattern = format!("%{q}%");
            match engram_storage::repo::skills::list_skills(
                &self.state.pool,
                Some(pattern),
                None,
                None,
            )
            .await
            {
                Ok(rows) => Some(json!(rows
                    .iter()
                    .take(max as usize)
                    .map(|s| json!({"slug": s.slug, "name": s.name, "description": s.description}))
                    .collect::<Vec<_>>())),
                Err(e) => Some(json!({ "error": e.to_string() })),
            }
        };
        // todos（标题/正文子串）
        let td = scope_of("todos");
        let todos_fut = async {
            if !td {
                return None;
            }
            match todo_svc(&self.state)
                .list(None, None, None, None, Some(&q), None, max)
                .await
            {
                Ok(rows) => Some(json!(
                    rows.iter()
                        .map(|t| json!({"id": t.id, "title": t.title, "status": t.status}))
                        .collect::<Vec<_>>()
                )),
                Err(e) => Some(json!({ "error": e.to_string() })),
            }
        };
        let (mem_r, wiki_r, skills_r, todos_r) =
            tokio::join!(mem_fut, wiki_fut, skills_fut, todos_fut);

        // projects（项目名/描述命中 + 各项目文档按行检索，总量封顶）
        let mut projects_val: Option<serde_json::Value> = None;
        if scope_of("project") {
            let mut hits: Vec<serde_json::Value> = Vec::new();
            match self.svc_project().list_projects(None).await {
                Ok(projects) => {
                    let ql = q.to_lowercase();
                    'outer: for pr in projects.iter().take(20) {
                        let desc = pr.description.as_deref().unwrap_or("").to_lowercase();
                        let name_hit = pr.name.to_lowercase().contains(&ql);
                        let desc_hit = desc.contains(&ql);
                        if (name_hit || desc_hit) && hits.len() < max as usize {
                            hits.push(json!({
                                "project": pr.name, "match": "项目名/描述",
                                "status": pr.status,
                            }));
                        }
                        if let Ok(doc_hits) =
                            self.svc_project().search_doc_lines(pr.id, &q, 2).await
                        {
                            for h in doc_hits {
                                if hits.len() >= max as usize + 2 {
                                    break 'outer;
                                }
                                hits.push(json!({
                                    "project": pr.name, "match": "文档行",
                                    "title": h.title, "line": h.line, "text": h.text,
                                }));
                            }
                        }
                    }
                    projects_val = Some(json!(hits));
                }
                Err(e) => projects_val = Some(json!({ "error": e.to_string() })),
            }
        }

        let mut out = json!({
            "query": q,
            "note": "各域 top-k 摘要——精确/过滤检索用单域工具；wiki 全文 get_page，memory 原文 get_session，技能全文 skills get",
        });
        if let Some(v) = &mem_r {
            out["memory"] = v.clone();
        }
        if let Some(v) = &wiki_r {
            out["wiki"] = v.clone();
        }
        if let Some(v) = &skills_r {
            out["skills"] = v.clone();
        }
        if let Some(v) = &todos_r {
            out["todos"] = v.clone();
        }
        if let Some(v) = &projects_val {
            out["projects"] = v.clone();
        }

        // R6：rerank=true 时五域命中合并 top-10 交 LLM 精排，附 reranked 视图（失败降级为无此字段）
        if params.0.rerank == Some(true) {
            let mut candidates: Vec<UnifiedHit> = Vec::new();
            let push_arr =
                |domain: &str, arr: &serde_json::Value, candidates: &mut Vec<UnifiedHit>| {
                    if let Some(items) = arr.as_array() {
                        for it in items {
                            let title = it
                                .get("title")
                                .or_else(|| it.get("name"))
                                .or_else(|| it.get("slug"))
                                .and_then(|x| x.as_str())
                                .map(|s| s.to_string());
                            let snippet = it
                                .get("snippet")
                                .or_else(|| it.get("description"))
                                .or_else(|| it.get("text"))
                                .and_then(|x| x.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_default();
                            if title.is_none() && snippet.is_empty() {
                                continue;
                            }
                            candidates.push(UnifiedHit {
                                domain: domain.to_string(),
                                id: uuid::Uuid::now_v7(),
                                title,
                                snippet,
                                score: 0.0,
                                extra: serde_json::json!({}),
                            });
                        }
                    }
                };
            if let Some(v) = mem_r.as_ref() {
                if let Some(l1) = v.get("l1") {
                    push_arr("memory", l1, &mut candidates);
                }
                if let Some(l2) = v.get("l2") {
                    push_arr("memory", l2, &mut candidates);
                }
            }
            if let Some(v) = wiki_r.as_ref() {
                push_arr("wiki", v, &mut candidates);
            }
            if let Some(v) = skills_r.as_ref() {
                push_arr("skills", v, &mut candidates);
            }
            if let Some(v) = todos_r.as_ref() {
                push_arr("todos", v, &mut candidates);
            }
            if let Some(v) = projects_val.as_ref() {
                push_arr("projects", v, &mut candidates);
            }

            let top = candidates.len().min(10);
            if top > 1 {
                match engram_core::unified::rerank_hits(&self.state.llm(), &q, &candidates[..top])
                    .await
                {
                    Ok(order) => {
                        let reranked: Vec<serde_json::Value> = order
                            .into_iter()
                            .filter_map(|i| candidates.get(i))
                            .map(|h| {
                                json!({
                                    "domain": h.domain,
                                    "title": h.title,
                                    "snippet": h.snippet,
                                })
                            })
                            .collect();
                        out["reranked"] = json!(reranked);
                        out["note"] = json!(
                            "各域 top-k 摘要 + reranked=LLM 精排序（跨域）；精确检索用单域工具"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "search_all rerank 失败——仅返回分组摘要");
                    }
                }
            }
        }

        ok_json(out)
    }

    // ---------- 渐进式发现：域入口工具（每域一个，域内操作按需发现） ----------
    //
    // 历史上的 53 个扁平工具全部收编为「域 + action」，action 表（摘要/破坏性/参数
    // schema）是 dispatch::action_docs 的静态表——L0 目录、help 手册、管理台三方同源。
    // scope 检查在各域实现方法内原样保留；这里只做 help 渲染与 action 分发。

    /// 用户记忆域（单一入口）。记忆四层：L0 会话 →（蒸馏）→ L1 原子 → L2 场景 → L3 画像，
    /// 实体坐标系横向串联。开场用 action="context" 装载，定向回忆用 "search"，
    /// 收尾用 "write_session" 写入；遗忘用 "forget"。
    /// 速记：remember 正文字段名是 text；strength=fact 直写原话限 120 字，默认蒸馏路径成段内容也可，更长走 write_session。操作全景：action="help"。
    #[tool(
        name = "memory",
        annotations(
            title = "用户记忆域",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn memory_tool(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(call): Parameters<dispatch::DomainCall>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        if call.action == "help" {
            let cfg = load_config(&self.state.pool).await;
            return ok_json(dispatch::render_manual("memory", &cfg.disabled_tools));
        }
        match call.action.as_str() {
            "context" => {
                self.memory_context(
                    ctx,
                    Parameters(dispatch::from_args("memory", "context", call.args)?),
                )
                .await
            }
            "search" => {
                self.memory_search(
                    ctx,
                    Parameters(dispatch::from_args("memory", "search", call.args)?),
                )
                .await
            }
            "remember" => {
                self.memory_remember(
                    ctx,
                    Parameters(dispatch::from_args("memory", "remember", call.args)?),
                )
                .await
            }
            "write_session" => {
                self.memory_write_session(
                    ctx,
                    Parameters(dispatch::from_args("memory", "write_session", call.args)?),
                )
                .await
            }
            "append_session" => {
                self.memory_append_session(
                    ctx,
                    Parameters(dispatch::from_args("memory", "append_session", call.args)?),
                )
                .await
            }
            "list_sessions" => {
                self.memory_list_sessions(
                    ctx,
                    Parameters(dispatch::from_args("memory", "list_sessions", call.args)?),
                )
                .await
            }
            "get_session" => {
                self.memory_get_session(
                    ctx,
                    Parameters(dispatch::from_args("memory", "get_session", call.args)?),
                )
                .await
            }
            "distill_result" => {
                self.memory_distill_result(
                    ctx,
                    Parameters(dispatch::from_args("memory", "distill_result", call.args)?),
                )
                .await
            }
            "kv_put" => {
                self.memory_kv_put(
                    ctx,
                    Parameters(dispatch::from_args("memory", "kv_put", call.args)?),
                )
                .await
            }
            "kv_get" => {
                self.memory_kv_get(
                    ctx,
                    Parameters(dispatch::from_args("memory", "kv_get", call.args)?),
                )
                .await
            }
            "kv_list" => {
                self.memory_kv_list(
                    ctx,
                    Parameters(dispatch::from_args("memory", "kv_list", call.args)?),
                )
                .await
            }
            "kv_search" => {
                self.memory_kv_search(
                    ctx,
                    Parameters(dispatch::from_args("memory", "kv_search", call.args)?),
                )
                .await
            }
            "list_atoms" => {
                self.memory_list_atoms(
                    ctx,
                    Parameters(dispatch::from_args("memory", "list_atoms", call.args)?),
                )
                .await
            }
            "entities" => {
                self.memory_entities(
                    ctx,
                    Parameters(dispatch::from_args("memory", "entities", call.args)?),
                )
                .await
            }
            "forget" => {
                self.memory_forget(
                    ctx,
                    Parameters(dispatch::from_args("memory", "forget", call.args)?),
                )
                .await
            }
            other => Err(dispatch::unknown_action("memory", other)),
        }
    }

    /// 项目记忆域（单一入口）：项目 = 一件有明确目标、跨会话推进的工作，
    /// 下挂多主机位置（登记制）与「分类 > 文档」树。开工 "list"/"get" 接上下文，
    /// 干活中 "doc_add"/"doc_update" 沉淀，收尾 "update" 改状态。
    /// 速记：所有 doc_* 操作需先定位项目（参数 project_id 或 project_name，先 "list"）。操作全景：action="help"。
    #[tool(
        name = "projects",
        annotations(
            title = "项目记忆域",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn projects_tool(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(call): Parameters<dispatch::DomainCall>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        if call.action == "help" {
            let cfg = load_config(&self.state.pool).await;
            return ok_json(dispatch::render_manual("projects", &cfg.disabled_tools));
        }
        match call.action.as_str() {
            "types" => {
                self.project_types(
                    ctx,
                    Parameters(dispatch::from_args("projects", "types", call.args)?),
                )
                .await
            }
            "list" => {
                self.project_list(
                    ctx,
                    Parameters(dispatch::from_args("projects", "list", call.args)?),
                )
                .await
            }
            "get" => {
                self.project_get(
                    ctx,
                    Parameters(dispatch::from_args("projects", "get", call.args)?),
                )
                .await
            }
            "create" => {
                self.project_create(
                    ctx,
                    Parameters(dispatch::from_args("projects", "create", call.args)?),
                )
                .await
            }
            "update" => {
                self.project_update(
                    ctx,
                    Parameters(dispatch::from_args("projects", "update", call.args)?),
                )
                .await
            }
            "delete" => {
                self.project_delete(
                    ctx,
                    Parameters(dispatch::from_args("projects", "delete", call.args)?),
                )
                .await
            }
            "batch_delete" => {
                self.project_batch_delete(
                    ctx,
                    Parameters(dispatch::from_args("projects", "batch_delete", call.args)?),
                )
                .await
            }
            "location_add" => {
                self.project_location_add(
                    ctx,
                    Parameters(dispatch::from_args("projects", "location_add", call.args)?),
                )
                .await
            }
            "location_update" => {
                self.project_location_update(
                    ctx,
                    Parameters(dispatch::from_args(
                        "projects",
                        "location_update",
                        call.args,
                    )?),
                )
                .await
            }
            "location_delete" => {
                self.project_location_delete(
                    ctx,
                    Parameters(dispatch::from_args(
                        "projects",
                        "location_delete",
                        call.args,
                    )?),
                )
                .await
            }
            "doc_add" => {
                self.project_doc_add(
                    ctx,
                    Parameters(dispatch::from_args("projects", "doc_add", call.args)?),
                )
                .await
            }
            "doc_get" => {
                self.project_doc_get(
                    ctx,
                    Parameters(dispatch::from_args("projects", "doc_get", call.args)?),
                )
                .await
            }
            "doc_search" => {
                self.project_doc_search(
                    ctx,
                    Parameters(dispatch::from_args("projects", "doc_search", call.args)?),
                )
                .await
            }
            "doc_update" => {
                self.project_doc_update(
                    ctx,
                    Parameters(dispatch::from_args("projects", "doc_update", call.args)?),
                )
                .await
            }
            "doc_patch" => {
                self.project_doc_patch(
                    ctx,
                    Parameters(dispatch::from_args("projects", "doc_patch", call.args)?),
                )
                .await
            }
            "doc_delete" => {
                self.project_doc_delete(
                    ctx,
                    Parameters(dispatch::from_args("projects", "doc_delete", call.args)?),
                )
                .await
            }
            other => Err(dispatch::unknown_action("projects", other)),
        }
    }

    /// 技能域（单一入口）：技能 = 可复用的指令包（SKILL.md 形态 + scripts/references 附件）。
    /// 需要某种能力前先 "list" 找现成的，命中 "get" 取全文照做；验证有效的做法
    /// 用 "create" 沉淀。操作全景：action="help"。
    #[tool(
        name = "skills",
        annotations(
            title = "技能域",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn skills_tool(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(call): Parameters<dispatch::DomainCall>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        if call.action == "help" {
            let cfg = load_config(&self.state.pool).await;
            return ok_json(dispatch::render_manual("skills", &cfg.disabled_tools));
        }
        match call.action.as_str() {
            "list" => {
                self.skills_list(
                    ctx,
                    Parameters(dispatch::from_args("skills", "list", call.args)?),
                )
                .await
            }
            "get" => {
                self.skills_get(
                    ctx,
                    Parameters(dispatch::from_args("skills", "get", call.args)?),
                )
                .await
            }
            "file_get" => {
                self.skills_file_get(
                    ctx,
                    Parameters(dispatch::from_args("skills", "file_get", call.args)?),
                )
                .await
            }
            "file_put" => {
                self.skills_file_put(
                    ctx,
                    Parameters(dispatch::from_args("skills", "file_put", call.args)?),
                )
                .await
            }
            "create" => {
                self.skills_create(
                    ctx,
                    Parameters(dispatch::from_args("skills", "create", call.args)?),
                )
                .await
            }
            "update" => {
                self.skills_update(
                    ctx,
                    Parameters(dispatch::from_args("skills", "update", call.args)?),
                )
                .await
            }
            "versions" => {
                self.skills_versions(
                    ctx,
                    Parameters(dispatch::from_args("skills", "versions", call.args)?),
                )
                .await
            }
            "restore" => {
                self.skills_restore(
                    ctx,
                    Parameters(dispatch::from_args("skills", "restore", call.args)?),
                )
                .await
            }
            "delete" => {
                self.skills_delete(
                    ctx,
                    Parameters(dispatch::from_args("skills", "delete", call.args)?),
                )
                .await
            }
            "import" => {
                self.skills_import(
                    ctx,
                    Parameters(dispatch::from_args("skills", "import", call.args)?),
                )
                .await
            }
            other => Err(dispatch::unknown_action("skills", other)),
        }
    }

    /// Wiki 域（单一入口）：世界知识库——Markdown 页面 + [[wikilink]] + 混合检索。
    /// 查证「客观知识」用 "search"；用户要求沉淀时：单条结论 "archive_query"、
    /// 整篇文档 "ingest"（异步）、明确要页面 "write_page"。操作全景：action="help"。
    #[tool(
        name = "wiki",
        annotations(
            title = "Wiki 域",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn wiki_tool(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(call): Parameters<dispatch::DomainCall>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        if call.action == "help" {
            let cfg = load_config(&self.state.pool).await;
            return ok_json(dispatch::render_manual("wiki", &cfg.disabled_tools));
        }
        match call.action.as_str() {
            "search" => {
                self.wiki_search(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "search", call.args)?),
                )
                .await
            }
            "list_pages" => {
                self.wiki_list_pages(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "list_pages", call.args)?),
                )
                .await
            }
            "get_page" => {
                self.wiki_get_page(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "get_page", call.args)?),
                )
                .await
            }
            "write_page" => {
                self.wiki_write_page(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "write_page", call.args)?),
                )
                .await
            }
            "ingest" => {
                self.wiki_ingest(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "ingest", call.args)?),
                )
                .await
            }
            "archive_query" => {
                self.wiki_archive_query(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "archive_query", call.args)?),
                )
                .await
            }
            "versions" => {
                self.wiki_versions(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "versions", call.args)?),
                )
                .await
            }
            "version_content" => {
                self.wiki_version_content(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "version_content", call.args)?),
                )
                .await
            }
            "restore_version" => {
                self.wiki_restore_version(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "restore_version", call.args)?),
                )
                .await
            }
            "sources" => {
                self.wiki_sources(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "sources", call.args)?),
                )
                .await
            }
            "delete_source" => {
                self.wiki_delete_source(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "delete_source", call.args)?),
                )
                .await
            }
            "libraries" => {
                self.wiki_libraries(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "libraries", call.args)?),
                )
                .await
            }
            "graph" => {
                self.wiki_graph(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "graph", call.args)?),
                )
                .await
            }
            "lint" => {
                self.wiki_lint(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "lint", call.args)?),
                )
                .await
            }
            "lint_deep" => {
                self.wiki_lint_deep(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "lint_deep", call.args)?),
                )
                .await
            }
            "document_add" => {
                self.wiki_document_add(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "document_add", call.args)?),
                )
                .await
            }
            "document_get" => {
                self.wiki_document_get(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "document_get", call.args)?),
                )
                .await
            }
            "documents_search" => {
                self.wiki_documents_search(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "documents_search", call.args)?),
                )
                .await
            }
            "reviews" => {
                self.wiki_reviews(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "reviews", call.args)?),
                )
                .await
            }
            "review_resolve" => {
                self.wiki_review_resolve(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "review_resolve", call.args)?),
                )
                .await
            }
            "index" => {
                self.wiki_index(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "index", call.args)?),
                )
                .await
            }
            "archive" => {
                self.wiki_archive(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "archive", call.args)?),
                )
                .await
            }
            "delete_page" => {
                self.wiki_delete_page(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "delete_page", call.args)?),
                )
                .await
            }
            other => Err(dispatch::unknown_action("wiki", other)),
        }
    }

    /// 待办域（todos scope，第七域）：快速记录与跟进不绑定项目的待办
    /// （灵感/学习计划/系统操作/问题排查）。"add" 秒记，"list" 看进行中，
    /// "done" 完成，"delete" 删。操作全景：action="help"。
    #[tool(
        name = "todos",
        annotations(
            title = "待办域",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn todos_tool(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(call): Parameters<dispatch::DomainCall>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        if call.action == "help" {
            let cfg = load_config(&self.state.pool).await;
            return ok_json(dispatch::render_manual("todos", &cfg.disabled_tools));
        }
        match call.action.as_str() {
            "add" => {
                self.todo_add(
                    ctx,
                    Parameters(dispatch::from_args("todos", "add", call.args)?),
                )
                .await
            }
            "list" => {
                self.todo_list(
                    ctx,
                    Parameters(dispatch::from_args("todos", "list", call.args)?),
                )
                .await
            }
            "get" => {
                self.todo_get(
                    ctx,
                    Parameters(dispatch::from_args("todos", "get", call.args)?),
                )
                .await
            }
            "links" => {
                self.todo_links(
                    ctx,
                    Parameters(dispatch::from_args("todos", "links", call.args)?),
                )
                .await
            }
            "link" => {
                self.todo_link(
                    ctx,
                    Parameters(dispatch::from_args("todos", "link", call.args)?),
                )
                .await
            }
            "unlink" => {
                self.todo_unlink(
                    ctx,
                    Parameters(dispatch::from_args("todos", "unlink", call.args)?),
                )
                .await
            }
            "done" => {
                self.todo_done(
                    ctx,
                    Parameters(dispatch::from_args("todos", "done", call.args)?),
                )
                .await
            }
            "update" => {
                self.todo_update(
                    ctx,
                    Parameters(dispatch::from_args("todos", "update", call.args)?),
                )
                .await
            }
            "delete" => {
                self.todo_delete(
                    ctx,
                    Parameters(dispatch::from_args("todos", "delete", call.args)?),
                )
                .await
            }
            other => Err(dispatch::unknown_action("todos", other)),
        }
    }

    /// 代码图谱域（单一入口）：注册代码库 → 建索引 → 图谱查询
    /// （search/explore/node/callers/callees/impact），读懂陌生代码库的调用关系。
    /// 操作全景：action="help"。
    #[tool(
        name = "codegraph",
        annotations(
            title = "代码图谱域",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn codegraph_tool(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(call): Parameters<dispatch::DomainCall>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_codegraph(&p)?;
        if call.action == "help" {
            let cfg = load_config(&self.state.pool).await;
            return ok_json(dispatch::render_manual("codegraph", &cfg.disabled_tools));
        }
        match call.action.as_str() {
            "list" => self.codegraph_list(ctx).await,
            "register" => {
                self.codegraph_register(
                    ctx,
                    Parameters(dispatch::from_args("codegraph", "register", call.args)?),
                )
                .await
            }
            "query" => {
                self.codegraph_query(
                    ctx,
                    Parameters(dispatch::from_args("codegraph", "query", call.args)?),
                )
                .await
            }
            "index" => {
                self.codegraph_index(
                    ctx,
                    Parameters(dispatch::from_args("codegraph", "index", call.args)?),
                )
                .await
            }
            "sync" => {
                self.codegraph_sync(
                    ctx,
                    Parameters(dispatch::from_args("codegraph", "sync", call.args)?),
                )
                .await
            }
            "delete" => {
                self.codegraph_delete(
                    ctx,
                    Parameters(dispatch::from_args("codegraph", "delete", call.args)?),
                )
                .await
            }
            other => Err(dispatch::unknown_action("codegraph", other)),
        }
    }
}

/// MCP instructions：initialize 时返回给调用方 AI 的顶层使用说明。
const SERVER_INSTRUCTIONS: &str = "\
Engram —— 单用户 AI 长期记忆平台。MCP 工具面采用渐进式发现：六个领域各一个入口工具\
（memory 用户记忆 / projects 项目记忆 / skills 技能 / wiki 知识库 / todos 待办 / codegraph 代码图谱），\
外加跨域全局检索 search_all（一次查询并发五域，各回 top-k 摘要）。\
域工具调用形态 {\"action\":\"<操作名>\", ...参数}；每个工具的描述里带操作目录（常驻可见），\
参数细节用 {\"action\":\"help\"} 一轮取回全域操作手册。

用户记忆分四层蒸馏：L0 原始会话 →（蒸馏）→ L1 原子事实 → L2 场景模式 → L3 用户画像；\
另有实体坐标系（人物/项目/主题/群组/地点）横向串联记忆。全部记忆可溯源、可遗忘（void 可 restore 撤销）。

memory 域用法：
1. 会话开始：{\"action\":\"context\"} 装载用户画像与近期记忆，再开始对话；
2. 对话中需要背景：{\"action\":\"search\",\"query\":\"…\"} 定向回忆，或 {\"action\":\"entities\"} 按人/项目/主题查档案；
3. 记一句话事实：{\"action\":\"remember\",\"text\":\"…\"}（不必手搓 turns）；
4. 会话收尾：{\"action\":\"write_session\",\"turns\":[…]} 把值得长期记住的对话写入（蒸馏自动沉淀）；
   长对话分段用 {\"action\":\"append_session\"} 追加；
5. 用户明确表达遗忘：「别记住这个」→ {\"action\":\"forget\"}（void 会话作废且蒸馏产物级联归档；
   误作废用 mode=\"restore\" 撤销）。凭据类内容（密码/密钥）蒸馏会主动跳过，属有意的隐私保护。

projects 域用法：项目 = 一件有明确目标、一次干不完、跨多次会话推进的工作。
1. 开工：{\"action\":\"list\"} / {\"action\":\"get\"} 找到这件事的锚点接上上下文；没有就 {\"action\":\"create\"}；
2. 找内容：get 默认索引模式（文档只给 id/分类/标题/字符数）；{\"action\":\"doc_search\"} 定位到哪篇哪行，
   {\"action\":\"doc_get\",\"doc_id\":\"…\",\"start_line\":…,\"end_line\":…} 区间精读——按需取用，无截断；
3. 干活中：{\"action\":\"doc_add\"} / {\"action\":\"doc_update\"} 沉淀进展与结论；
   小修一段用 {\"action\":\"doc_patch\"}（行级 replace/insert/delete，不必取全文重发）；
4. 收尾：{\"action\":\"update\"} 改状态、写总结文档，下次会话从 get 接上。

skills 域用法：技能 = 可复用的指令包（SKILL.md 形态 + scripts/references 附件）。
1. 需要某种能力前：{\"action\":\"list\"} 看有没有现成技能，命中 {\"action\":\"get\"} 照做；
2. 用户说「把这个做法存成技能」：{\"action\":\"create\"}；修正演进：{\"action\":\"update\"}（自动留版本）；
   改坏了 {\"action\":\"versions\"} 查历史、{\"action\":\"restore\"} 回滚；
3. 用户给现成 SKILL.md：{\"action\":\"import\"}。

wiki 域用法：多库知识库（Markdown 页面 + [[wikilink]] + 混合检索）。每个操作可选 \"library\":\"<库slug>\"
（缺省 main 主库）；{\"action\":\"libraries\"} 列出全部库及页面计数，建库/删库在 Web。
1. 查证事实性知识 → {\"action\":\"search\"}（命中带片段，全文 get_page）；浏览结构 → {\"action\":\"list_pages\"} / {\"action\":\"graph\"}（均按库）；
2. 沉淀：单条结论 {\"action\":\"archive_query\"}，整篇文档 {\"action\":\"ingest\"}（异步，产物落同库），明确要页面 {\"action\":\"write_page\"}（覆盖前先 get_page，旧文自动留版本）；
3. 版本与原料：{\"action\":\"versions\"}/{\"action\":\"restore_version\"} 查历史与回滚（误删页可重建）；{\"action\":\"sources\"}/{\"action\":\"delete_source\"} 清理织入原料（lint 报 stale_source 时用）。

todos 域用法：不绑定项目的快速待办（灵感/学习计划/系统操作/问题排查）。
{\"action\":\"add\",\"title\":\"…\"} 秒记；{\"action\":\"list\"} 看进行中；{\"action\":\"done\",\"id\":\"…\"} 完成。

codegraph 域用法：注册代码库 → index/sync → {\"action\":\"query\"}（search/explore/node/callers/callees/impact）读懂调用关系。\
explore 默认返回符号大纲（不带源码）；单符号源码用 node，整份源码 explore 传 include_source=true。

域的选择：回忆「用户本人是谁、偏好什么、经历过什么」用 memory；查证「客观知识」用 wiki；
跨会话的工作线用 projects；可复用能力用 skills；不确定在哪域就 search_all。
LLM 供应商/模型的配置与排障是管理员专属，走 Web 控制台「设置 → AI 功能」——MCP 工具面
不提供 provider 配置工具（AI 报 LLM 未配置时，引导用户去设置页，不要尝试自行配置）。

分权规则（务必遵守）：
- 用户记忆的写入通道只有「写会话」：事实抽取、画像更新、实体维护全部由蒸馏完成；
- 直接改写用户记忆语义内容（原子内容、画像分面、实体档案）是用户专属权限，MCP 工具面不提供；
- 纠错也走会话：把正确的表述写成对话（correction 语义），蒸馏会自动生成取代链；
- 敏感对话（医疗/感情/财务等）写入时可置 sensitive=true——敏感是标记不是隐身，检索与上下文默认可见（2026-09-12 口径放开）；凭据类（密码/密钥/token）无论 sensitive 一律不写入；
- 破坏性操作（各域 delete/forget 类，目录里有【破坏性】标注）不可逆，只对用户明确请求使用；
- skills delete 仅限用户明确要求——内容过时用 update 修订，改坏用 restore 回滚，不要自行删除。\
";

#[tool_handler(router = self.tool_router)]
impl ServerHandler for EngramMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("engram", env!("CARGO_PKG_VERSION")))
            .with_instructions(SERVER_INSTRUCTIONS)
    }

    /// 停用工具不进 tools/list（对 AI 隐身），控制台管理端点仍展示全量。
    /// 另按 key 的 scope 过滤：memory-only 的 key 不展示 project_* 工具（反之亦然），
    /// AI 客户端看到的工具面与它实际能调用的完全一致。
    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, rmcp::ErrorData> {
        let cfg = load_config(&self.state.pool).await;
        // 认证主体缺失时不做 scope 过滤（协议能力层放行；业务拒绝在 tools/call 的 scope 检查）；
        // search_all 是跨域工具——只要持有任一可检索域的 scope 就可见（域内结果按 scope 分域执行）
        let scope = principal_of(&context).ok();
        let tools: Vec<_> = self
            .tool_router
            .list_all()
            .into_iter()
            .filter(|t| !cfg.disabled_tools.iter().any(|d| d == t.name.as_ref()))
            .filter(|t| {
                scope.as_ref().is_none_or(|p| match t.name.as_ref() {
                    "search_all" => ["memory", "wiki", "skills", "todos", "project"]
                        .iter()
                        .any(|s| p.has_scope(s)),
                    name => p.has_scope(tool_scope(name)),
                })
            })
            .collect();
        // 动态描述：发现能力长在工具面上——域操作目录（L0）织进域工具描述，
        // 云端资产清单（技能/项目/代码库）原样沿用（见 tools_catalog 段）
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        let catalogs = ToolCatalogs::for_tools(&self.state.pool, &names).await;
        let tools = tools
            .into_iter()
            .map(|t| {
                let name = t.name.as_ref();
                let asset = catalogs.extra_for(name);
                let catalog = dispatch::render_catalog(name, &cfg.disabled_tools);
                let extra = match (catalog, asset) {
                    (Some(c), Some(a)) => Some(format!("{c}\n\n{a}")),
                    (Some(c), None) => Some(c),
                    (None, Some(a)) => Some(a.to_string()),
                    (None, None) => None,
                };
                with_dynamic_description(t, extra.as_deref())
            })
            .collect();
        let supports_cache_hints = context
            .protocol_version()
            .is_some_and(|version| version >= rmcp::model::ProtocolVersion::V_2026_07_28);
        Ok(rmcp::model::ListToolsResult {
            result_type: Some(rmcp::model::ResultType::COMPLETE),
            tools,
            meta: None,
            next_cursor: None,
            ttl_ms: supports_cache_hints.then_some(0),
            cache_scope: supports_cache_hints.then_some(rmcp::model::CacheScope::Public),
        })
    }

    /// 停用工具的调用直接拒绝（服务总开关在 HTTP gate 层已拦）。
    /// 渐进式发现：域内单个操作也可停用（disabled_tools 里的 `域.action` 键）。
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, rmcp::ErrorData> {
        let cfg = load_config(&self.state.pool).await;
        let name = request.name.as_ref();
        if cfg.disabled_tools.iter().any(|d| d == name) {
            return Err(mcp_err(
                ErrorCode::INVALID_REQUEST,
                format!("工具 {name} 已停用——控制台「MCP」页可重新开启"),
            ));
        }
        // action 级开关：{"action":"delete"} → 键 "todos.delete"
        if dispatch::is_domain_tool(name) {
            let action = request
                .arguments
                .as_ref()
                .and_then(|a| a.get("action"))
                .and_then(|v| v.as_str());
            if let Some(action) = action {
                let key = dispatch::action_key(name, action);
                if cfg.disabled_tools.iter().any(|d| d == &key) {
                    return Err(mcp_err(
                        ErrorCode::INVALID_REQUEST,
                        format!("操作 {key} 已停用——控制台「MCP」页可重新开启"),
                    ));
                }
            }
        }
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }
}

/// 构造挂载到 axum 的 MCP 服务（Streamable HTTP，会话保存在进程内存）。
///
/// Host 白名单：SDK 默认只放行 loopback（防 DNS rebinding）；远程部署用
/// `AGENT_MEMORY_MCP_ALLOWED_HOSTS`（逗号分隔，如 `mem.example.com,mem.example.com:8080`）放开。
pub fn service(state: AppState) -> StreamableHttpService<EngramMcpServer, LocalSessionManager> {
    let mut config = StreamableHttpServerConfig::default();
    // 工具面是纯 request-response（无服务端主动通知）：全无状态 + JSON 响应最稳——
    // 每个请求独立认证、独立应答，无会话句柄依赖
    config.legacy_session_mode = false;
    config.json_response = true;
    if let Ok(hosts) = std::env::var("AGENT_MEMORY_MCP_ALLOWED_HOSTS") {
        let list: Vec<String> = hosts
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if !list.is_empty() {
            config.allowed_hosts = list;
        }
    }
    StreamableHttpService::new(
        move || Ok(EngramMcpServer::new(state.clone())),
        std::sync::Arc::new(LocalSessionManager::default()),
        config,
    )
}

// ---------- 动态工具描述（发现能力长在工具面上） ----------
//
// 云端资产（技能/项目）的清单随库内容即时变化，AI 的发现通道只有 tools/list——
// 把「当前有什么可用」直接织进工具描述（Claude Code Skill 工具同款思路），
// 用户不需要在提示词里维护路由清单。tools/list 与控制台 /settings/mcp 同源拼装，
// 管理台看到的描述就是 AI 实际收到的描述。

/// 单工具动态清单条数上限（防清单膨胀无限挤占 AI 上下文；超出部分提示用工具查全量）。
const CATALOG_ITEM_CAP: usize = 40;

/// skills_list 动态段：枚举启用技能（slug：name/description）。
/// 云部署后的技能发现入口——库里有什么，AI 连上来第一眼就看到。
async fn skills_catalog(pool: &engram_storage::PgPool) -> Option<String> {
    let rows = engram_storage::repo::skills::list_skills(pool, None, None, Some(true))
        .await
        .ok()?;
    if rows.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = rows
        .iter()
        .take(CATALOG_ITEM_CAP)
        .map(|s| {
            let desc = if s.description.trim().is_empty() {
                s.name.as_str()
            } else {
                s.description.as_str()
            };
            format!("- {}：{}", s.slug, desc)
        })
        .collect();
    if rows.len() > CATALOG_ITEM_CAP {
        lines.push(format!(
            "…其余 {} 个请调用本工具查看完整清单",
            rows.len() - CATALOG_ITEM_CAP
        ));
    }
    Some(format!(
        "【当前可用技能 {} 个】（命中候选后用 skills 的 action=\"get\" 取全文照做）\n{}",
        rows.len(),
        lines.join("\n")
    ))
}

/// project_list 动态段：枚举项目（name/类型/状态/描述）。
/// project_get / project_doc_* 都按项目名寻址——清单进描述可省一次列表往返。
async fn projects_catalog(pool: &engram_storage::PgPool) -> Option<String> {
    let rows = engram_storage::repo::project::list_projects(pool, None)
        .await
        .ok()?;
    if rows.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = rows
        .iter()
        .take(CATALOG_ITEM_CAP)
        .map(|p| {
            let desc = p.description.as_deref().unwrap_or("").trim();
            let desc = if desc.is_empty() {
                String::new()
            } else {
                let head: String = desc.chars().take(80).collect();
                format!("：{head}")
            };
            format!(
                "- {}（{}·{}）{}",
                p.name,
                engram_core::project::type_label(&p.r#type),
                p.status,
                desc
            )
        })
        .collect();
    if rows.len() > CATALOG_ITEM_CAP {
        lines.push(format!(
            "…其余 {} 个请调用本工具查看完整清单",
            rows.len() - CATALOG_ITEM_CAP
        ));
    }
    Some(format!(
        "【当前项目 {} 个】（projects 的 action=\"get\" / action=\"doc_get\" 支持按 name 寻址）\n{}",
        rows.len(),
        lines.join("\n")
    ))
}

/// codegraph_list 动态段：项目清单（name·状态·规模）。
/// AI 连上即知道有哪些代码库可查、哪个 ready。
async fn codegraph_catalog(pool: &engram_storage::PgPool) -> Option<String> {
    let rows = engram_cg_bridge::CgBridge::new(pool.clone(), cg_root_from_env())
        .list()
        .await
        .ok()?;
    if rows.is_empty() {
        return None;
    }
    let lines: Vec<String> = rows
        .iter()
        .take(CATALOG_ITEM_CAP)
        .map(|p| {
            let scale = p
                .stats
                .as_ref()
                .and_then(|s| {
                    let f = s.get("files")?.as_i64()?;
                    let n = s.get("symbols")?.as_i64()?;
                    Some(format!("{f} 文件/{n} 符号"))
                })
                .unwrap_or_else(|| "未索引".into());
            format!("- {}（{}）{}", p.name, p.status, scale)
        })
        .collect();
    Some(format!(
        "【已注册代码库 {} 个】（codegraph 的 action=\"query\" 按 name 查询）
{}",
        rows.len(),
        lines.join(
            "
"
        )
    ))
}

/// MCP 工具面的 codegraph 工作目录：与 api 同约定（AGENT_MEMORY_DATA_DIR/codegraph）。
fn cg_root_from_env() -> std::path::PathBuf {
    std::path::Path::new(
        &std::env::var("AGENT_MEMORY_DATA_DIR").unwrap_or_else(|_| "./data".into()),
    )
    .join("codegraph")
}

/// 按本次工具面实际包含的工具惰性取动态清单（工具不在面内就不查库）。
struct ToolCatalogs {
    skills: Option<String>,
    projects: Option<String>,
    codegraph: Option<String>,
}

impl ToolCatalogs {
    async fn for_tools(pool: &engram_storage::PgPool, names: &[&str]) -> Self {
        let skills = if names.contains(&"skills") {
            skills_catalog(pool).await
        } else {
            None
        };
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
            skills,
            projects,
            codegraph,
        }
    }

    fn extra_for(&self, name: &str) -> Option<&str> {
        match name {
            "skills" => self.skills.as_deref(),
            "projects" => self.projects.as_deref(),
            "codegraph" => self.codegraph.as_deref(),
            _ => None,
        }
    }
}

/// 把动态清单段拼到工具描述尾部（tools/list 与控制台共用）。
fn with_dynamic_description(mut t: rmcp::model::Tool, extra: Option<&str>) -> rmcp::model::Tool {
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
fn default_true() -> bool {
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

const MCP_SETTINGS_KEY: &str = "mcp";

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
    let mut keys: Vec<String> = EngramMcpServer::tool_router()
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
    let router = EngramMcpServer::tool_router();
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

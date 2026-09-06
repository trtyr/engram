//! MCP（Model Context Protocol）适配器：四域工具面（用户记忆 / 项目记忆 / 技能 / Wiki）。
//!
//! 官方 Rust SDK（rmcp）Streamable HTTP 传输，由 api 装配到 engram-server 的 `/mcp` 端点。
//! 与 HTTP 路由平级的第二适配器：同一套 core 服务与 scope 分权，独立成 crate。
//! 单服务器多域：工具名前缀即域（memory_* / wiki_*，管理台按域分组），
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

pub mod wiki;

use axum::response::IntoResponse;
use engram_core::auth::Principal;
use engram_core::state::AppState;

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
fn tool_scope(name: &str) -> &'static str {
    match name.split('_').next() {
        Some("project") => "project",
        Some("skills") => "skills",
        Some("wiki") => "wiki",
        Some("codegraph") => "codegraph",
        _ => "memory",
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

// ---------- 工具参数 ----------

/// memory_context / memory_search 公共可选参数里的时间串直接用 String（ISO8601）。

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
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ListAtomsParams {
    /// 原子类型：preference/fact/decision/event/insight/correction/failure/convention
    #[schemars(
        description = "可选：按类型过滤。preference=偏好, fact=事实, decision=决策, event=事件, insight=洞察, correction=纠正, failure=失败, convention=惯例。"
    )]
    pub kind: Option<String>,
    /// active / superseded / archived
    #[schemars(
        description = "可选：按状态过滤。active=有效（默认查这个）, superseded=被取代, archived=归档。"
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
    /// 会话级敏感标记（医疗/感情/财务等隐私）：蒸馏产物继承，默认不进检索
    #[schemars(
        description = "整段对话含用户隐私（医疗/感情/财务等）时置 true：蒸馏产物自动继承敏感标记，默认不进检索与上下文包。"
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
    #[schemars(description = "要遗忘的会话 id（UUID）。")]
    pub session_id: String,
    /// void（默认：蒸馏跳过，记录保留）| erase（需 erase scope：物理删除）
    #[schemars(
        description = "遗忘力度：\"void\"（默认，推荐——会话作废、原文保留，已蒸馏产物自动级联归档）/ \"erase\"（物理删除，需要 erase scope 的 key）。"
    )]
    pub mode: Option<String>,
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
    #[schemars(description = "文档标题（同项目同分类下唯一）。")]
    pub title: String,
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

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CgQueryParams {
    /// 项目名（codegraph_list 里的 name；也接受 id）
    #[schemars(description = "项目名（codegraph_list 返回的 name；也接受 uuid）。")]
    pub project: String,
    /// explore | search | node | callers | callees | impact
    #[schemars(
        description = "查询类型：search=搜符号, explore=区域总览(markdown), node=符号详情(markdown), callers=谁调用它, callees=它调用谁, impact=改动影响面。"
    )]
    pub kind: String,
    /// 查询文本或符号名
    #[schemars(
        description = "查询文本（search/explore）或符号名（node/callers/callees/impact）。"
    )]
    pub target: String,
    /// explore→max-files；impact→depth
    #[schemars(description = "可选：explore 的 max-files 或 impact 的 depth。")]
    pub depth: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsGetParams {
    /// 技能 slug（kebab-case 标识）
    #[schemars(description = "技能 slug（来自 skills_list 的返回，如 review-pr）。")]
    pub slug: String,
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
    #[schemars(description = "可选：true=启用 / false=停用（停用后列表与检索对 AI 隐身）。")]
    pub enabled: Option<bool>,
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
    #[tool(
        name = "memory_context",
        annotations(
            title = "装载记忆上下文",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
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
        ok_json(serde_json::to_value(&pack).unwrap_or(serde_json::json!({})))
    }

    /// 定向检索用户记忆（全文 + 向量混合，跨 L1/L2/L3/实体四层）。
    ///
    /// 何时用：对话中需要回忆与当前话题相关的用户背景、既往决策、偏好、历史事件时。
    /// 何时不用：会话开场的全景装载用 memory_context；浏览全量列表用 memory_list_atoms。
    /// 命中会回写热度（hit_count），常被检索的内容会在整理中获得更高权重。
    /// 敏感条目默认排除；返回 {entities, l1, l2, l3}，各元素含 score/title/snippet。
    #[tool(
        name = "memory_search",
        annotations(
            title = "检索用户记忆",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
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
                false,
                from,
                to,
            )
            .await
            .map_err(from_memory)?;
        ok_json(serde_json::to_value(&resp).unwrap_or(serde_json::json!({})))
    }

    /// 浏览 L1 原子事实列表（keyset 分页，可按类型/状态/待审过滤）。
    ///
    /// 何时用：需要系统性浏览用户的事实条目（而非定向检索）时；或巡检 needs_review 条目。
    /// 何时不用：有明确主题的回忆用 memory_search；开场装载用 memory_context。
    #[tool(
        name = "memory_list_atoms",
        annotations(
            title = "列出原子事实",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn memory_list_atoms(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ListAtomsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let lp = params.0;
        let cursor = lp.cursor.as_deref().map(parse_flex_datetime).transpose()?;
        let atoms = self
            .svc()
            .list_atoms(
                lp.kind.as_deref(),
                lp.status.as_deref(),
                lp.needs_review,
                cursor,
                lp.limit.unwrap_or(100),
            )
            .await
            .map_err(from_memory)?;
        ok_json(serde_json::to_value(&atoms).unwrap_or(serde_json::json!([])))
    }

    /// 列出 L0 原始会话（keyset 分页，可按 agent 过滤）。
    ///
    /// 何时用：找某段对话的原文入口时（拿到 session_id 后用 memory_get_session 看全文）。
    #[tool(
        name = "memory_list_sessions",
        annotations(
            title = "列出原始会话",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn memory_list_sessions(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ListSessionsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let lp = params.0;
        let cursor = lp.cursor.as_deref().map(parse_flex_datetime).transpose()?;
        let sessions = self
            .svc()
            .list_sessions(lp.agent.as_deref(), cursor, lp.limit.unwrap_or(50))
            .await
            .map_err(from_memory)?;
        ok_json(serde_json::to_value(&sessions).unwrap_or(serde_json::json!([])))
    }

    /// 读取一个 L0 原始会话全文（逐轮对话原文）。
    ///
    /// 何时用：memory_search / memory_list_sessions 定位到会话后，需要核对原文细节时。
    #[tool(
        name = "memory_get_session",
        annotations(
            title = "读取原始会话",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "memory_write_session",
        annotations(
            title = "写入会话记忆",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
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
        ok_json(serde_json::to_value(&s).unwrap_or(serde_json::json!({})))
    }

    /// 向一个未蒸馏的会话追加轮次（长对话分片落库，不必等收尾一次性写）。
    ///
    /// 何时用：同一会话持续进行、已用 memory_write_session 开头后，后续内容追加进来。
    /// 已蒸馏的会话不可追加（会报错）——那就新开一个会话。
    #[tool(
        name = "memory_append_session",
        annotations(
            title = "追加会话轮次",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
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
        ok_json(serde_json::to_value(&s).unwrap_or(serde_json::json!({})))
    }

    /// 遗忘：用户说「别记住这段/把这事忘了」时使用，对任何会话都有效。
    ///
    /// mode="void"（默认）：该会话作废、原文保留可审计；若会话已蒸馏，其产出的
    /// active 原子会**级联归档**——检索与上下文包立即不再返回它们。
    /// mode="erase"：物理删除该会话（不可逆，需要 erase scope 的 key）。
    /// 注意：只对用户明确表达的遗忘请求使用，不要自行判断「这段不重要」就遗忘。
    #[tool(
        name = "memory_forget",
        annotations(
            title = "遗忘会话",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
            other => Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                format!("mode 只支持 void / erase，收到 {other:?}"),
            )),
        }
    }

    /// 检索实体（用户记忆的横向透镜：人物/项目/主题/群组/地点）。
    ///
    /// 何时用：想按「某个具体的人/项目/主题」横向拉出相关记忆线索时；
    /// 或对话中出现新人物/项目，先查一下是否已有档案。
    /// 实体由蒸馏从会话中自动抽取维护——发现信息更新请写会话，不要要求直接改实体。
    #[tool(
        name = "memory_entities",
        annotations(
            title = "检索实体",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "project_types",
        annotations(
            title = "列出项目类型模板",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "project_list",
        annotations(
            title = "列出项目",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "project_get",
        annotations(
            title = "项目详情",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "project_create",
        annotations(
            title = "新建项目",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "project_update",
        annotations(
            title = "编辑项目",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "project_delete",
        annotations(
            title = "删除项目",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "project_batch_delete",
        annotations(
            title = "批量删除项目",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "project_location_add",
        annotations(
            title = "登记项目位置",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "project_location_update",
        annotations(
            title = "编辑项目位置",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "project_location_delete",
        annotations(
            title = "删除项目位置",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "project_doc_add",
        annotations(
            title = "新增项目文档",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
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
            .add_doc(id, &dp.category, &dp.title, &dp.content)
            .await
            .map_err(from_project)?;
        ok_json(serde_json::to_value(&doc).unwrap_or(serde_json::json!({})))
    }

    /// 读取项目文档：全文或按行区间精读（1-based，含两端）。
    ///
    /// 何时用：project_get 索引或 project_doc_search 命中之后精确读内容。
    /// 传 start_line/end_line 只读该区间（输出恒带「行号: 」前缀，便于连环寻址）；
    /// 不传读全文（无损，不截断），with_line_numbers=true 可给全文加行号。
    /// 行号基于文档当前版本——改文档后需重取。
    #[tool(
        name = "project_doc_get",
        annotations(
            title = "读取项目文档",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "project_doc_search",
        annotations(
            title = "检索项目文档行",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "project_doc_update",
        annotations(
            title = "编辑项目文档",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
        let current = self.svc_project().get_doc(id).await.map_err(from_project)?;
        // 分类只在换分类时由 service 校验（保留被移除分类下的存量文档原地编辑能力）
        let doc = self
            .svc_project()
            .update_doc(
                id,
                &dp.category.unwrap_or(current.category),
                &dp.title.unwrap_or(current.title),
                &dp.content.unwrap_or(current.content),
            )
            .await
            .map_err(from_project)?;
        ok_json(serde_json::to_value(&doc).unwrap_or(serde_json::json!({})))
    }

    /// 删除项目文档（不可逆）。
    ///
    /// 何时用：文档写废或彻底过时。只对明确表达的删除请求使用。
    #[tool(
        name = "project_doc_delete",
        annotations(
            title = "删除项目文档",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "skills_list",
        annotations(
            title = "列出技能",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "skills_get",
        annotations(
            title = "读取技能",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "skills_file_get",
        annotations(
            title = "读取技能文件",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "skills_file_put",
        annotations(
            title = "写入技能文件",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "codegraph_list",
        annotations(
            title = "列出图谱项目",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn codegraph_list(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_codegraph(&p)?;
        let rows = cg_svc(&self.state).list().await.map_err(from_cg)?;
        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
    }

    /// 注册代码图谱项目（本地绝对路径或 git URL）。
    ///
    /// 何时用：想让 AI 理解某个代码库的结构与调用关系时。注册后须 codegraph_index
    /// 建索引（异步 job，稍等片刻再 codegraph_list 确认 ready）。
    #[tool(
        name = "codegraph_register",
        annotations(
            title = "注册图谱项目",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "codegraph_index",
        annotations(
            title = "建图谱索引",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
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
            "hint": "索引异步执行（首次可能数分钟）——稍后 codegraph_list 确认 ready",
        }))
    }

    /// 增量同步索引（代码小改动后刷新；异步 job）。
    ///
    /// 何时用：项目 ready 后代码有小改动，不想全量重建时。
    #[tool(
        name = "codegraph_sync",
        annotations(
            title = "同步图谱索引",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "codegraph_query",
        annotations(
            title = "图谱查询",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn codegraph_query(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<CgQueryParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_codegraph(&p)?;
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
        let v = cg_svc(&self.state)
            .query(id, kind, &params.0.target, params.0.depth)
            .await
            .map_err(from_cg)?;
        ok_json(v)
    }

    /// 沉淀新技能：把本次对话中验证有效的做法固化成可复用指令包。
    ///
    /// 何时用：用户说「把这个做法存成技能/记成 SOP」，或一套流程已被验证有效且可复用时。
    /// 何时不用：一次性的操作细节不值得建技能；用户个人事实走记忆域（memory_write_session）。
    #[tool(
        name = "skills_create",
        annotations(
            title = "创建技能",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
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
            })
            .await
            .map_err(from_skills)?;
        ok_json(serde_json::to_value(&s).unwrap_or(serde_json::json!({})))
    }

    /// 更新技能（正文/名称/描述/标签/启停；语义变更自动留版本快照，可回滚）。
    ///
    /// 何时用：技能做法需要修正或演进时（如用户指出了更好的步骤）。
    /// 注意：改坏可回滚（版本快照），但删除不可逆——拿不准就更新而不是删除。
    #[tool(
        name = "skills_update",
        annotations(
            title = "更新技能",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
                },
            )
            .await
            .map_err(from_skills)?;
        ok_json(serde_json::to_value(&s).unwrap_or(serde_json::json!({})))
    }

    /// 删除技能（级联删版本快照，不可逆）。
    ///
    /// 何时用：仅当用户明确要求删除某个技能时。不要因「内容过时」自行删除——用 skills_update 修订。
    #[tool(
        name = "skills_delete",
        annotations(
            title = "删除技能",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
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
    #[tool(
        name = "skills_import",
        annotations(
            title = "导入 SKILL.md",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
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
                })
                .await
                .map(|s| ("imported", s))
        };
        let (status, s) = result.map_err(from_skills)?;
        ok_json(serde_json::json!({ "status": status, "skill": s }))
    }

    // ---------- Wiki 域（wiki scope；实现细节见 mcp_wiki.rs） ----------

    /// Wiki 检索（FTS + 向量 RRF 融合，带 wiki 方向意图 purpose）。
    ///
    /// 何时用：需要查证「世界知识」（用户 Wiki 里沉淀的文档、概念、实体、问答）时。
    /// 何时不用：回忆「用户本人」的偏好/事实/经历用 memory_search——那是用户记忆域。
    /// 返回 {purpose, pages}：purpose 是 wiki 的研究方向意图（未设为 null），
    /// pages 为命中页面全文；写作前先检索可避免造重复页面。
    #[tool(
        name = "wiki_search",
        annotations(
            title = "检索 Wiki",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn wiki_search(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiSearchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let wp = params.0;
        let result = wiki::svc(&self.state)
            .search_with_purpose(&wp.query, wp.max_items.unwrap_or(20))
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(result)
    }

    /// 浏览 Wiki 页面列表（可按页型过滤；列表不带正文）。
    ///
    /// 何时用：想系统性看看 Wiki 里有什么（而非定向检索）时；读全文用 wiki_get_page。
    #[tool(
        name = "wiki_list_pages",
        annotations(
            title = "列出 Wiki 页面",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn wiki_list_pages(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiListPagesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lp = params.0;
        let pages = wiki::svc(&self.state)
            .list_pages(lp.page_type.as_deref(), lp.limit.unwrap_or(100))
            .await
            .map_err(wiki::from_wiki)?;
        let values: Vec<serde_json::Value> = serde_json::to_value(&pages)
            .unwrap_or(serde_json::json!([]))
            .as_array()
            .map(|a| a.iter().map(wiki::trim_page).collect())
            .unwrap_or_default();
        ok_json(serde_json::to_value(&values).unwrap_or(serde_json::json!([])))
    }

    /// 读取一个 Wiki 页面全文（含 frontmatter 与版本）。
    ///
    /// 何时用：wiki_search / wiki_list_pages 定位到页面后需要读全文时。
    #[tool(
        name = "wiki_get_page",
        annotations(
            title = "读取 Wiki 页面",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn wiki_get_page(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiGetPageParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let page = wiki::svc(&self.state)
            .get_page(&params.0.slug)
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
    #[tool(
        name = "wiki_write_page",
        annotations(
            title = "写入 Wiki 页面",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn wiki_write_page(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiWritePageParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let wp = params.0;
        let page = wiki::svc(&self.state)
            .put_page(
                &wp.slug,
                &wp.title,
                &wp.content,
                wp.folder.as_deref(),
                Some("ai"),
            )
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::to_value(&page).unwrap_or(serde_json::json!({})))
    }

    /// 把一段源文本织入 Wiki（异步：入队 LLM 流水线，自动抽取实体/概念并互链）。
    ///
    /// 何时用：有一篇完整文档 / 长文本值得沉淀进知识库时。内容相同（sha 命中）会跳过。
    /// 注意：织入是异步任务（前端「任务」页可见），立即返回 skipped 只代表入队/去重结果；
    /// 单条问答式的结论用 wiki_archive_query 更合适。
    #[tool(
        name = "wiki_ingest",
        annotations(
            title = "织入 Wiki 来源",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn wiki_ingest(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiIngestParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let wp = params.0;
        let skipped = wiki::svc(&self.state)
            .ingest(&wp.title, &wp.text)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::json!({
            "skipped": skipped,
            "async": true,
            "message": if skipped {
                "内容已存在（sha 命中），本次跳过"
            } else {
                "已入队织入任务——LLM 流水线异步处理，稍后可在 Wiki 页面看到产物"
            }
        }))
    }

    /// 把一条问答（问 + 答）存档为 queries 页并自动再摄取。
    ///
    /// 何时用：一次检索/讨论得出值得长期保留的结论时，落成「查询」页沉淀。
    /// 同标题已存档 → 幂等跳过（skipped=true），不会重复烧 LLM。
    #[tool(
        name = "wiki_archive_query",
        annotations(
            title = "存档问答",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn wiki_archive_query(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiArchiveQueryParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let qp = params.0;
        let skipped = wiki::svc(&self.state)
            .archive_query(&qp.title, &qp.question, &qp.answer)
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
    #[tool(
        name = "wiki_graph",
        annotations(
            title = "Wiki 链接图",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn wiki_graph(
        &self,
        ctx: RequestContext<RoleServer>,
        _params: Parameters<wiki::WikiNoParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let graph = wiki::svc(&self.state)
            .graph()
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::to_value(&graph).unwrap_or(serde_json::json!({})))
    }

    /// Wiki 健康检查（死链、孤页、缺源等 lint 报告）。
    ///
    /// 何时用：怀疑 Wiki 结构有问题（失效双链、孤立页面）时体检；只报告不修改。
    #[tool(
        name = "wiki_lint",
        annotations(
            title = "Wiki 健康检查",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn wiki_lint(
        &self,
        ctx: RequestContext<RoleServer>,
        _params: Parameters<wiki::WikiNoParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let report = wiki::svc(&self.state)
            .lint()
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::to_value(&report).unwrap_or(serde_json::json!({})))
    }
}

/// MCP instructions：initialize 时返回给调用方 AI 的顶层使用说明。
const SERVER_INSTRUCTIONS: &str = "\
Engram —— 长期记忆平台（用户记忆域 + 项目记忆域 + 技能域 + Wiki 域 MCP）。

用户记忆分四层蒸馏：L0 原始会话 →（蒸馏）→ L1 原子事实 → L2 场景模式 → L3 用户画像；\
另有实体坐标系（人物/项目/主题/群组/地点）横向串联记忆。全部记忆可溯源、可遗忘。
项目记忆是跨会话工作线：项目 = 一件有明确目标、一次干不完、跨多次会话推进的工作，\
下挂多主机位置（登记制）与「分类 > 文档」树。

用户记忆用法：
1. 会话开始：调用 memory_context 装载用户画像与近期记忆，再开始对话；
2. 对话中需要背景：用 memory_search 定向回忆，或 memory_entities 按人/项目/主题查档案；
3. 会话收尾：用 memory_write_session 把值得长期记住的对话写入（蒸馏自动沉淀为结构化记忆）；
   长对话可分段 memory_append_session 追加；
4. 用户明确表达遗忘：「别记住这个」→ memory_forget（void 会话作废且已蒸馏产物自动级联归档，检索立即失效）。

项目记忆用法：
1. 开工：project_list / project_get 找到这件事的项目锚点，读目标、位置与文档索引接上上下文；
   没有就 project_create 建一个（dev=开发 / research=调研），再用 project_location_add 登记代码位置；
2. 找内容：project_get 默认索引模式（文档只给 id/分类/标题/字符数，不带正文）；
   project_doc_search 按关键词定位到「哪篇文档哪一行」，project_doc_get 传 start_line/end_line
   区间精读（无截断、无损）——按需取用，不必整篇倒腾；
3. 干活中：有阶段性进展或结论就 project_doc_add / project_doc_update 沉淀成文档
   （category 必须是项目已有分类，要新分类先 project_update 追加进 categories）；
4. 收尾：project_update 改状态、写总结文档，下次会话从 project_get 接上。

技能域用法：技能 = 可复用的指令包（SKILL.md 形态：名称/描述/标签 + markdown 正文）。
1. 需要某种可复用能力前：先 skills_list 看有没有现成技能，命中就 skills_get 照做；
2. 用户说「把这个做法存成技能」或一套流程被验证有效且可复用：skills_create 沉淀；
3. 技能需要修正/演进：skills_update（自动留版本快照，可回滚）；
4. 用户给了现成 SKILL.md：skills_import（frontmatter 容错解析）。

Wiki 域用法：世界知识库——Markdown 页面 + [[wikilink]] 互链 + 混合检索（FTS + 向量）。
1. 需要查证事实性知识（文档、概念、实体、既往问答）→ wiki_search；
2. 浏览结构：wiki_list_pages / wiki_graph，读全文 wiki_get_page；
3. 用户要求把知识沉淀进 Wiki → 单条结论 wiki_archive_query，整篇文档 wiki_ingest（异步织入），
   明确要页面则 wiki_write_page（更新前先 wiki_get_page 读原文，别盲目覆盖）；
4. 怀疑结构问题（死链/孤页）→ wiki_lint。
域的选择：回忆「用户本人是谁、偏好什么、经历过什么」用 memory_*；查证「客观知识」用 wiki_*。

分权规则（务必遵守）：
- 用户记忆的写入通道只有「写会话」：事实抽取、画像更新、实体维护全部由蒸馏完成；
- 直接改写用户记忆语义内容（原子内容、画像分面、实体档案）是用户专属权限，MCP 工具面不提供；
- 纠错也走会话：把正确的表述写成对话（correction 语义），蒸馏会自动生成取代链；
- 敏感对话（医疗/感情/财务等）写入时置 sensitive=true，默认不进检索与上下文；
- project_delete / project_batch_delete 不可逆，只对用户明确表达的删除请求使用；
- skills_delete 仅限用户明确要求——内容过时用 skills_update 修订，不要自行删除。\
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
        // 认证主体缺失时不做 scope 过滤（协议能力层放行；业务拒绝在 tools/call 的 scope 检查）
        let scope = principal_of(&context).ok();
        let tools: Vec<_> = self
            .tool_router
            .list_all()
            .into_iter()
            .filter(|t| !cfg.disabled_tools.iter().any(|d| d == t.name.as_ref()))
            .filter(|t| {
                scope
                    .as_ref()
                    .is_none_or(|p| p.has_scope(tool_scope(t.name.as_ref())))
            })
            .collect();
        // 动态描述：发现能力长在工具面上——云端资产清单织进工具描述（见 tools_catalog 段）
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        let catalogs = ToolCatalogs::for_tools(&self.state.pool, &names).await;
        let tools = tools
            .into_iter()
            .map(|t| {
                let extra = catalogs.extra_for(t.name.as_ref());
                with_dynamic_description(t, extra)
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
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, rmcp::ErrorData> {
        let cfg = load_config(&self.state.pool).await;
        if cfg
            .disabled_tools
            .iter()
            .any(|d| d == request.name.as_ref())
        {
            return Err(mcp_err(
                ErrorCode::INVALID_REQUEST,
                format!("工具 {} 已停用——控制台「MCP」页可重新开启", request.name),
            ));
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
        "【当前可用技能 {} 个】（命中候选后用 skills_get 取全文照做）\n{}",
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
        "【当前项目 {} 个】（project_get / project_doc_* 支持按 name 寻址）\n{}",
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
        "【已注册代码库 {} 个】（codegraph_query 按 name 查询）
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
        let skills = if names.contains(&"skills_list") {
            skills_catalog(pool).await
        } else {
            None
        };
        let projects = if names.contains(&"project_list") {
            projects_catalog(pool).await
        } else {
            None
        };
        let codegraph = if names.contains(&"codegraph_list") {
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
            "skills_list" => self.skills.as_deref(),
            "project_list" => self.projects.as_deref(),
            "codegraph_list" => self.codegraph.as_deref(),
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
    /// 停用的工具名清单：tools/list 不出现、tools/call 报错
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

/// MCP 工具条目（控制台工具清单）。
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct McpToolInfo {
    pub name: String,
    /// 所属资产域（工具名前缀；memory → 用户记忆，wiki → Wiki，未来逐域扩展）
    pub domain: String,
    pub description: String,
    pub read_only: Option<bool>,
    pub destructive: Option<bool>,
    /// 参数 JSON Schema（tools/list 的 inputSchema 同源；控制台详情展示用）
    #[schema(value_type = Object)]
    pub parameters: serde_json::Value,
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
    /// 停用的工具名（tools/list 对 AI 隐身、call 拒绝）
    pub disabled_tools: Vec<String>,
    /// initialize 时下发给调用方 AI 的使用说明（与工具面同源展示）
    pub instructions: String,
    pub tools: Vec<McpToolInfo>,
}

/// 已知工具名清单（控制台配置校验用）。
pub fn tool_catalog() -> Vec<String> {
    EngramMcpServer::tool_router()
        .list_all()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect()
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
            let extra = catalogs.extra_for(t.name.as_ref());
            let t = with_dynamic_description(t, extra);
            McpToolInfo {
                name: t.name.to_string(),
                domain: t.name.split('_').next().unwrap_or("other").to_string(),
                description: t.description.as_deref().unwrap_or("").to_string(),
                read_only: t.annotations.as_ref().and_then(|a| a.read_only_hint),
                destructive: t.annotations.as_ref().and_then(|a| a.destructive_hint),
                parameters: serde_json::to_value(&*t.input_schema).unwrap_or(serde_json::json!({})),
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

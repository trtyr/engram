//! MCP（Model Context Protocol）服务端：用户记忆域 + 项目记忆域工具面。
//!
//! 官方 Rust SDK（rmcp）Streamable HTTP 传输，宿主于 engram-server 的 `/mcp` 端点。
//! 鉴权复用 Bearer 中间件（amk_ key / ams_ 会话）：每个工具调用请求都过 `bearer_auth`，
//! Principal 已注入 request extensions；rmcp 把 HTTP request Parts 注入工具上下文，
//! 工具实现从这里取 Principal 做与 HTTP API 同一套的 scope / 编辑分权检查。
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
use uuid::Uuid;

use crate::auth::Principal;
use crate::state::AppState;
use axum::response::IntoResponse;

/// 错误桥：把本模块的错误统一成 MCP ErrorData。
fn mcp_err(code: ErrorCode, msg: impl Into<String>) -> rmcp::ErrorData {
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
        ProjectError::Conflict(m) => rmcp::ErrorData::new(
            ErrorCode::INVALID_REQUEST,
            m,
            None,
        ),
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
        description = "蒸馏模式：\"auto\"（默认，写入后防抖自动蒸馏）/ \"manual\"（立即触发蒸馏）/ \"off\"（不蒸馏）。一般用默认。"
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
    #[schemars(description = "蒸馏模式：\"auto\"（默认）/ \"off\"。")]
    pub distill: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ForgetParams {
    /// 会话 id（UUID）
    #[schemars(description = "要遗忘的会话 id（UUID）。")]
    pub session_id: String,
    /// void（默认：蒸馏跳过，记录保留）| erase（需 erase scope：物理删除）
    #[schemars(
        description = "遗忘力度：\"void\"（默认，「这段白记了」——蒸馏跳过、原文保留）/ \"erase\"（物理删除，需要 erase scope 的 key）。"
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

/// 项目寻址公共参数：id 或 name 二选一（项目名唯一，可直接用名字）。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectRef {
    /// 项目 id（UUID，来自 project_list / project_get）
    #[schemars(description = "项目 id（UUID，来自 project_list / project_get 的返回）。")]
    pub project_id: Option<String>,
    /// 项目名（项目名唯一，可代替 id 定位）
    #[schemars(description = "项目名（唯一，可代替 project_id 定位）。与 project_id 至少给一个。")]
    pub project_name: Option<String>,
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
    #[schemars(description = "主机 IP（内网/公网/IPv6 均可，仅登记不校验格式；本机可填 127.0.0.1）。")]
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
    #[schemars(description = "可选：替换整个 Markdown 正文（是替换不是追加——改长文档先 project_doc_get 取原文）。不传不改。")]
    pub content: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectDocDeleteParams {
    /// 文档 id（UUID）
    #[schemars(description = "文档 id（UUID，来自 project_get 返回的 docs 列表）。")]
    pub doc_id: String,
}

// ---------- MCP 服务器 ----------

/// Engram 用户记忆 MCP 服务器。工具实现直调 MemoryService（进程内，不走 HTTP 回环）。
#[derive(Clone)]
pub struct MemoryMcpServer {
    state: AppState,
    tool_router: ToolRouter<Self>,
}

impl MemoryMcpServer {
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

    /// 校验分类名是项目已有分类（防笔误造出树上看不见的孤儿分类）。
    async fn require_category(
        &self,
        project_id: Uuid,
        category: &str,
    ) -> Result<Vec<String>, rmcp::ErrorData> {
        let detail = self
            .svc_project()
            .get_project(project_id)
            .await
            .map_err(from_project)?;
        if detail.categories.iter().any(|c| c == category) {
            Ok(detail.categories)
        } else {
            Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "分类「{category}」不在项目分类里——现有：{}。要新分类就先 project_update 把它加进 categories",
                    detail.categories.join("、")
                ),
            ))
        }
    }
}

#[tool_router]
impl MemoryMcpServer {
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

    /// 遗忘：用户说「别记住这段/把这事忘了」时使用。
    ///
    /// mode="void"（默认）：该会话被蒸馏跳过（若尚未蒸馏），原文保留可审计；
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
        ok_json(serde_json::to_value(engram_core::project::ProjectService::list_types())
            .unwrap_or(serde_json::json!([])))
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

    /// 项目详情：本体 + 多主机位置 + 全部分类文档（含正文）。
    ///
    /// 何时用：开工拉上下文——目标（description）、进度与决策（docs）、
    /// 代码在哪（locations）一次拿全。可用 project_id 或 project_name（唯一）定位。
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
        params: Parameters<ProjectRef>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let id = self
            .resolve_project(&params.0.project_id, &params.0.project_name)
            .await?;
        let detail = self.svc_project().get_project(id).await.map_err(from_project)?;
        ok_json(serde_json::to_value(&detail).unwrap_or(serde_json::json!({})))
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
        let current = self.svc_project().get_project(id).await.map_err(from_project)?;
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
        self.svc_project().delete_project(id).await.map_err(from_project)?;
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
                mcp_err(ErrorCode::INVALID_PARAMS, format!("ids 里有非法 UUID：{raw}"))
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
            .add_location(id, &lp.ip, &lp.host, &lp.os, &lp.path, lp.purpose.as_deref())
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
        self.require_category(id, &dp.category).await?;
        let doc = self
            .svc_project()
            .add_doc(id, &dp.category, &dp.title, &dp.content)
            .await
            .map_err(from_project)?;
        ok_json(serde_json::to_value(&doc).unwrap_or(serde_json::json!({})))
    }

    /// 读取单个项目文档全文。
    ///
    /// 何时用：改长文档前取原文（project_doc_update 的 content 是替换式）。
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
        let id = Uuid::parse_str(&params.0.doc_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "doc_id 不是合法 UUID"))?;
        let doc = self.svc_project().get_doc(id).await.map_err(from_project)?;
        ok_json(serde_json::to_value(&doc).unwrap_or(serde_json::json!({})))
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
        if let Some(category) = &dp.category {
            self.require_category(current.project_id, category).await?;
        }
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
        self.svc_project().delete_doc(id).await.map_err(from_project)?;
        ok_json(serde_json::json!({ "deleted": params.0.doc_id }))
    }
}

/// MCP instructions：initialize 时返回给调用方 AI 的顶层使用说明。
const SERVER_INSTRUCTIONS: &str = "\
Engram —— 长期记忆平台（用户记忆域 + 项目记忆域 MCP）。

用户记忆分四层蒸馏：L0 原始会话 →（蒸馏）→ L1 原子事实 → L2 场景模式 → L3 用户画像；\
另有实体坐标系（人物/项目/主题/群组/地点）横向串联记忆。全部记忆可溯源、可遗忘。
项目记忆是跨会话工作线：项目 = 一件有明确目标、一次干不完、跨多次会话推进的工作，\
下挂多主机位置（登记制）与「分类 > 文档」树。

用户记忆用法：
1. 会话开始：调用 memory_context 装载用户画像与近期记忆，再开始对话；
2. 对话中需要背景：用 memory_search 定向回忆，或 memory_entities 按人/项目/主题查档案；
3. 会话收尾：用 memory_write_session 把值得长期记住的对话写入（蒸馏自动沉淀为结构化记忆）；
   长对话可分段 memory_append_session 追加；
4. 用户明确表达遗忘：「别记住这个」→ memory_forget。

项目记忆用法：
1. 开工：project_list / project_get 找到这件事的项目锚点，读目标、进度与决策文档接上上下文；
   没有就 project_create 建一个（dev=开发 / research=调研），再用 project_location_add 登记代码位置；
2. 干活中：有阶段性进展或结论就 project_doc_add / project_doc_update 沉淀成文档
   （category 必须是项目已有分类，要新分类先 project_update 追加进 categories）；
3. 收尾：project_update 改状态、写总结文档，下次会话从 project_get 接上。

分权规则（务必遵守）：
- 用户记忆的写入通道只有「写会话」：事实抽取、画像更新、实体维护全部由蒸馏完成；
- 直接改写用户记忆语义内容（原子内容、画像分面、实体档案）是用户专属权限，MCP 工具面不提供；
- 纠错也走会话：把正确的表述写成对话（correction 语义），蒸馏会自动生成取代链；
- 敏感对话（医疗/感情/财务等）写入时置 sensitive=true，默认不进检索与上下文；
- project_delete / project_batch_delete 不可逆，只对用户明确表达的删除请求使用。\
";

#[tool_handler(router = self.tool_router)]
impl ServerHandler for MemoryMcpServer {
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
pub fn service(state: AppState) -> StreamableHttpService<MemoryMcpServer, LocalSessionManager> {
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
        move || Ok(MemoryMcpServer::new(state.clone())),
        std::sync::Arc::new(LocalSessionManager::default()),
        config,
    )
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
pub async fn load_config(pool: &sqlx::PgPool) -> McpConfig {
    let row: Option<(sqlx::types::Json<McpConfig>,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = $1")
            .bind(MCP_SETTINGS_KEY)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    row.map(|(j,)| j.0).unwrap_or_default()
}

/// 写配置（settings KV upsert）。
pub async fn save_config(
    pool: &sqlx::PgPool,
    cfg: &McpConfig,
) -> Result<(), crate::error::ApiError> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES ($1, $2)
         ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = now()",
    )
    .bind(MCP_SETTINGS_KEY)
    .bind(sqlx::types::Json(cfg))
    .execute(pool)
    .await
    .map_err(|e| crate::error::ApiError::Internal(anyhow::anyhow!(e.to_string())))?;
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

// ---------- 管理端点（Web 控制台用） ----------

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct McpToolInfo {
    pub name: String,
    /// 所属资产域（工具名前缀；memory → 用户记忆，wiki → Wiki，未来逐域扩展）
    pub domain: String,
    pub description: String,
    pub read_only: Option<bool>,
    pub destructive: Option<bool>,
}

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

async fn build_info(pool: &sqlx::PgPool) -> McpInfo {
    let cfg = load_config(pool).await;
    let router = MemoryMcpServer::tool_router();
    let tools = router
        .list_all()
        .into_iter()
        .map(|t| McpToolInfo {
            name: t.name.to_string(),
            domain: t.name.split('_').next().unwrap_or("other").to_string(),
            description: t.description.as_deref().unwrap_or("").to_string(),
            read_only: t.annotations.as_ref().and_then(|a| a.read_only_hint),
            destructive: t.annotations.as_ref().and_then(|a| a.destructive_hint),
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

/// MCP 服务信息（Web 控制台「MCP」页：端点、协议版本、开关状态、工具清单）。
#[utoipa::path(get, path = "/settings/mcp",
    responses((status = 200, body = McpInfo)))]
pub async fn settings_mcp(
    principal: axum::Extension<Principal>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<axum::Json<McpInfo>, crate::error::ApiError> {
    if !matches!(principal.0, Principal::Admin) {
        return Err(crate::error::ApiError::Forbidden("仅限管理员".into()));
    }
    Ok(axum::Json(build_info(&state.pool).await))
}

#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct McpConfigUpdate {
    /// 服务总开关
    pub enabled: Option<bool>,
    /// 停用工具全量清单（覆盖式；空数组 = 全部启用）。未知工具名 400。
    pub disabled_tools: Option<Vec<String>>,
}

/// 更新 MCP 配置（服务开关 / 工具粒度开关）。
#[utoipa::path(put, path = "/settings/mcp",
    request_body = McpConfigUpdate,
    responses((status = 200, body = McpInfo), (status = 400, body = crate::error::ErrorEnvelope)))]
pub async fn settings_mcp_update(
    principal: axum::Extension<Principal>,
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::Json(req): axum::Json<McpConfigUpdate>,
) -> Result<axum::Json<McpInfo>, crate::error::ApiError> {
    if !matches!(principal.0, Principal::Admin) {
        return Err(crate::error::ApiError::Forbidden("仅限管理员".into()));
    }
    let mut cfg = load_config(&state.pool).await;
    if let Some(enabled) = req.enabled {
        cfg.enabled = enabled;
    }
    if let Some(disabled) = req.disabled_tools {
        // 工具名校验：停用一个不存在的名字多半是调用方笔误，宁可 400
        let known: Vec<String> = MemoryMcpServer::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        for name in &disabled {
            if !known.contains(name) {
                return Err(crate::error::ApiError::BadRequest(format!(
                    "未知工具 {name:?}——可用：{}",
                    known.join(", ")
                )));
            }
        }
        cfg.disabled_tools = disabled;
    }
    save_config(&state.pool, &cfg).await?;
    Ok(axum::Json(build_info(&state.pool).await))
}

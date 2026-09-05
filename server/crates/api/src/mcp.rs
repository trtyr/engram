//! MCP（Model Context Protocol）服务端：用户记忆域工具面。
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
}

/// MCP instructions：initialize 时返回给调用方 AI 的顶层使用说明。
const SERVER_INSTRUCTIONS: &str = "\
Engram —— 用户长期记忆平台（用户记忆域 MCP）。

记忆分四层蒸馏：L0 原始会话 →（蒸馏）→ L1 原子事实 → L2 场景模式 → L3 用户画像；\
另有实体坐标系（人物/项目/主题/群组/地点）横向串联记忆。全部记忆可溯源、可遗忘。

使用时机：
1. 会话开始：调用 memory_context 装载用户画像与近期记忆，再开始对话；
2. 对话中需要背景：用 memory_search 定向回忆，或 memory_entities 按人/项目/主题查档案；
3. 会话收尾：用 memory_write_session 把值得长期记住的对话写入（蒸馏自动沉淀为结构化记忆）；
   长对话可分段 memory_append_session 追加；
4. 用户明确表达遗忘：「别记住这个」→ memory_forget。

分权规则（务必遵守）：
- 你的写入通道只有「写会话」：事实抽取、画像更新、实体维护全部由蒸馏完成；
- 直接改写记忆语义内容（原子内容、画像分面、实体档案）是用户专属权限，MCP 工具面不提供；
- 纠错也走会话：把正确的表述写成对话（correction 语义），蒸馏会自动生成取代链；
- 敏感对话（医疗/感情/财务等）写入时置 sensitive=true，默认不进检索与上下文。\
";

#[tool_handler(router = self.tool_router)]
impl ServerHandler for MemoryMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("engram", env!("CARGO_PKG_VERSION")))
            .with_instructions(SERVER_INSTRUCTIONS)
    }

    /// 停用工具不进 tools/list（对 AI 隐身），控制台管理端点仍展示全量。
    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, rmcp::ErrorData> {
        let cfg = load_config(&self.state.pool).await;
        let tools: Vec<_> = self
            .tool_router
            .list_all()
            .into_iter()
            .filter(|t| !cfg.disabled_tools.iter().any(|d| d == t.name.as_ref()))
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

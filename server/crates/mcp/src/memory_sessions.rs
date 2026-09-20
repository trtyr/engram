//! memory_sessions 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

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
    /// 客户端幂等键（可选）：同一 ref 重复调用返回原会话不新建——网络重试防重
    #[schemars(
        description = "可选：客户端幂等键（如 UUID）。同一 ref 重复调用返回原会话不新建——网络重试防重。"
    )]
    pub client_ref: Option<String>,
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

#[tool_router(router = memory_sessions_router)]
impl EngramMcpServer {
    /// 列出 L0 原始会话（keyset 分页，可按 agent 过滤）。
    ///
    /// 何时用：找某段对话的原文入口时（拿到 session_id 后用 memory_get_session 看全文）。
    pub(crate) async fn memory_list_sessions(
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
    pub(crate) async fn memory_get_session(
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
    pub(crate) async fn memory_write_session(
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
        // P001 身份归因：key 写入自动带 key_id + key 名快照（key 删除不丢归因）
        let (api_key_id, key_name_snapshot) = match &principal {
            Principal::ApiKey { key_id, name, .. } => (Some(*key_id), Some(name.as_str())),
            Principal::Admin => (None, None),
        };
        let s = self
            .svc()
            .write_session_identity(
                &agent,
                turns,
                wp.distill.as_deref().unwrap_or("auto"),
                wp.sensitive.unwrap_or(false),
                api_key_id,
                key_name_snapshot,
                wp.client_ref.as_deref(),
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
    pub(crate) async fn memory_append_session(
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

    /// 遗忘：用户说「别记住这段/把这事忘了」时使用，对任何会话都有效。
    ///
    /// mode="void"（默认）：该会话作废、原文保留可审计；若会话已蒸馏，其产出的
    /// 原子（active 与 superseded）会**级联归档**——检索与上下文包立即不再返回它们。
    /// mode="erase"：物理删除该会话及其派生原子的检索可见性（不可逆，需要 erase scope 的 key）。
    /// mode="restore"：撤销 void——误作废的后悔药，恢复会话与被归档的原子。
    /// 注意：只对用户明确表达的遗忘请求使用，不要自行判断「这段不重要」就遗忘。
    pub(crate) async fn memory_forget(
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
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_memory_sessions() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::memory_sessions_router()
}

//! server 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

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

// ---------- MCP 服务器 ----------

/// Engram MCP 服务器（用户记忆域 + Wiki 域）。工具实现进程内直调各域 service（不走 HTTP 回环）。
#[derive(Clone)]
pub struct EngramMcpServer {
    pub(crate) state: AppState,
    pub(crate) tool_router: ToolRouter<Self>,
}

impl EngramMcpServer {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            tool_router: Self::build_tool_router(),
        }
    }

    pub(crate) fn svc(&self) -> engram_core::memory::MemoryService {
        engram_core::memory::MemoryService::new(self.state.pool.clone(), self.state.registry())
    }

    pub(crate) fn svc_project(&self) -> engram_core::project::ProjectService {
        engram_core::project::ProjectService::new(self.state.pool.clone())
    }

    /// 资产台账域服务（2026-09-21 新增）。
    pub(crate) fn svc_asset(&self) -> engram_core::assets::AssetService {
        engram_core::assets::AssetService::new(self.state.pool.clone())
    }

    /// 位置登记的资产引用解析（资产 id / 台账名 / 别名三态）+ 身份字段带出。
    /// 传了 asset 且 ip/host/os 为空时用台账值补齐——唯一事实源：身份在台账，项目里只引用不重抄。
    pub(crate) async fn resolve_location_asset(
        &self,
        asset: Option<&str>,
        ip: Option<&str>,
        host: Option<&str>,
        os: Option<&str>,
    ) -> Result<(Option<Uuid>, String, String, String), rmcp::ErrorData> {
        let mut ip = ip.unwrap_or("").trim().to_string();
        let mut host = host.unwrap_or("").trim().to_string();
        let mut os = os.unwrap_or("").trim().to_string();
        let Some(key) = asset.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok((None, ip, host, os));
        };
        let a = match Uuid::parse_str(key) {
            Ok(id) => self.svc_asset().get(id).await.map_err(from_asset)?.asset,
            Err(_) => self
                .svc_asset()
                .get_by_name_or_alias(key)
                .await
                .map_err(from_asset)?,
        };
        if ip.is_empty() {
            ip = a.ip.clone();
        }
        if host.is_empty() {
            host = a.name.clone();
        }
        if os.is_empty() {
            os = a.os.clone();
        }
        Ok((Some(a.id), ip, host, os))
    }

    /// skills service（域服务构造器集中在此，便于测试替换与依赖收口）。
    pub(crate) fn skills_svc(&self) -> engram_core::skills::SkillsService {
        engram_core::skills::SkillsService::new(self.state.pool.clone())
    }

    /// 单库终局：main 主库 id（无外部入参——多库 API 已移除，2026-09-20）。
    pub(crate) async fn resolve_wiki_lib(&self) -> Result<Uuid, rmcp::ErrorData> {
        engram_core::wiki::libraries::resolve(&self.state.pool, None)
            .await
            .map_err(wiki::from_wiki)
    }

    /// project_id / project_name 二选一定位项目 id（项目名唯一，可寻址）。
    pub(crate) async fn resolve_project(
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
                    "search_all" => ["memory", "wiki", "todos", "project"]
                        .iter()
                        .any(|s| p.domain_access(s) != DomainAccess::None),
                    // jobs 无域 scope（对齐 HTTP：任何合法凭证可读任务——AI 轮询自己触发的任务）
                    "jobs" => true,
                    name => {
                        p.domain_access(tool_scope(name)) != DomainAccess::None
                            || (name == "memory"
                                && p.domain_access("original") != DomainAccess::None)
                    }
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
                // 动作级权限（公网多Agent P001 步骤3）：scope 的 :ro 只读变体只能调本域
                // 读类动作；拒绝报错列出可用只读动作（报错即文档）。缺 scope 的拒绝保持在
                // handler 内的 require_*（原语义不变）。
                let principal = principal_of(&context)?;
                dispatch::check_action_access(&principal, name, action)?;
            }
        }
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }
}

/// MCP 请求体上限（字节）。
///
/// 产物上传通道（codegraph db 经 JSON-RPC base64 直传）需要：db 上限 256MB × base64 膨胀 1.33
/// ≈ 341MB，留余量到 384MB。rmcp 默认仅 **4MB** —— 2026-09-21 活体实测：真实 24.7MB 产物
/// → 32MB body 被 **413 Payload Too Large / 连接重置** 掐断（push 通道实际不可用）。
/// api 路由上的 `DefaultBodyLimit` 同用本常量（单一事实源）。
pub const MAX_REQUEST_BODY_BYTES: usize = 384 * 1024 * 1024;

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
    // 大 body 放行（产物上传）：rmcp 默认 4MB 会把 codegraph 产物上传掐成 413/连接重置
    config.max_request_body_bytes = MAX_REQUEST_BODY_BYTES;
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

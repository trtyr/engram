//! codegraph 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

/// CodeGraph 桥（root 与 api 层同一约定：data_dir/codegraph）。
pub(crate) fn cg_svc(state: &AppState) -> engram_cg_bridge::CgBridge {
    engram_cg_bridge::CgBridge::new(state.pool.clone(), state.data_dir.join("codegraph"))
}

/// 项目寻址：名字优先（AI 友好），uuid 亦可。
pub(crate) async fn cg_resolve(
    state: &AppState,
    project: &str,
) -> Result<uuid::Uuid, rmcp::ErrorData> {
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

/// 产物上传（公网模型 P001-4）：客户端本机 CLI index 后推 codegraph.db+HEAD。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CgUploadParams {
    /// 项目名（不存在则新建 upload 型条目；repo 型拒绝——本机索引不归上传通道管）
    #[schemars(
        description = "项目名（不存在则新建 upload 型条目；已存在且为 upload 型则覆盖产物并更新 head；repo 型条目拒绝——本机索引不归上传通道管，请换名或先 delete）。"
    )]
    pub name: String,
    /// commit hash（7~40 位 hex；声明式新鲜度）
    #[schemars(
        description = "commit hash（7~40 位 hex，短/长 SHA 都收）——服务端不读代码，按此声明标注新鲜度。客户端本机 `git rev-parse HEAD` 取。"
    )]
    pub head: String,
    /// codegraph.db 内容（base64；原始 SQLite 二进制直接编码，不要压缩）
    #[schemars(
        description = "本机 .codegraph/codegraph.db 文件内容的 base64（原始二进制直接编码，不要压缩/文本化）；上限 256MB。"
    )]
    pub db_b64: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CgQueryParams {
    /// 项目名（codegraph_list 里的 name；也接受 id）
    #[schemars(description = "项目名（codegraph_list 返回的 name；也接受 uuid）。")]
    pub project: String,
    /// explore | search | node | callers | callees | impact
    #[schemars(
        description = "查询类型：search=搜符号, explore=区域符号大纲(默认不带源码), node=符号详情含源码, callers=谁调用它, callees=它调用谁, impact=改动影响面, full_graph=整张图原始 JSON（不需要 target）。"
    )]
    pub kind: String,
    /// 查询文本或符号名（full_graph 不需要）
    #[schemars(
        description = "查询文本（search/explore）或符号名（node/callers/callees/impact）；kind=full_graph 时不需要。注意 explore 按目录名或符号定位，不支持按单文件文件名查询。"
    )]
    pub target: Option<String>,
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

/// codegraph_list 动态段：项目清单（name·状态·规模）。
/// AI 连上即知道有哪些代码库可查、哪个 ready。
pub(crate) async fn codegraph_catalog(pool: &engram_storage::PgPool) -> Option<String> {
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
pub(crate) fn cg_root_from_env() -> std::path::PathBuf {
    // EN-47：数据根解析唯一收口（wiki-engine::data_root——env 优先，fallback ~/.engram/app 且不再静默）
    engram_wiki_engine::data_root().join("codegraph")
}

#[tool_router(router = codegraph_router)]
impl EngramMcpServer {
    // ---------- 代码图谱域工具（codegraph scope） ----------

    /// 列出代码图谱项目（注册状态、索引统计）。
    ///
    /// 何时用：想探索/查询某个代码库前，先看注册了哪些项目、哪个 ready；
    /// 没有 → codegraph_register 注册（本地路径或 git URL）再 codegraph_index。
    pub(crate) async fn codegraph_list(
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
    pub(crate) async fn codegraph_register(
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
    pub(crate) async fn codegraph_index(
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
    pub(crate) async fn codegraph_sync(
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
    /// callers / callees / impact 影响面 / full_graph 全图（EN-61）。
    ///
    /// 何时用：读陌生代码前先 search/explore；改代码前用 callers/impact 评估影响面；
    /// 深入一个函数用 node；要整张图的原始 JSON（导入到别处/全局统计）用 full_graph。
    pub(crate) async fn codegraph_query(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<CgQueryParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_codegraph(&p)?;
        // full_graph（EN-61）：整张图导出——不需要 target；bridge.full_graph 已存在，这里只是接入口
        if params.0.kind == "full_graph" {
            return self.full_graph_reply(&params.0.project).await;
        }
        if params
            .0
            .target
            .as_deref()
            .is_none_or(|t| t.trim().is_empty())
        {
            return Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                "target 不能为空——先用 kind=search 搜符号，再对具体符号做 callers/impact（kind=full_graph 除外，不需要 target）",
            ));
        }
        let kind = engram_cg_bridge::QueryKind::from_str_opt(&params.0.kind).ok_or_else(|| {
            mcp_err(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "未知查询类型 {}——explore/search/node/callers/callees/impact/full_graph",
                    params.0.kind
                ),
            )
        })?;
        self.query_and_freshness(&params.0, kind).await
    }

    /// 注销代码图谱项目（删除注册与索引；不可逆——本地路径项目的源码不动）。
    ///
    /// 何时用：项目已完结/注册错了。按 name 或 id 注销。
    pub(crate) async fn codegraph_delete(
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

    /// 产物上传（公网模型 P001-4）：客户端本机 codegraph CLI index 后，上传
    /// codegraph.db（base64）+ HEAD——服务端只存 + 声明式新鲜度（head/uploaded_at），
    /// 无代码、无 git 凭证。
    ///
    /// 何时用：本机开发仓库、服务端看不到路径（另一台机器/公网部署）时的接入通道。
    /// 查询侧照常：upload 型条目 path 即产物目录，explore/query 直接吃 db。
    pub(crate) async fn codegraph_upload(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<CgUploadParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_codegraph(&p)?;
        let dp = params.0;
        use base64::Engine as _;
        let db_bytes = base64::engine::general_purpose::STANDARD
            .decode(dp.db_b64.trim())
            .map_err(|e| {
                mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    format!("db_b64 不是合法 base64: {e}"),
                )
            })?;
        let proj = cg_svc(&self.state)
            .upload_artifact(&dp.name, &dp.head, &db_bytes)
            .await
            .map_err(from_cg)?;
        ok_json(serde_json::json!({
            "id": proj.id, "name": proj.name, "status": proj.status,
            "source_kind": proj.source_kind, "head": proj.head,
            "uploaded_at": proj.uploaded_at, "db_bytes": db_bytes.len(),
            "path": proj.path,
            "hint": "产物已就位，codegraph query/list 即查即用；重复 upload 同名覆盖产物并更新 head"
        }))
    }

    /// 失效条目对账（EN-48）：把「注册状态 ready 但索引产物已丢失 / 路径已不存在」的条目
    /// 落到 error，让 list 不再把幽灵条目冒充可用资产（此前它们会一直显示 ready）。
    ///
    /// 自愈（EN-48 残留）：「路径仍在、仅产物丢失」的条目自动入队重建 job（报告
    /// queued_rebuild 带 job_id，异步执行——稍后 codegraph_list 确认回到 ready）；
    /// 路径不存在的幽灵无法自愈，需人工重新注册或删除。
    ///
    /// 何时用：查询报「索引产物已丢失 / 项目路径不存在」而 codegraph list 仍显示 ready 时；
    /// 或迁移、重装、换机器后做一次体检。
    pub(crate) async fn codegraph_gc(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_codegraph(&p)?;
        let mut report = cg_svc(&self.state).gc().await.map_err(from_cg)?;
        // 自愈：产物丢失（路径仍在）的条目自动重建——入队失败不中断对账，报告里如实标注
        let mut queued = Vec::new();
        if let Some(items) = report["needs_rebuild"].as_array() {
            for item in items {
                let Some(id) = item["id"].as_str().and_then(|s| Uuid::parse_str(s).ok()) else {
                    continue;
                };
                let name = item["name"].clone();
                match engram_jobs::JobQueue::new(self.state.pool.clone())
                    .enqueue(
                        engram_jobs::JobTemplate::new("cg_index")
                            .with_payload(serde_json::json!({"project_id": id})),
                    )
                    .await
                {
                    Ok(job) => queued.push(serde_json::json!({
                        "id": id, "name": name, "job_id": job.id,
                    })),
                    Err(e) => queued.push(serde_json::json!({
                        "id": id, "name": name, "enqueue_error": e.to_string(),
                    })),
                }
            }
        }
        report["queued_rebuild"] = serde_json::json!(queued);
        ok_json(report)
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
    pub(crate) async fn codegraph_tool(
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
        let action = call.action.clone();
        match action.as_str() {
            "list" | "query" => self.codegraph_read_group(ctx, call).await,
            "register" | "index" | "sync" | "delete" | "upload" | "gc" => {
                self.codegraph_write_group(ctx, call).await
            }
            other => Err(dispatch::unknown_action("codegraph", other)),
        }
    }
    /// full_graph（EN-61）：整张图导出 + 新鲜度注入。
    async fn full_graph_reply(&self, project: &str) -> Result<CallToolResult, rmcp::ErrorData> {
        let id = cg_resolve(&self.state, project).await?;
        let mut v = cg_svc(&self.state).full_graph(id).await.map_err(from_cg)?;
        if let Ok(proj) = cg_svc(&self.state).get(id).await
            && proj.status == "ready"
        {
            v["_freshness"] = cg_svc(&self.state).freshness_for(&proj).await;
        }
        ok_json(v)
    }

    /// 查询 + 新鲜度注入（索引落后 HEAD 时显式提醒；仅 object 响应注入——search 返回数组不能带键）。
    async fn query_and_freshness(
        &self,
        params: &CgQueryParams,
        kind: engram_cg_bridge::QueryKind,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let id = cg_resolve(&self.state, &params.project).await?;
        let mut v = cg_svc(&self.state)
            .query(
                id,
                kind,
                params.target.as_deref().unwrap_or_default(),
                params.depth,
                params.include_source.unwrap_or(false),
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
    /// codegraph 读类动作分发（分组见 dispatch.rs 动作表）。
    async fn codegraph_read_group(
        &self,
        ctx: RequestContext<RoleServer>,
        call: dispatch::DomainCall,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        match call.action.as_str() {
            "list" => self.codegraph_list(ctx).await,
            "query" => {
                self.codegraph_query(
                    ctx,
                    Parameters(dispatch::from_args("codegraph", "query", call.args)?),
                )
                .await
            }
            other => Err(dispatch::unknown_action("codegraph", other)),
        }
    }

    /// codegraph 写类动作分发（分组见 dispatch.rs 动作表）。
    async fn codegraph_write_group(
        &self,
        ctx: RequestContext<RoleServer>,
        call: dispatch::DomainCall,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        match call.action.as_str() {
            "register" => {
                self.codegraph_register(
                    ctx,
                    Parameters(dispatch::from_args("codegraph", "register", call.args)?),
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
            "upload" => {
                self.codegraph_upload(
                    ctx,
                    Parameters(dispatch::from_args("codegraph", "upload", call.args)?),
                )
                .await
            }
            "gc" => self.codegraph_gc(ctx).await,
            other => Err(dispatch::unknown_action("codegraph", other)),
        }
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_codegraph() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::codegraph_router()
}

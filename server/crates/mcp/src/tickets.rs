//! tickets 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

/// 工单域 add 参数：与 TodoAddParams 同字段但**无 kind**（tickets.add 固定开工单）。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TicketAddParams {
    /// 一句话标题（必填）
    #[schemars(description = "一句话标题（必填，≤200 字）。")]
    pub title: String,
    /// 详情（可选 markdown）
    #[schemars(description = "详情（可选 markdown）。")]
    pub body: Option<String>,
    /// 工单严重度 P0-P3
    #[schemars(description = "可选：工单严重度 P0/P1/P2/P3。")]
    pub severity: Option<String>,
    /// 工单症状
    #[schemars(description = "工单症状/现象描述。")]
    pub symptom: Option<String>,
    /// 工单复现路径
    #[schemars(description = "工单复现路径。")]
    pub reproduce: Option<String>,
    /// 工单验收标准
    #[schemars(description = "工单验收标准。")]
    pub acceptance: Option<String>,
    /// 自由标签
    #[schemars(description = "自由标签。")]
    pub tags: Option<Vec<String>>,
    /// 截止时间（ISO8601，可选）
    #[schemars(description = "可选截止时间（ISO8601）。")]
    pub due_at: Option<String>,
    /// 相关项目名提示（纯文本备注，不绑定）
    #[schemars(description = "可选：相关项目名提示（纯文本备注，不绑定项目）。")]
    pub project_hint: Option<String>,
}

/// 工单域 list 参数：与 TodoListParams 同字段但**无 kind**（tickets.list 固定只回工单）。
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct TicketListParams {
    /// 状态过滤：open/confirmed/in_progress/resolved/verified/archived（缺省全部，open 优先展示）
    #[schemars(
        description = "可选状态过滤：open/confirmed/in_progress/resolved/verified/archived。缺省全部（open 优先）。"
    )]
    pub status: Option<String>,
    /// 工单严重度 P0-P3
    #[schemars(description = "可选：工单严重度 P0-P3。")]
    pub severity: Option<String>,
    /// 标签过滤
    #[schemars(description = "可选标签过滤。")]
    pub tag: Option<String>,
    /// 标题/正文子串
    #[schemars(description = "可选子串过滤（标题或正文）。")]
    pub q: Option<String>,
    /// 摘要模式（MCP 默认 true）：只返回 短号/标题/形态/状态/分级/关联计数
    #[schemars(
        description = "可选：摘要模式，默认 true——只返回 short_no/标题/形态/状态/分级/标签/关联计数（不含 body/symptom 等长字段）。brief=false 返回全量。"
    )]
    pub brief: Option<bool>,
}

#[tool_router(router = tickets_router)]
impl EngramMcpServer {
    /// 开工单（tickets 域）：kind 固定 ticket——结构化问题跟踪。
    pub(crate) async fn ticket_add(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TicketAddParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let tp = params.0;
        let dto = todo_svc(&self.state)
            .create(
                &tp.title,
                tp.body.as_deref().unwrap_or(""),
                "ticket", // 拆域锁定：tickets.add 固定开工单
                "",       // ticket 无 priority（分级用 severity）
                tp.severity.as_deref(),
                tp.symptom.as_deref().unwrap_or(""),
                tp.reproduce.as_deref().unwrap_or(""),
                tp.acceptance.as_deref().unwrap_or(""),
                tp.tags.as_deref().unwrap_or(&[]),
                match tp.due_at.as_deref() {
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

    /// 工单列表（open 优先；status/severity/tag/q 过滤；仅 kind=ticket）。
    pub(crate) async fn ticket_list(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TicketListParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let lp = params.0;
        let rows = todo_svc(&self.state)
            .list(
                lp.status.as_deref(),
                Some("ticket"), // 拆域锁定：tickets.list 只回工单（待办走 todos.list）
                None,           // ticket 无 priority（分级 severity 下方）
                lp.tag.as_deref(),
                lp.q.as_deref(),
                lp.severity.as_deref(),
                None,
                500, // 工单量级小，一次拉全
            )
            .await
            .map_err(from_todo)?;
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

    /// 工单域（todos scope）：结构化问题跟踪——severity/symptom/acceptance 结构化字段，
    /// 状态流转 confirmed/in_progress/resolved/verified（工单六态）。与待办同表不同心智
    /// （2026-09-18 拆域：todos 纯待办、tickets 纯工单，互不可见）。操作全景：action="help"。
    #[tool(
        name = "tickets",
        annotations(
            title = "工单域",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(crate) async fn tickets_tool(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(call): Parameters<dispatch::DomainCall>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        if call.action == "help" {
            let cfg = load_config(&self.state.pool).await;
            return ok_json(dispatch::render_manual("tickets", &cfg.disabled_tools));
        }
        let action = call.action.clone();
        match action.as_str() {
            "list" | "get" | "links" => self.tickets_read_group(ctx, call).await,
            "add" | "link" | "unlink" | "update" | "delete" => {
                self.tickets_write_group(ctx, call).await
            }
            other => Err(dispatch::unknown_action("tickets", other)),
        }
    }
    /// tickets 读类动作分发（分组见 dispatch.rs 动作表）。
    async fn tickets_read_group(
        &self,
        ctx: RequestContext<RoleServer>,
        call: dispatch::DomainCall,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        match call.action.as_str() {
            "list" => {
                self.ticket_list(
                    ctx,
                    Parameters(dispatch::from_args("tickets", "list", call.args)?),
                )
                .await
            }
            "get" => {
                self.todo_get(
                    ctx,
                    Parameters(dispatch::from_args("tickets", "get", call.args)?),
                )
                .await
            }
            "links" => {
                self.todo_links(
                    ctx,
                    Parameters(dispatch::from_args("tickets", "links", call.args)?),
                )
                .await
            }
            other => Err(dispatch::unknown_action("tickets", other)),
        }
    }

    /// tickets 写类动作分发（分组见 dispatch.rs 动作表）。
    async fn tickets_write_group(
        &self,
        ctx: RequestContext<RoleServer>,
        call: dispatch::DomainCall,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        match call.action.as_str() {
            "add" => {
                self.ticket_add(
                    ctx,
                    Parameters(dispatch::from_args("tickets", "add", call.args)?),
                )
                .await
            }
            "link" => {
                self.todo_link(
                    ctx,
                    Parameters(dispatch::from_args("tickets", "link", call.args)?),
                )
                .await
            }
            "unlink" => {
                self.todo_unlink(
                    ctx,
                    Parameters(dispatch::from_args("tickets", "unlink", call.args)?),
                )
                .await
            }
            "update" => {
                self.todo_update(
                    ctx,
                    Parameters(dispatch::from_args("tickets", "update", call.args)?),
                )
                .await
            }
            "delete" => {
                self.todo_delete(
                    ctx,
                    Parameters(dispatch::from_args("tickets", "delete", call.args)?),
                )
                .await
            }
            other => Err(dispatch::unknown_action("tickets", other)),
        }
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_tickets() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::tickets_router()
}

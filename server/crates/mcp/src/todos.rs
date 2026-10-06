//! todos 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

pub(crate) fn todo_svc(state: &AppState) -> engram_core::todos::TodoService {
    engram_core::todos::TodoService::new(state.pool.clone())
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
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct TodoListParams {
    /// 状态过滤，todo 态 open/done/archived
    #[schemars(description = "可选状态过滤：open/done/archived。缺省全部（open 优先）。")]
    pub status: Option<String>,
    /// low | normal | high
    #[schemars(description = "可选优先级过滤。")]
    pub priority: Option<String>,
    /// 逾期/今日过滤
    #[schemars(
        description = "可选 due 过滤：overdue=未完成且已过期（按到期升序）；today=今天到期。"
    )]
    pub due: Option<String>,
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
    /// 新标题（可选）
    #[schemars(description = "可选新标题。")]
    pub title: Option<String>,
    /// 新详情（可选）
    #[schemars(description = "可选新详情。")]
    pub body: Option<String>,
    /// low | normal | high
    #[schemars(description = "可选优先级：low/normal/high。")]
    pub priority: Option<String>,
    /// open | done | archived
    #[schemars(description = "可选状态：open/done/archived。")]
    pub status: Option<String>,
    /// 可选：标签整体替换（传即替换全量；不传不动——EN-237②）
    #[schemars(description = "可选：自由标签整体替换（传了就全量替换；不传保持不变）。")]
    pub tags: Option<Vec<String>>,
    /// 截止时间（ISO8601，可选）
    #[schemars(description = "可选截止时间（ISO8601）。")]
    pub due_at: Option<String>,
    /// 显式清除字段（白名单 due_at）
    #[schemars(
        description = "可选：显式清除字段清单（白名单 due_at，如 [\"due_at\"]）——清空该字段而非置之不理。"
    )]
    pub clear: Option<Vec<String>>,
}

/// clear 白名单判定（TodoUpdateParams.clear）
fn clears(field: &str, clear: &Option<Vec<String>>) -> bool {
    clear.as_ref().is_some_and(|v| v.iter().any(|f| f == field))
}

#[tool_router(router = todos_router)]
impl EngramMcpServer {
    // ---------- 待办域工具（todos scope） ----------

    /// 快速记一条待办（灵感/学习计划/系统操作——不绑定项目）。
    pub(crate) async fn todo_add(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TodoAddParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let tp = params.0;
        // 工单在 tickets 域（0074 项目绑定制）——本域只管行动项
        let dto = todo_svc(&self.state)
            .create(
                &tp.title,
                tp.body.as_deref().unwrap_or(""),
                tp.priority.as_deref().unwrap_or("normal"),
                tp.tags.as_deref().unwrap_or(&[]),
                match tp.due_at.as_deref() {
                    // D18：显式传了 due_at 就必须可解析（此前垃圾值被静默吞成 None，
                    // 调用方以为设置了截止时间实际没生效）
                    Some(s) => Some(parse_flex_datetime(s)?),
                    None => None,
                },
            )
            .await
            .map_err(from_todo)?;
        ok_json(slim_todo(
            serde_json::to_value(&dto).unwrap_or(serde_json::json!({})),
        ))
    }

    /// 待办列表（open 优先；status/priority/tag/q 过滤）。
    pub(crate) async fn todo_list(
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
                lp.priority.as_deref(),
                lp.tag.as_deref(),
                lp.q.as_deref(),
                lp.due.as_deref(),
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
                        "status": t.status,
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
                "total": todo_svc(&self.state)
                    .count(lp.status.as_deref(), lp.priority.as_deref(), lp.tag.as_deref(), lp.q.as_deref())
                    .await
                    .unwrap_or(-1),
                "items": brief,
                "hint": "摘要模式（body 已省略）——brief=false 取全量；引用条目用 EN-<短号>",
            }));
        }
        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
    }

    /// 待办详情。
    pub(crate) async fn todo_get(
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
    pub(crate) async fn todo_done(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TodoIdParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let id = self.todo_ref_id(&params.0.id).await?;
        let dto = todo_svc(&self.state)
            .update(id, None, None, None, Some("done"), None, None)
            .await
            .map_err(from_todo)?;
        ok_json(slim_todo(
            serde_json::to_value(&dto).unwrap_or(serde_json::json!({})),
        ))
    }

    /// 更新待办（标题/详情/优先级/状态，部分字段 None 不动）。
    pub(crate) async fn todo_update(
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
                params.0.title.as_deref(),
                params.0.body.as_deref(),
                params.0.priority.as_deref(),
                params.0.status.as_deref(),
                match params.0.due_at.as_deref() {
                    Some(s) => Some(Some(parse_flex_datetime(s)?)),
                    None if clears("due_at", &params.0.clear) => Some(None),
                    None => None,
                },
                params.0.tags.as_deref(),
            )
            .await
            .map_err(from_todo)?;
        ok_json(slim_todo(
            serde_json::to_value(&dto).unwrap_or(serde_json::json!({})),
        ))
    }

    /// 删除待办（物理删除；归档语义走 todo_update status=archived）。
    pub(crate) async fn todo_delete(
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

    /// 待办域（todos scope）：快速记录与跟进不绑定项目的行动项/灵感速记（kind 固定 todo）。
    /// 工单在独立 tickets 域（2026-09-18 拆域——同表不同心智）。"add" 秒记，"list" 看进行中，
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
    pub(crate) async fn todos_tool(
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
        let action = call.action.clone();
        match action.as_str() {
            "list" | "get" | "links" => self.todos_read_group(ctx, call).await,
            "add" | "link" | "unlink" | "done" | "update" | "delete" => {
                self.todos_write_group(ctx, call).await
            }
            other => Err(dispatch::unknown_action("todos", other)),
        }
    }
    /// todos 读类动作分发（分组见 dispatch.rs 动作表）。
    async fn todos_read_group(
        &self,
        ctx: RequestContext<RoleServer>,
        call: dispatch::DomainCall,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        match call.action.as_str() {
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
            other => Err(dispatch::unknown_action("todos", other)),
        }
    }

    /// todos 写类动作分发（分组见 dispatch.rs 动作表）。
    async fn todos_write_group(
        &self,
        ctx: RequestContext<RoleServer>,
        call: dispatch::DomainCall,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        match call.action.as_str() {
            "add" => {
                self.todo_add(
                    ctx,
                    Parameters(dispatch::from_args("todos", "add", call.args)?),
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
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_todos() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::todos_router()
}

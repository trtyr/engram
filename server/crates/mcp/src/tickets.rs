//! tickets 域 MCP 工具面（0074 项目绑定制）：工单必须绑定已有项目——
//! project 参数（项目 id 或**精确**项目名）解析不到直接拒绝，不自动建项目、不模糊匹配。

use super::*;

/// 工单 id（或 EN-短号）参数。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TicketIdParams {
    /// 工单 id 或 EN-短号
    #[schemars(description = "工单 id（tickets.list 返回）或 EN-<短号>。")]
    pub id: String,
}

/// 工单域 add 参数：**必须绑定项目**。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TicketAddParams {
    /// 项目（必填）：项目 id 或精确项目名
    #[schemars(
        description = "必填：工单挂在哪个项目下——项目 id 或精确项目名。解析不到直接拒绝（不会自动建项目）。"
    )]
    pub project: String,
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
}

/// 工单域 list 参数。
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
    /// 项目 id 或精确项目名（可选：只看该项目下的工单）
    #[schemars(description = "可选：按项目过滤——项目 id 或精确项目名，解析不到报错。")]
    pub project: Option<String>,
    /// 标题/正文/症状子串
    #[schemars(description = "可选子串过滤（标题、正文或症状）。")]
    pub q: Option<String>,
    /// 翻页游标（keyset：上一次 list 返回的 next_cursor）
    #[schemars(description = "可选翻页游标（keyset）：上一次 list 返回的 next_cursor。缺省从头。")]
    pub cursor: Option<String>,
    /// 条数上限（缺省 50，单页上限 500——更多结果用 cursor 翻页）
    #[schemars(description = "条数上限（缺省 50，单页上限 500——更多结果用 cursor 翻页）。")]
    pub limit: Option<i64>,
    /// 摘要模式（MCP 默认 true）：只返回 短号/标题/状态/分级/项目
    #[schemars(
        description = "可选：摘要模式，默认 true——只返回 short_no/标题/状态/分级/项目（不含 body/symptom 等长字段）。brief=false 返回全量。"
    )]
    pub brief: Option<bool>,
}

/// 工单 update 参数（resolution 必填语义：转 resolved/verified 前必须有解决记录）。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TicketUpdateParams {
    /// 工单 id 或 EN-短号
    #[schemars(description = "工单 id 或 EN-<短号>。")]
    pub id: String,
    /// 新标题（可选）
    #[schemars(description = "可选新标题。")]
    pub title: Option<String>,
    /// 新详情（可选）
    #[schemars(description = "可选新详情。")]
    pub body: Option<String>,
    /// open | confirmed | in_progress | resolved | verified | archived
    #[schemars(
        description = "可选状态：open/confirmed/in_progress/resolved/verified/archived。转 resolved/verified 必须带 resolution。"
    )]
    pub status: Option<String>,
    /// 工单严重度 P0-P3；null=显式清除（回到未定级）
    #[schemars(description = "可选：工单严重度 P0/P1/P2/P3；null=清除分级。")]
    #[serde(default, deserialize_with = "double_option")]
    pub severity: Option<Option<String>>,
    /// 工单症状
    #[schemars(description = "工单症状/现象描述。")]
    pub symptom: Option<String>,
    /// 工单复现路径
    #[schemars(description = "工单复现路径。")]
    pub reproduce: Option<String>,
    /// 工单验收标准
    #[schemars(description = "工单验收标准。")]
    pub acceptance: Option<String>,
    /// 工单解决记录（resolved 前必填）
    #[schemars(description = "工单解决记录——状态转 resolved 前必填（做了什么/怎么修的）。")]
    pub resolution: Option<String>,
}

/// serde 双层 Option：字段缺失→None（不动）；字段=null→Some(None)（显式清除）；字段=值→Some(Some(v))。
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<T>::deserialize(de)?))
}

fn ticket_svc(state: &AppState) -> engram_core::tickets::TicketService {
    engram_core::tickets::TicketService::new(state.pool.clone())
}

/// project 参数解析：UUID 直接认；否则精确项目名（不模糊匹配）。
/// 解析不到 → 明确报错（绝不自动建项目）。工单/凭据两个域共用（0075）。
pub(crate) async fn resolve_project(
    state: &AppState,
    project: &str,
) -> Result<Uuid, rmcp::ErrorData> {
    let p = project.trim();
    if let Ok(id) = Uuid::parse_str(p) {
        let exists = engram_storage::repo::project::existing_ids(&state.pool, &[id])
            .await
            .map_err(|e| mcp_err(ErrorCode::INTERNAL_ERROR, format!("存储暂时不可用: {e}")))?;
        if exists.is_empty() {
            return Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "项目 {id} 不存在——工单必须绑定已有项目；先 projects create 建项目，或检查 project id。"
                ),
            ));
        }
        return Ok(id);
    }
    let id = engram_storage::repo::project::id_by_name(&state.pool, p)
        .await
        .map_err(|e| mcp_err(ErrorCode::INTERNAL_ERROR, format!("存储暂时不可用: {e}")))?;
    id.ok_or_else(|| {
        mcp_err(
            ErrorCode::INVALID_PARAMS,
            format!("项目「{p}」不存在——工单必须绑定已有项目；先 projects create 建项目，或检查项目名（精确匹配，不做模糊）。"),
        )
    })
}

#[tool_router(router = tickets_router)]
impl EngramMcpServer {
    /// 开工单（tickets 域）：必须绑定已有项目——绑定不了就不进工单。
    pub(crate) async fn ticket_add(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TicketAddParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let tp = params.0;
        let project_id = resolve_project(&self.state, &tp.project).await?;
        let dto = ticket_svc(&self.state)
            .create(
                project_id,
                &tp.title,
                tp.body.as_deref().unwrap_or(""),
                tp.severity.as_deref(),
                tp.symptom.as_deref().unwrap_or(""),
                tp.reproduce.as_deref().unwrap_or(""),
                tp.acceptance.as_deref().unwrap_or(""),
            )
            .await
            .map_err(from_ticket)?;
        ok_json(serde_json::to_value(&dto).unwrap_or(serde_json::json!({})))
    }

    /// 工单列表（open 优先；status/severity/project/q 过滤）。
    pub(crate) async fn ticket_list(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TicketListParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let lp = params.0;
        let project_id = match lp.project.as_deref() {
            Some(pj) => Some(resolve_project(&self.state, pj).await?),
            None => None,
        };
        let (rows, total) = ticket_svc(&self.state)
            .list(
                lp.status.as_deref(),
                lp.severity.as_deref(),
                project_id,
                lp.q.as_deref(),
                lp.cursor.as_deref(),
                lp.limit.unwrap_or(50).min(500),
            )
            .await
            .map_err(from_ticket)?;
        if lp.brief.unwrap_or(true) {
            // 项目名速查表（brief 里带项目名，AI 不用二次查）
            let names: std::collections::HashMap<Uuid, String> =
                engram_storage::repo::project::list_projects(&self.state.pool, None)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|p| (p.id, p.name))
                    .collect();
            let brief: Vec<serde_json::Value> = rows
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "id": t.id,
                        "short_no": t.short_no,
                        "ref": format!("EN-{}", t.short_no),
                        "title": t.title,
                        "status": t.status,
                        "severity": t.severity,
                        "project": names.get(&t.project_id).cloned().unwrap_or_default(),
                        "project_id": t.project_id,
                        "body_omitted": true,
                    })
                })
                .collect();
            return ok_json(serde_json::json!({
                "brief": true,
                "count": brief.len(),
                "total": total,
                "items": brief,
                "hint": "摘要模式（body 已省略）——brief=false 取全量；引用条目用 EN-<短号>",
            }));
        }
        let v = serde_json::json!({ "items": rows, "total": total });
        ok_json(v)
    }

    /// 工单域（todos scope）：结构化问题跟踪——severity/symptom/acceptance/resolution 结构化字段，
    /// 六态状态机。**项目绑定制**：工单必须挂在已有项目下。操作全景：action="help"。
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
            "list" | "get" | "events" => self.tickets_read_group(ctx, call).await,
            "add" | "update" | "delete" | "comment" => self.tickets_write_group(ctx, call).await,
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
                self.ticket_get(
                    ctx,
                    Parameters(dispatch::from_args("tickets", "get", call.args)?),
                )
                .await
            }
            "events" => {
                self.ticket_events(
                    ctx,
                    Parameters(dispatch::from_args("tickets", "events", call.args)?),
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
            "update" => {
                self.ticket_update(
                    ctx,
                    Parameters(dispatch::from_args("tickets", "update", call.args)?),
                )
                .await
            }
            "delete" => {
                self.ticket_delete(
                    ctx,
                    Parameters(dispatch::from_args("tickets", "delete", call.args)?),
                )
                .await
            }
            "comment" => {
                self.ticket_comment(
                    ctx,
                    Parameters(dispatch::from_args("tickets", "comment", call.args)?),
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

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TicketEventsParams {
    /// 工单 id 或 EN-短号
    #[schemars(
        description = "工单 id 或 EN-短号。返回活动时间线（状态流转 event + 评论 comment，升序）。"
    )]
    pub id: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TicketCommentParams {
    /// 工单 id 或 EN-短号
    #[schemars(description = "工单 id 或 EN-短号。评论将入活动时间线。")]
    pub id: String,
    /// 评论正文
    #[schemars(description = "评论正文（trim 后非空）。")]
    pub text: String,
}

// 时间线处理器（读：events / 写：comment）
impl EngramMcpServer {
    /// 工单详情（UUID 或 EN-短号）。
    pub(crate) async fn ticket_get(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TicketIdParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let dto = ticket_svc(&self.state)
            .find_by_ref(&params.0.id)
            .await
            .map_err(from_ticket)?
            .ok_or_else(|| {
                mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    format!("工单不存在: {}", params.0.id),
                )
            })?;
        ok_json(serde_json::to_value(&dto).unwrap_or(serde_json::json!({})))
    }

    pub(crate) async fn ticket_update(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TicketUpdateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let tp = params.0;
        let id = ticket_svc(&self.state)
            .find_by_ref(&tp.id)
            .await
            .map_err(from_ticket)?
            .ok_or_else(|| mcp_err(ErrorCode::INVALID_PARAMS, format!("工单不存在: {}", tp.id)))?
            .id;
        let dto = ticket_svc(&self.state)
            .update(
                id,
                tp.title.as_deref(),
                tp.body.as_deref(),
                tp.status.as_deref(),
                tp.severity.as_ref().map(|o| o.as_deref()),
                tp.symptom.as_deref(),
                tp.reproduce.as_deref(),
                tp.acceptance.as_deref(),
                tp.resolution.as_deref(),
                "mcp:tickets",
            )
            .await
            .map_err(from_ticket)?;
        ok_json(serde_json::to_value(&dto).unwrap_or(serde_json::json!({})))
    }

    /// 删除工单（物理删除；归档语义走 status=archived）。
    pub(crate) async fn ticket_delete(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TicketIdParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let id = ticket_svc(&self.state)
            .find_by_ref(&params.0.id)
            .await
            .map_err(from_ticket)?
            .ok_or_else(|| {
                mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    format!("工单不存在: {}", params.0.id),
                )
            })?
            .id;
        ticket_svc(&self.state)
            .delete(id)
            .await
            .map_err(from_ticket)?;
        ok_json(serde_json::json!({ "deleted": id.to_string() }))
    }

    pub(crate) async fn ticket_events(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TicketEventsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let id = ticket_svc(&self.state)
            .find_by_ref(&params.0.id)
            .await
            .map_err(from_ticket)?
            .ok_or_else(|| {
                mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    format!("工单不存在: {}", params.0.id),
                )
            })?
            .id;
        let rows = ticket_svc(&self.state)
            .events(id)
            .await
            .map_err(from_ticket)?;
        ok_json(serde_json::json!({
            "ticket_id": id.to_string(),
            "count": rows.len(),
            "events": rows,
        }))
    }

    pub(crate) async fn ticket_comment(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TicketCommentParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let text = params.0.text.trim();
        if text.is_empty() {
            return Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                "评论不能为空".to_string(),
            ));
        }
        let id = ticket_svc(&self.state)
            .find_by_ref(&params.0.id)
            .await
            .map_err(from_ticket)?
            .ok_or_else(|| {
                mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    format!("工单不存在: {}", params.0.id),
                )
            })?
            .id;
        ticket_svc(&self.state)
            .event_add(
                id,
                "comment",
                &serde_json::json!({ "text": text }),
                "mcp:tickets",
            )
            .await
            .map_err(from_ticket)?;
        ok_json(serde_json::json!({ "commented": true }))
    }
}

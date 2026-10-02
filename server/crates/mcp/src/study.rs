//! study 域 MCP 工具面（P007-T004）：学习过程的路线图状态机。
//!
//! 分工：study 只存「学没学/学到哪/下一步学啥」的过程状态；
//! 知识内容归 wiki，原文归 documents，感悟叙事归 memory。

use super::*;

fn study_svc(state: &AppState) -> engram_core::study::StudyService {
    engram_core::study::StudyService::new(state.pool.clone())
}

/// StudyError → MCP 错误码（与 wiki::from_wiki 同语义）。
pub fn from_study(e: engram_core::study::StudyError) -> rmcp::ErrorData {
    use engram_core::study::StudyError;
    match e {
        StudyError::NotFound(m) => rmcp::ErrorData::resource_not_found(m, None),
        StudyError::BadRequest(m) => rmcp::ErrorData::invalid_params(m, None),
        StudyError::Storage(m) => rmcp::ErrorData::internal_error(m, None),
    }
}

fn require_study(principal: &Principal) -> Result<(), rmcp::ErrorData> {
    match principal.domain_access("study") {
        engram_core::auth::DomainAccess::None => Err(mcp_err(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            "缺少 study scope——请用带 study scope 的 amk_ key 连接 MCP",
        )),
        engram_core::auth::DomainAccess::ReadOnly => Err(mcp_err(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            "study scope 为只读（:ro）——写操作需要完整 study scope",
        )),
        DomainAccess::Full => Ok(()),
    }
}

fn require_study_read(principal: &Principal) -> Result<(), rmcp::ErrorData> {
    match principal.domain_access("study") {
        engram_core::auth::DomainAccess::None => Err(mcp_err(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            "缺少 study scope——请用带 study scope 的 amk_ key 连接 MCP",
        )),
        _ => Ok(()),
    }
}

// ---------- 工具参数 ----------

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StudyAddParams {
    /// 领域名（如「RAG 入门」）
    #[schemars(description = "领域名（如「RAG 入门」）。")]
    pub name: String,
    /// 目标（学到什么程度算完——归档锚）
    #[schemars(description = "可选：目标（学到什么程度算完，归档锚）。")]
    pub goal: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StudyGetParams {
    /// topic id（add/list 返回）
    #[schemars(description = "topic id（add/list 返回）。")]
    pub id: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StudyListParams {}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StudyTopicUpdateParams {
    /// topic id
    #[schemars(description = "topic id。")]
    pub id: String,
    /// 新领域名（不传不动）
    #[schemars(description = "可选：新领域名。")]
    pub name: Option<String>,
    /// 新目标（不传不动）
    #[schemars(description = "可选：新目标（学到什么程度算完）。")]
    pub goal: Option<String>,
    /// active | paused | done（done=归档，产出物留 wiki）
    #[schemars(description = "可选状态：active/paused/done（done=归档，数据保留，wiki 产出物不受影响）。")]
    pub status: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StudyItemAddParams {
    /// topic id
    #[schemars(description = "topic id。")]
    pub topic_id: String,
    /// 知识点名（如「语义切分」）
    #[schemars(description = "知识点名（如「语义切分」）。")]
    pub name: String,
    /// 路线图顺序（缺省排尾部）
    #[schemars(description = "可选：路线图顺序 position（缺省排尾部）。")]
    pub position: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StudyUnitSetParams {
    /// item id
    #[schemars(description = "item id。")]
    pub item_id: String,
    /// not_started | learning | learned（learned 记 learned_at；允许回退）
    #[schemars(description = "状态：not_started/learning/learned（learned 记时间戳；允许回退——学习本就反复）。")]
    pub status: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StudyItemLinkParams {
    /// item id
    #[schemars(description = "item id。")]
    pub item_id: String,
    /// 关联 wiki 页 slug 列表
    #[schemars(description = "可选：关联 wiki 页 slug 列表（覆盖式更新）。")]
    pub wiki_slugs: Option<Vec<String>>,
    /// 关联文档 id 列表
    #[schemars(description = "可选：关联文档 id 列表（覆盖式更新）。")]
    pub doc_ids: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StudyTopicDeleteParams {
    /// topic id（级联删全部节点）
    #[schemars(description = "topic id（级联删全部节点，不可恢复）。")]
    pub id: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StudyItemSetReviewParams {
    /// item id
    #[schemars(description = "item id。")]
    pub item_id: String,
    /// 是否需要复习（learned 后想保持记忆就开）
    #[schemars(description = "是否需要复习（learned 后想保持记忆就开；关掉即移出复习队列）。")]
    pub needs_review: bool,
    /// 到期时间（RFC3339；缺省=立即到期）
    #[schemars(description = "可选：复习到期时间 RFC3339（如 2026-10-09T00:00:00Z）；缺省=立即到期。")]
    pub review_due_at: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StudyReviewsDueParams {}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StudyJournalAddParams {
    /// topic id
    #[schemars(description = "topic id。")]
    pub topic_id: String,
    /// 进度备注（学了什么/卡在哪/下一步）
    #[schemars(description = "进度备注（学了什么/卡在哪/下一步）。")]
    pub note: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StudyJournalListParams {
    /// topic id
    #[schemars(description = "topic id。")]
    pub topic_id: String,
    /// 条数上限（默认 20）
    #[schemars(description = "可选：条数上限（默认 20，最大 200）。")]
    pub limit: Option<i64>,
}

// ---------- 工具面 ----------

#[tool_router(router = study_router)]
impl EngramMcpServer {
    /// Study 域（单一入口）：学习路线图跟踪——领域 track → 知识单元状态机（待学/进行中/已学）→ 挂 wiki 页。
    /// 只管「学没学/学到哪/下一步学啥」的过程状态；知识内容归 wiki，原文归 documents。
    /// 核心用法：study get{id} 一次拿全【进度+下一步队列+资料清单】——跨会话恢复学习上下文。
    #[tool(
        name = "study",
        annotations(
            title = "Study 域",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(crate) async fn study_tool(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(call): Parameters<dispatch::DomainCall>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_study(&p)?;
        if call.action == "help" {
            let cfg = load_config(&self.state.pool).await;
            return ok_json(dispatch::render_manual("study", &cfg.disabled_tools));
        }
        match call.action.as_str() {
            "add" => {
                self.study_add(
                    ctx,
                    Parameters(dispatch::from_args("study", "add", call.args)?),
                )
                .await
            }
            "get" => {
                self.study_get(
                    ctx,
                    Parameters(dispatch::from_args("study", "get", call.args)?),
                )
                .await
            }
            "list" => {
                self.study_list(ctx, Parameters(dispatch::from_args("study", "list", call.args)?))
                    .await
            }
            "topic_update" => {
                self.study_topic_update(
                    ctx,
                    Parameters(dispatch::from_args("study", "topic_update", call.args)?),
                )
                .await
            }
            "topic_delete" => {
                self.study_topic_delete(
                    ctx,
                    Parameters(dispatch::from_args("study", "topic_delete", call.args)?),
                )
                .await
            }
            "item_add" => {
                self.study_item_add(
                    ctx,
                    Parameters(dispatch::from_args("study", "item_add", call.args)?),
                )
                .await
            }
            "unit_set" => {
                self.study_unit_set(
                    ctx,
                    Parameters(dispatch::from_args("study", "unit_set", call.args)?),
                )
                .await
            }
            "item_link" => {
                self.study_item_link(
                    ctx,
                    Parameters(dispatch::from_args("study", "item_link", call.args)?),
                )
                .await
            }
            "item_set_review" => {
                self.study_item_set_review(
                    ctx,
                    Parameters(dispatch::from_args("study", "item_set_review", call.args)?),
                )
                .await
            }
            "reviews_due" => {
                self.study_reviews_due(
                    ctx,
                    Parameters(dispatch::from_args("study", "reviews_due", call.args)?),
                )
                .await
            }
            "journal_add" => {
                self.study_journal_add(
                    ctx,
                    Parameters(dispatch::from_args("study", "journal_add", call.args)?),
                )
                .await
            }
            "journal_list" => {
                self.study_journal_list(
                    ctx,
                    Parameters(dispatch::from_args("study", "journal_list", call.args)?),
                )
                .await
            }
            other => Err(dispatch::unknown_action("study", other)),
        }
    }

    /// 开题（新学习领域）。
    pub(crate) async fn study_add(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(dp): Parameters<StudyAddParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_study(&p)?;
        let id = study_svc(&self.state)
            .topic_create(&dp.name, dp.goal.as_deref().unwrap_or(""))
            .await
            .map_err(from_study)?;
        ok_json(serde_json::json!({
            "id": id,
            "name": dp.name.trim(),
            "hint": "开题成功——item_add 加知识点，unit_set 勾状态；get{id} 查看全景",
        }))
    }

    /// topic_get 全量（核心契约：一次拿全【进度+下一步队列+资料清单】）。
    pub(crate) async fn study_get(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(gp): Parameters<StudyGetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_study_read(&p)?;
        let id = uuid::Uuid::parse_str(&gp.id)
            .map_err(|_| rmcp::ErrorData::invalid_params(format!("id 非法: {}", gp.id), None))?;
        let full = study_svc(&self.state)
            .topic_get(id)
            .await
            .map_err(from_study)?;
        match full {
            Some(f) => ok_json(serde_json::to_value(&f).unwrap_or(serde_json::json!({}))),
            None => Err(rmcp::ErrorData::resource_not_found(
                format!("topic {} 不存在", gp.id),
                None,
            )),
        }
    }

    /// 全量 topics 简报。
    pub(crate) async fn study_list(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(_lp): Parameters<StudyListParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_study_read(&p)?;
        let rows = study_svc(&self.state).topic_list().await.map_err(from_study)?;
        ok_json(serde_json::json!({ "topics": rows, "count": rows.len() }))
    }

    /// 补丁式更新 topic（name/goal/status）。
    pub(crate) async fn study_topic_update(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(tp): Parameters<StudyTopicUpdateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_study(&p)?;
        let id = uuid::Uuid::parse_str(&tp.id)
            .map_err(|_| rmcp::ErrorData::invalid_params(format!("id 非法: {}", tp.id), None))?;
        study_svc(&self.state)
            .topic_update(id, tp.name.as_deref(), tp.goal.as_deref(), tp.status.as_deref())
            .await
            .map_err(from_study)?;
        ok_json(serde_json::json!({ "id": tp.id, "ok": true }))
    }

    /// 删 topic（级联 items）。
    pub(crate) async fn study_topic_delete(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(tp): Parameters<StudyTopicDeleteParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_study(&p)?;
        let id = uuid::Uuid::parse_str(&tp.id)
            .map_err(|_| rmcp::ErrorData::invalid_params(format!("id 非法: {}", tp.id), None))?;
        study_svc(&self.state).topic_delete(id).await.map_err(from_study)?;
        ok_json(serde_json::json!({ "id": tp.id, "deleted": true }))
    }

    /// 加知识点。
    pub(crate) async fn study_item_add(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(ip): Parameters<StudyItemAddParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_study(&p)?;
        let topic_id = uuid::Uuid::parse_str(&ip.topic_id).map_err(|_| {
            rmcp::ErrorData::invalid_params(format!("topic_id 非法: {}", ip.topic_id), None)
        })?;
        let id = study_svc(&self.state)
            .item_add(topic_id, &ip.name, ip.position.map(|p| p as i32))
            .await
            .map_err(from_study)?;
        ok_json(serde_json::json!({ "id": id, "name": ip.name.trim(), "ok": true }))
    }

    /// 知识点状态机。
    pub(crate) async fn study_unit_set(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(up): Parameters<StudyUnitSetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_study(&p)?;
        let item_id = uuid::Uuid::parse_str(&up.item_id).map_err(|_| {
            rmcp::ErrorData::invalid_params(format!("item_id 非法: {}", up.item_id), None)
        })?;
        study_svc(&self.state)
            .item_set_status(item_id, &up.status)
            .await
            .map_err(from_study)?;
        ok_json(serde_json::json!({ "item_id": up.item_id, "status": up.status, "ok": true }))
    }

    /// 知识点挂资料。
    pub(crate) async fn study_item_link(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(lp): Parameters<StudyItemLinkParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_study(&p)?;
        let item_id = uuid::Uuid::parse_str(&lp.item_id).map_err(|_| {
            rmcp::ErrorData::invalid_params(format!("item_id 非法: {}", lp.item_id), None)
        })?;
        study_svc(&self.state)
            .item_link(item_id, lp.wiki_slugs.clone(), lp.doc_ids.clone())
            .await
            .map_err(from_study)?;
        ok_json(serde_json::json!({ "item_id": lp.item_id, "ok": true }))
    }

    /// SRS 复习标记（P007 二期 T011）。
    pub(crate) async fn study_item_set_review(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(rp): Parameters<StudyItemSetReviewParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_study(&p)?;
        let item_id = uuid::Uuid::parse_str(&rp.item_id).map_err(|_| {
            rmcp::ErrorData::invalid_params(format!("item_id 非法: {}", rp.item_id), None)
        })?;
        let due = match &rp.review_due_at {
            Some(raw) => Some(
                chrono::DateTime::parse_from_rfc3339(raw)
                    .map_err(|e| {
                        rmcp::ErrorData::invalid_params(format!("review_due_at 非法: {e}"), None)
                    })?
                    .with_timezone(&chrono::Utc),
            ),
            None => None,
        };
        study_svc(&self.state)
            .item_set_review(item_id, rp.needs_review, due)
            .await
            .map_err(from_study)?;
        ok_json(serde_json::json!({ "item_id": rp.item_id, "needs_review": rp.needs_review, "ok": true }))
    }

    /// 复习队列（已标记且到期）。
    pub(crate) async fn study_reviews_due(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(_rp): Parameters<StudyReviewsDueParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_study_read(&p)?;
        let rows = study_svc(&self.state).reviews_due().await.map_err(from_study)?;
        ok_json(serde_json::json!({ "reviews": rows, "count": rows.len() }))
    }

    /// journal 进度时间线：记一笔（P007 二期 T012）。
    pub(crate) async fn study_journal_add(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(jp): Parameters<StudyJournalAddParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_study(&p)?;
        let topic_id = uuid::Uuid::parse_str(&jp.topic_id).map_err(|_| {
            rmcp::ErrorData::invalid_params(format!("topic_id 非法: {}", jp.topic_id), None)
        })?;
        let id = study_svc(&self.state)
            .journal_add(topic_id, &jp.note)
            .await
            .map_err(from_study)?;
        ok_json(serde_json::json!({ "id": id, "topic_id": jp.topic_id, "ok": true }))
    }

    /// journal 进度时间线：查最近（新→旧）。
    pub(crate) async fn study_journal_list(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(jp): Parameters<StudyJournalListParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_study_read(&p)?;
        let topic_id = uuid::Uuid::parse_str(&jp.topic_id).map_err(|_| {
            rmcp::ErrorData::invalid_params(format!("topic_id 非法: {}", jp.topic_id), None)
        })?;
        let rows = study_svc(&self.state)
            .journal_list(topic_id, jp.limit.unwrap_or(20))
            .await
            .map_err(from_study)?;
        ok_json(serde_json::json!({ "journal": rows, "count": rows.len() }))
    }
}

pub(crate) fn routes_study() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::study_router()
}

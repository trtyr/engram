//! projects 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

// ---------- 项目记忆工具参数 ----------

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectTypesParams {}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectListParams {
    /// 可选：按类型（场景）过滤：dev=开发 / ops=运维 / research=调研 / study=学习 / life=生活 / create=创作
    #[schemars(
        description = "可选：按类型（场景）过滤。dev=开发, ops=运维, research=调研, study=学习, life=生活, create=创作。"
    )]
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
    /// 场景：dev=开发 / ops=运维 / research=调研 / study=学习 / life=生活 / create=创作
    #[schemars(
        description = "项目**场景**（决定初始文档分类，建后可自由增删）：dev=开发（后端/前端/测试/部署/规划）、ops=运维（台账/部署/网络/备份/故障/巡检）、research=调研（待查/线索/资料/结论/疑点/证伪）、study=学习（大纲/笔记/练习/资源/进度）、life=生活（计划/清单/记录/花销/参考）、create=创作（选题/草稿/素材/成稿/发布）。不确定就先跑 projects types 看模板。"
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

/// project_list 动态段：枚举项目（name/类型/状态/描述）。
/// project_get / project_doc_* 都按项目名寻址——清单进描述可省一次列表往返。
pub(crate) async fn projects_catalog(pool: &engram_storage::PgPool) -> Option<String> {
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
        "【当前项目 {} 个】（projects 的 action=\"get\" / action=\"doc_get\" 支持按 name 寻址）\n{}",
        rows.len(),
        lines.join("\n")
    ))
}

#[tool_router(router = projects_router)]
impl EngramMcpServer {
    // ---------- 项目记忆工具（跨会话工作线：项目 / 位置 / 分类文档） ----------

    /// 列出项目类型模板（建项目选类型用）。
    ///
    /// 何时用：project_create 前不知道给什么 type 时。返回 dev（开发，预置 后端/前端/测试/规划）
    /// 与 research（调研，预置 待查/线索/资料/结论/疑点/证伪）两类及其默认分类。
    pub(crate) async fn project_types(
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
    pub(crate) async fn project_list(
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
    /// 不可失败（架构治理 task-5 分类 A：不可失败，保留并注明理由）。
    #[allow(clippy::expect_used)]
    pub(crate) async fn project_get(
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
    pub(crate) async fn project_create(
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
    pub(crate) async fn project_update(
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
    pub(crate) async fn project_delete(
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
    pub(crate) async fn project_batch_delete(
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

    /// 项目记忆域（单一入口）：项目 = 一件有明确目标、跨会话推进的工作，
    /// 下挂多主机位置（登记制）与「分类 > 文档」树。开工 "list"/"get" 接上下文，
    /// 干活中 "doc_add"/"doc_update" 沉淀，收尾 "update" 改状态。
    /// 速记：所有 doc_* 操作需先定位项目（参数 project_id 或 project_name，先 "list"）。操作全景：action="help"。
    #[tool(
        name = "projects",
        annotations(
            title = "项目记忆域",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(crate) async fn projects_tool(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(call): Parameters<dispatch::DomainCall>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        if call.action == "help" {
            let cfg = load_config(&self.state.pool).await;
            return ok_json(dispatch::render_manual("projects", &cfg.disabled_tools));
        }
        match call.action.as_str() {
            "types" => {
                self.project_types(
                    ctx,
                    Parameters(dispatch::from_args("projects", "types", call.args)?),
                )
                .await
            }
            "list" => {
                self.project_list(
                    ctx,
                    Parameters(dispatch::from_args("projects", "list", call.args)?),
                )
                .await
            }
            "get" => {
                self.project_get(
                    ctx,
                    Parameters(dispatch::from_args("projects", "get", call.args)?),
                )
                .await
            }
            "create" => {
                self.project_create(
                    ctx,
                    Parameters(dispatch::from_args("projects", "create", call.args)?),
                )
                .await
            }
            "update" => {
                self.project_update(
                    ctx,
                    Parameters(dispatch::from_args("projects", "update", call.args)?),
                )
                .await
            }
            "delete" => {
                self.project_delete(
                    ctx,
                    Parameters(dispatch::from_args("projects", "delete", call.args)?),
                )
                .await
            }
            "batch_delete" => {
                self.project_batch_delete(
                    ctx,
                    Parameters(dispatch::from_args("projects", "batch_delete", call.args)?),
                )
                .await
            }
            "location_add" => {
                self.project_location_add(
                    ctx,
                    Parameters(dispatch::from_args("projects", "location_add", call.args)?),
                )
                .await
            }
            "location_update" => {
                self.project_location_update(
                    ctx,
                    Parameters(dispatch::from_args(
                        "projects",
                        "location_update",
                        call.args,
                    )?),
                )
                .await
            }
            "location_delete" => {
                self.project_location_delete(
                    ctx,
                    Parameters(dispatch::from_args(
                        "projects",
                        "location_delete",
                        call.args,
                    )?),
                )
                .await
            }
            "doc_add" => {
                self.project_doc_add(
                    ctx,
                    Parameters(dispatch::from_args("projects", "doc_add", call.args)?),
                )
                .await
            }
            "doc_get" => {
                self.project_doc_get(
                    ctx,
                    Parameters(dispatch::from_args("projects", "doc_get", call.args)?),
                )
                .await
            }
            "doc_search" => {
                self.project_doc_search(
                    ctx,
                    Parameters(dispatch::from_args("projects", "doc_search", call.args)?),
                )
                .await
            }
            "doc_update" => {
                self.project_doc_update(
                    ctx,
                    Parameters(dispatch::from_args("projects", "doc_update", call.args)?),
                )
                .await
            }
            "doc_patch" => {
                self.project_doc_patch(
                    ctx,
                    Parameters(dispatch::from_args("projects", "doc_patch", call.args)?),
                )
                .await
            }
            "doc_delete" => {
                self.project_doc_delete(
                    ctx,
                    Parameters(dispatch::from_args("projects", "doc_delete", call.args)?),
                )
                .await
            }
            "file_put" => {
                self.project_file_upsert(
                    ctx,
                    Parameters(dispatch::from_args("projects", "file_put", call.args)?),
                )
                .await
            }
            "file_get" => {
                self.project_file_get(
                    ctx,
                    Parameters(dispatch::from_args("projects", "file_get", call.args)?),
                )
                .await
            }
            "file_list" => {
                self.project_file_list(
                    ctx,
                    Parameters(dispatch::from_args("projects", "file_list", call.args)?),
                )
                .await
            }
            "file_delete" => {
                self.project_file_delete(
                    ctx,
                    Parameters(dispatch::from_args("projects", "file_delete", call.args)?),
                )
                .await
            }
            "link" => {
                self.project_link_add(
                    ctx,
                    Parameters(dispatch::from_args("projects", "link", call.args)?),
                )
                .await
            }
            "unlink" => {
                self.project_link_delete(
                    ctx,
                    Parameters(dispatch::from_args("projects", "unlink", call.args)?),
                )
                .await
            }
            "links" => {
                self.project_links(
                    ctx,
                    Parameters(dispatch::from_args("projects", "links", call.args)?),
                )
                .await
            }
            other => Err(dispatch::unknown_action("projects", other)),
        }
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_projects() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::projects_router()
}

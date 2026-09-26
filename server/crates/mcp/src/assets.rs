//! assets 域 MCP 工具面（资产台账：主机 / 云实例 / 域名 / 账号 / 设备）。
//!
//! 2026-09-21 用户拍板独立成域（见《项目与资产模型 · README》§2）：资产是「我拥有的、
//! 可以被操作的东西」——身份唯一、无收尾、被多个项目引用；项目侧只引用（`project_locations.asset_id`）。
//! 本模块只做「语义 → core::assets 服务」的映射，持久化在 `engram_storage::repo::asset`。

use super::*;

// ---------- 参数 ----------

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct AssetKindsParams {}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct AssetListParams {
    /// 可选：按类型过滤
    #[schemars(
        description = "可选：按类型过滤。host=主机 / cloud=云实例 / domain=域名 / account=账号 / device=设备 / other=其他。"
    )]
    pub kind: Option<String>,
    /// 可选：检索词
    #[schemars(description = "可选：检索词——命中 名称 / 别名 / IP（大小写不敏感）。")]
    pub q: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct AssetGetParams {
    /// 资产 id
    #[schemars(description = "资产 id（UUID，来自 list / 建档返回）。与 name 至少给一个。")]
    pub asset_id: Option<String>,
    /// 资产名或别名
    #[schemars(
        description = "资产名或**别名**（如 trtyr-mac / tencent-beijing——历史写法都算）。与 asset_id 至少给一个。"
    )]
    pub name: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct AssetRunbookParams {
    /// 资产 id
    #[schemars(description = "资产 id（UUID）。与 name 至少给一个。")]
    pub asset_id: Option<String>,
    /// 资产名或别名
    #[schemars(description = "资产名或别名。与 asset_id 至少给一个。")]
    pub name: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct AssetRunbookSaveParams {
    /// 资产 id
    #[schemars(description = "资产 id（UUID）。与 name 至少给一个。")]
    pub asset_id: Option<String>,
    /// 资产名或别名
    #[schemars(description = "资产名或别名。与 asset_id 至少给一个。")]
    pub name: Option<String>,
    /// Markdown 全文（整体替换；旧文自动入修订史）
    #[schemars(
        description = "运行手册 Markdown 全文（整体替换保存；保存前旧文自动入修订史——错改可回滚）。"
    )]
    pub md: String,
    /// 编辑人标识（可选，留痕用）
    #[schemars(description = "编辑人标识（可选，留痕用；缺省 mcp:assets）。")]
    pub editor: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct AssetRunbookVersionsParams {
    /// 资产 id
    #[schemars(description = "资产 id（UUID）。与 name 至少给一个。")]
    pub asset_id: Option<String>,
    /// 资产名或别名
    #[schemars(description = "资产名或别名。与 asset_id 至少给一个。")]
    pub name: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct AssetRunbookRestoreParams {
    /// 资产 id
    #[schemars(description = "资产 id（UUID）。与 name 至少给一个。")]
    pub asset_id: Option<String>,
    /// 资产名或别名
    #[schemars(description = "资产名或别名。与 asset_id 至少给一个。")]
    pub name: Option<String>,
    /// 回滚目标修订 id
    #[schemars(description = "回滚目标修订 id（runbook_versions 列表里的 id）。")]
    pub version_id: String,
    /// 编辑人标识（可选，留痕用）
    #[schemars(description = "编辑人标识（可选；缺省 mcp:assets）。")]
    pub editor: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct AssetAddParams {
    /// 资产类型
    #[schemars(
        description = "资产类型：host=主机 / cloud=云实例 / domain=域名 / account=账号 / device=设备 / other=其他。不确定就先跑 kinds。"
    )]
    pub kind: String,
    /// 台账名
    #[schemars(
        description = "台账名（唯一；用人能认的名字，如「MacBook Air M1」「腾讯云 · 北京」）。"
    )]
    pub name: String,
    /// 别名数组
    #[schemars(
        description = "可选：别名数组——同一对象的历史写法 / 主机名 / ssh 别名（如 trtyr-mac、tencent-beijing）。别名与名称共享命名空间，不许与其它资产相撞。"
    )]
    pub aliases: Option<Vec<String>>,
    /// 规范地址
    #[schemars(description = "可选：规范地址（公网或组网 IP；会变，变化走 update）。")]
    pub ip: Option<String>,
    /// 操作系统
    #[schemars(description = "可选：操作系统。")]
    pub os: Option<String>,
    /// 备注
    #[schemars(description = "可选：一句话备注（规格 / 位置 / 用途线索）。")]
    pub note: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct AssetUpdateParams {
    /// 资产 id
    #[schemars(description = "资产 id（UUID）。")]
    pub asset_id: String,
    /// 新类型
    #[schemars(description = "可选：新类型（host/cloud/domain/account/device/other）。不传不改。")]
    pub kind: Option<String>,
    /// 运维字段（整体替换）
    #[schemars(
        description = "可选：运维字段（自由 kv 对象——主机名/规格/用途/到期日等）。**整体替换**；不传不改。这些字段可被 list 的关键词检索命中。"
    )]
    pub fields: Option<serde_json::Value>,
    /// 新名称
    #[schemars(description = "可选：新台账名。不传不改。")]
    pub name: Option<String>,
    /// 新别名数组（整体替换）
    #[schemars(description = "可选：新别名数组（**整体替换**，不是追加）。不传不改。")]
    pub aliases: Option<Vec<String>>,
    /// 新地址
    #[schemars(description = "可选：新 IP。不传不改。")]
    pub ip: Option<String>,
    /// 新系统
    #[schemars(description = "可选：新操作系统。不传不改。")]
    pub os: Option<String>,
    /// 新备注
    #[schemars(description = "可选：新备注。不传不改。")]
    pub note: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct AssetDeleteParams {
    /// 资产 id
    #[schemars(description = "资产 id（UUID）。被项目位置引用时会**拒绝删除**——先解绑。")]
    pub asset_id: String,
}

// ---------- 工具 ----------

#[tool_router(router = assets_router)]
impl EngramMcpServer {
    /// 资产台账域（单一入口）：我拥有的、可以被操作的东西——主机 / 云实例 / 域名 / 账号 / 设备。
    /// 身份唯一、无「收尾」，被项目**引用**（项目的位置登记指向这里）。操作全景：action="help"。
    #[tool(
        name = "assets",
        annotations(
            title = "资产台账域",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(crate) async fn assets_tool(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(call): Parameters<dispatch::DomainCall>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_assets(&p)?;
        if call.action == "help" {
            let cfg = load_config(&self.state.pool).await;
            return ok_json(dispatch::render_manual("assets", &cfg.disabled_tools));
        }
        match call.action.as_str() {
            "kinds" => {
                self.asset_kinds(
                    ctx,
                    Parameters(dispatch::from_args("assets", "kinds", call.args)?),
                )
                .await
            }
            "list" => {
                self.asset_list(
                    ctx,
                    Parameters(dispatch::from_args("assets", "list", call.args)?),
                )
                .await
            }
            "get" => {
                self.asset_get(
                    ctx,
                    Parameters(dispatch::from_args("assets", "get", call.args)?),
                )
                .await
            }
            "add" => {
                self.asset_add(
                    ctx,
                    Parameters(dispatch::from_args("assets", "add", call.args)?),
                )
                .await
            }
            "update" => {
                self.asset_update(
                    ctx,
                    Parameters(dispatch::from_args("assets", "update", call.args)?),
                )
                .await
            }
            "delete" => {
                self.asset_delete(
                    ctx,
                    Parameters(dispatch::from_args("assets", "delete", call.args)?),
                )
                .await
            }
            "runbook" => {
                self.asset_runbook(
                    ctx,
                    Parameters(dispatch::from_args("assets", "runbook", call.args)?),
                )
                .await
            }
            "runbook_save" => {
                self.asset_runbook_save(
                    ctx,
                    Parameters(dispatch::from_args("assets", "runbook_save", call.args)?),
                )
                .await
            }
            "runbook_versions" => {
                self.asset_runbook_versions(
                    ctx,
                    Parameters(dispatch::from_args(
                        "assets",
                        "runbook_versions",
                        call.args,
                    )?),
                )
                .await
            }
            "runbook_restore" => {
                self.asset_runbook_restore(
                    ctx,
                    Parameters(dispatch::from_args("assets", "runbook_restore", call.args)?),
                )
                .await
            }
            other => Err(dispatch::unknown_action("assets", other)),
        }
    }

    /// 列出资产类型（建档选 kind 用）。
    pub(crate) async fn asset_kinds(
        &self,
        ctx: RequestContext<RoleServer>,
        _params: Parameters<AssetKindsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_assets(&p)?;
        ok_json(
            serde_json::to_value(engram_core::assets::AssetService::list_kinds())
                .unwrap_or(serde_json::json!([])),
        )
    }

    /// 列出资产台账（可按类型过滤 / 关键词检索）。
    pub(crate) async fn asset_list(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<AssetListParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_assets(&p)?;
        let lp = params.0;
        let rows = self
            .svc_asset()
            .list(lp.kind.as_deref(), lp.q.as_deref())
            .await
            .map_err(from_asset)?;
        ok_json(serde_json::json!({
            "count": rows.len(),
            "assets": serde_json::to_value(&rows).unwrap_or(serde_json::json!([])),
        }))
    }

    /// 读资产详情（本体 + 被哪些项目位置引用）。
    pub(crate) async fn asset_get(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<AssetGetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_assets(&p)?;
        let gp = params.0;
        let svc = self.svc_asset();
        let detail = match (gp.asset_id.as_deref(), gp.name.as_deref()) {
            (Some(id), _) => {
                let id = Uuid::parse_str(id)
                    .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "asset_id 不是合法 UUID"))?;
                svc.get(id).await.map_err(from_asset)?
            }
            (None, Some(name)) => {
                let a = svc.get_by_name_or_alias(name).await.map_err(from_asset)?;
                svc.get(a.id).await.map_err(from_asset)?
            }
            (None, None) => {
                return Err(mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    "需要 asset_id 或 name 之一来定位资产",
                ));
            }
        };
        ok_json(serde_json::to_value(&detail).unwrap_or(serde_json::json!({})))
    }

    /// asset_id/name → 资产 Uuid（runbook 四动作共用定位器）。
    async fn resolve_asset_id(
        svc: &engram_core::assets::AssetService,
        asset_id: Option<&str>,
        name: Option<&str>,
    ) -> Result<Uuid, rmcp::ErrorData> {
        match (asset_id, name) {
            (Some(id), _) => Uuid::parse_str(id)
                .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "asset_id 不是合法 UUID")),
            (None, Some(name)) => Ok(svc.get_by_name_or_alias(name).await.map_err(from_asset)?.id),
            (None, None) => Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                "需要 asset_id 或 name 之一来定位资产",
            )),
        }
    }

    /// 运行手册：读某资产的 Markdown 正文（看一眼就知道这台机器什么情况）。
    pub(crate) async fn asset_runbook(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<AssetRunbookParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_assets(&p)?;
        let rp = params.0;
        let svc = self.svc_asset();
        let id = Self::resolve_asset_id(&svc, rp.asset_id.as_deref(), rp.name.as_deref()).await?;
        let md = svc.runbook(id).await.map_err(from_asset)?;
        ok_json(serde_json::json!({ "asset_id": id, "runbook_md": md }))
    }

    /// 运行手册：保存（整体替换；旧文自动入修订史——错改可回滚）。
    pub(crate) async fn asset_runbook_save(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<AssetRunbookSaveParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_assets(&p)?;
        let rp = params.0;
        let svc = self.svc_asset();
        let id = Self::resolve_asset_id(&svc, rp.asset_id.as_deref(), rp.name.as_deref()).await?;
        svc.save_runbook(id, &rp.md, rp.editor.as_deref().unwrap_or("mcp:assets"))
            .await
            .map_err(from_asset)?;
        ok_json(serde_json::json!({
            "asset_id": id,
            "saved": true,
            "hint": "旧文已入修订史——runbook_versions 查看，错改用 runbook_restore 回滚。",
        }))
    }

    /// 运行手册：修订史清单（新→旧）。
    pub(crate) async fn asset_runbook_versions(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<AssetRunbookVersionsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_assets(&p)?;
        let rp = params.0;
        let svc = self.svc_asset();
        let id = Self::resolve_asset_id(&svc, rp.asset_id.as_deref(), rp.name.as_deref()).await?;
        let versions = svc.runbook_versions(id).await.map_err(from_asset)?;
        ok_json(serde_json::json!({
            "asset_id": id,
            "count": versions.len(),
            "versions": serde_json::to_value(&versions).unwrap_or(serde_json::json!([])),
        }))
    }

    /// 运行手册：回滚到某修订（回滚前正文先入史——反复横跳可逆）。
    pub(crate) async fn asset_runbook_restore(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<AssetRunbookRestoreParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_assets(&p)?;
        let rp = params.0;
        let svc = self.svc_asset();
        let id = Self::resolve_asset_id(&svc, rp.asset_id.as_deref(), rp.name.as_deref()).await?;
        let vid = Uuid::parse_str(&rp.version_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "version_id 不是合法 UUID"))?;
        svc.restore_runbook(id, vid, rp.editor.as_deref().unwrap_or("mcp:assets"))
            .await
            .map_err(from_asset)?;
        ok_json(serde_json::json!({
            "asset_id": id,
            "restored_to": vid,
            "hint": "已回滚；回滚前的正文也已入史（可再滚回来）。",
        }))
    }

    /// 建档一台资产（名称与别名共享命名空间，不许与既有条目相撞）。
    pub(crate) async fn asset_add(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<AssetAddParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_assets(&p)?;
        let ap = params.0;
        let a = self
            .svc_asset()
            .create(
                &ap.kind,
                &ap.name,
                ap.aliases.as_deref().unwrap_or(&[]),
                ap.ip.as_deref().unwrap_or(""),
                ap.os.as_deref().unwrap_or(""),
                ap.note.as_deref().unwrap_or(""),
            )
            .await
            .map_err(from_asset)?;
        ok_json(serde_json::to_value(&a).unwrap_or(serde_json::json!({})))
    }

    /// 编辑资产（补丁式；aliases 传了就整体替换）。
    pub(crate) async fn asset_update(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<AssetUpdateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_assets(&p)?;
        let up = params.0;
        let id = Uuid::parse_str(&up.asset_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "asset_id 不是合法 UUID"))?;
        let a = self
            .svc_asset()
            .update(
                id,
                up.kind.as_deref(),
                up.name.as_deref(),
                up.aliases.as_deref(),
                up.ip.as_deref(),
                up.os.as_deref(),
                up.note.as_deref(),
                up.fields.as_ref(),
            )
            .await
            .map_err(from_asset)?;
        ok_json(serde_json::to_value(&a).unwrap_or(serde_json::json!({})))
    }

    /// 删除资产（【破坏性】被项目引用的资产会被拒绝——先解绑）。
    pub(crate) async fn asset_delete(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<AssetDeleteParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_assets(&p)?;
        let id = Uuid::parse_str(&params.0.asset_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "asset_id 不是合法 UUID"))?;
        self.svc_asset().delete(id).await.map_err(from_asset)?;
        ok_json(serde_json::json!({ "deleted": params.0.asset_id }))
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_assets() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::assets_router()
}

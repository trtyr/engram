//! wiki_curation 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

// 该域参数与错误桥仍在既有的 wiki 模块

#[tool_router(router = wiki_curation_router)]
impl EngramMcpServer {
    /// 知识晋升（EN-59）：把项目文档里的一条跨项目知识提炼成 wiki synthesis 页。
    ///
    /// 服务端自动双向回链：页 frontmatter 带 promoted_from（源回链）+ 源文档自动追加
    /// ⛳ 晋升标记（frontmatter.promoted 数组 + 正文末尾可见标记行）+ 登记表可查。
    /// 同一来源文档对同一页只登记一次——重复晋升报友好「已晋升过」。
    ///
    /// 何时用：写项目文档时提炼出「离开本项目还成立、别的项目用得上」的知识
    /// （判据三问见《文档工作流》晋升节）。提炼正文由调用方完成——服务端不做 LLM 提炼。
    pub(crate) async fn wiki_promote(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(ap): Parameters<wiki::WikiPromoteParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let doc_id = uuid::Uuid::parse_str(&ap.doc_id).map_err(|_| {
            rmcp::ErrorData::invalid_params(
                format!(
                    "doc_id 非法 UUID：{}——用 project-get 看文档列表取 id",
                    ap.doc_id
                ),
                None,
            )
        })?;
        let out = engram_core::promote::PromoteService::new(self.state.pool.clone())
            .promote(engram_core::promote::PromoteRequest {
                project: ap.project,
                doc_id,
                anchor: ap.anchor,
                slug: ap.slug,
                title: ap.title,
                content: ap.content,
                library: None, // 单库终局（ADR-16）：恒 main，RJ-19 清理
            })
            .await
            .map_err(|e| match e {
                engram_core::promote::PromoteError::NotFound(m) => {
                    rmcp::ErrorData::resource_not_found(m, None)
                }
                engram_core::promote::PromoteError::Conflict(m) => {
                    rmcp::ErrorData::invalid_params(m, None)
                }
                engram_core::promote::PromoteError::BadRequest(m) => {
                    rmcp::ErrorData::invalid_params(m, None)
                }
                other => rmcp::ErrorData::internal_error(other.to_string(), None),
            })?;
        ok_json(serde_json::json!({
            "promoted": true,
            "library": out.library,
            "page_slug": out.page_slug,
            "page_title": out.page_title,
            "project": out.project_name,
            "hint": "源文档已追加 ⛳ 晋升标记（双向可查）——wiki search 可命中新页",
        }))
    }

    /// 晋升登记列表（谁家的哪些知识晋升成了 wiki 页；按项目过滤）。
    pub(crate) async fn wiki_promotions(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiPromotionsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let rows = engram_core::promote::PromoteService::new(self.state.pool.clone())
            .list_promotions(params.0.project.as_deref())
            .await
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))?;
        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
    }

    /// 页面版本列表（新→旧；含已删除页的最后状态快照）。
    ///
    /// 何时用：覆盖更新前看看历史；或找某个版本的 version 号/预览正文。
    pub(crate) async fn wiki_versions(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiVersionsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let rows = wiki::svc(&self.state)
            .page_versions(lib, &params.0.slug)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
    }

    /// 读取某版本快照的正文（回滚前预览用）。
    pub(crate) async fn wiki_version_content(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiVersionContentParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let content = wiki::svc(&self.state)
            .page_version_content(lib, &params.0.slug, params.0.version)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::json!({
            "slug": params.0.slug,
            "version": params.0.version,
            "content": content,
        }))
    }

    /// 回滚页面到历史版本（回滚本身产生新版本，历史不丢；已删除的页面从快照重建）。
    ///
    /// 何时用：一次覆盖改坏内容时；或误删页面要找回。
    pub(crate) async fn wiki_restore_version(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiRestoreVersionParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let page = wiki::svc(&self.state)
            .restore_page_version(lib, &params.0.slug, params.0.version)
            .await
            .map_err(wiki::from_wiki)?;
        let mut v = wiki::trim_page(serde_json::to_value(&page).unwrap_or(serde_json::json!({})));
        v["restored_from"] = json!(params.0.version);
        v["message"] = json!("已恢复（本次回滚前状态留了快照，可再滚回）");
        ok_json(v)
    }

    /// 列出来源记录（wiki_sources：ingest 的源文本及其状态）。
    ///
    /// 何时用：lint 报 stale_source 后找要清理的 source_id；或查某次 ingest 的状态。
    pub(crate) async fn wiki_sources(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(_): Parameters<wiki::WikiLibParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let rows = wiki::svc(&self.state)
            .list_sources(lib)
            .await
            .map_err(wiki::from_wiki)?;
        let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(id, title, status, sha)| {
            json!({"source_id": id, "title": title, "status": status, "sha256": sha})
        })
        .collect();
        ok_json(serde_json::to_value(&items).unwrap_or(serde_json::json!([])))
    }

    /// 删除一条来源记录及其全部产出（级联：源、其执行记录、由它产出的页面；不可逆）。
    ///
    /// 何时用：lint 报 stale_source（页面已删但原料残留）或想整体撤销一次原料处理。
    pub(crate) async fn wiki_delete_source(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiDeleteSourceParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let id = Uuid::parse_str(&params.0.source_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "source_id 不是合法 UUID"))?;
        let lib = self.resolve_wiki_lib().await?;
        let report = wiki::svc(&self.state)
            .delete_source_cascade(lib, id)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(
            serde_json::to_value(&report)
                .unwrap_or(serde_json::json!({ "deleted": params.0.source_id })),
        )
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_wiki_curation() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::wiki_curation_router()
}

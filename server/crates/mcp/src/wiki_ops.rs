//! wiki_ops 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

// 该域参数与错误桥仍在既有的 wiki 模块

#[tool_router(router = wiki_ops_router)]
impl EngramMcpServer {
    // ---------- Wiki 域（wiki scope；实现细节见 mcp_wiki.rs） ----------

    /// Wiki 检索（FTS + 向量 RRF 融合，带 wiki 方向意图 purpose）。
    ///
    /// 何时用：需要查证「世界知识」（用户 Wiki 里沉淀的文档、概念、实体、问答）时。
    /// 何时不用：回忆「用户本人」的偏好/事实/经历用 memory_search——那是用户记忆域。
    /// 返回 {purpose, pages}：命中只带片段（命中词附近 ~160 字符）+ content_chars，
    /// 全文按需 wiki_get_page——检索可能拖回数万字符全文是 R 报告点名的上下文黑洞（P0-2）。
    pub(crate) async fn wiki_search(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiSearchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let wp = params.0;
        let lib = self.resolve_wiki_lib().await?;
        let result = wiki::svc(&self.state)
            .search_with_purpose(
                lib,
                &wp.query,
                wp.max_items.unwrap_or(20),
                wp.rerank.unwrap_or(false),
            )
            .await
            .map_err(wiki::from_wiki)?;
        let mut v = result;
        if let Some(pages) = v["pages"].as_array_mut() {
            *pages = pages
                .iter()
                .cloned()
                .map(|pg| wiki::snippet_page(pg, &wp.query))
                .collect();
        }
        v["hint"] = json!("命中只带片段——读全文用 get_page（slug 在每条命中里）");
        ok_json(v)
    }

    /// 浏览 Wiki 页面列表（可按页型过滤；列表不带正文）。
    ///
    /// 何时用：想系统性看看 Wiki 里有什么（而非定向检索）时；读全文用 wiki_get_page。
    pub(crate) async fn wiki_list_pages(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiListPagesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lp = params.0;
        let lib = self.resolve_wiki_lib().await?;
        let pages = wiki::svc(&self.state)
            .list_pages(
                lib,
                lp.page_type.as_deref(),
                lp.folder.as_deref(),
                lp.limit,
                lp.cursor.as_deref(),
            )
            .await
            .map_err(wiki::from_wiki)?;
        let values: Vec<serde_json::Value> = serde_json::to_value(&pages)
            .unwrap_or(serde_json::json!([]))
            .as_array()
            .map(|a| a.iter().cloned().map(wiki::trim_page).collect())
            .unwrap_or_default();
        ok_json(serde_json::to_value(&values).unwrap_or(serde_json::json!([])))
    }

    /// 读取一个 Wiki 页面全文（含 frontmatter 与版本）。
    ///
    /// 何时用：wiki_search / wiki_list_pages 定位到页面后需要读全文时。
    pub(crate) async fn wiki_get_page(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiGetPageParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let page = wiki::svc(&self.state)
            .get_page(lib, &params.0.slug)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::to_value(&page).unwrap_or(serde_json::json!({})))
    }

    /// 写入 / 更新一个 Wiki 页面（AI 通道，frontmatter.via 落 "ai" 标记）。
    ///
    /// 何时用：用户明确要求「把……记到 Wiki / 写个页面」时，把结构化的知识沉淀成页面
    /// （Markdown + [[wikilink]]）。已存在同 slug 页面则整体覆盖更新（版本 +1）。
    /// 注意：这是覆盖式写入——更新既有页面前先用 wiki_get_page 读原文，别盲目覆盖；
    /// 临时性的问答结论更适合 wiki_archive_query 而非手搓页面。
    pub(crate) async fn wiki_write_page(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiWritePageParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let wp = params.0;
        let lib = self.resolve_wiki_lib().await?;
        let page = wiki::svc(&self.state)
            .put_page(
                lib,
                &wp.slug,
                &wp.title,
                &wp.content,
                wp.folder.as_deref(),
                Some("ai"),
            )
            .await
            .map_err(wiki::from_wiki)?;
        // P0-1：刚发送的正文不回显；版本历史在覆盖时自动留快照
        let mut v = wiki::trim_page(serde_json::to_value(&page).unwrap_or(serde_json::json!({})));
        v["content_omitted"] = json!(true);
        // 写入即处理（2026-09-13）：AI 写页自动入队轻量再摄取（页面当原料吸收概念/互链；
        // 同 sha 去重；摄取失败不影响页面本身，失败会落 reviews via=ingest_failed 可见）
        match wiki::svc(&self.state)
            .auto_ingest_page(lib, &wp.title, &wp.content)
            .await
        {
            Ok(ingest) => v["auto_ingest"] = ingest,
            Err(e) => {
                v["auto_ingest"] = json!({"state": "failed", "hint": format!("织入入队失败（页面本身已保存）: {e}")});
            }
        }
        ok_json(v)
    }

    /// 把一条问答（问 + 答）存档为 queries 页并自动再摄取。
    ///
    /// 何时用：一次检索/讨论得出值得长期保留的结论时，落成「查询」页沉淀。
    /// 同标题已存档 → 幂等跳过（skipped=true），不会重复烧 LLM。
    pub(crate) async fn wiki_archive_query(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiArchiveQueryParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let qp = params.0;
        let lib = self.resolve_wiki_lib().await?;
        let skipped = wiki::svc(&self.state)
            .archive_query(lib, &qp.title, &qp.question, &qp.answer)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::json!({
            "skipped": skipped,
            "async": true,
            "message": if skipped {
                "同标题已存档，本次跳过"
            } else {
                "已落 queries 页并入队再摄取"
            }
        }))
    }

    /// Wiki 链接图全貌（节点 = 页面，边 = [[wikilink]]，含社区划分）。
    ///
    /// 何时用：写页面前了解现有结构、找相关页面、看知识网络长什么样。
    /// 页面很多时输出较大——粗看结构够用，定位具体页面用 wiki_search。
    pub(crate) async fn wiki_graph(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(_): Parameters<wiki::WikiLibParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let graph = wiki::svc(&self.state)
            .graph(lib)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::to_value(&graph).unwrap_or(serde_json::json!({})))
    }

    // 文档域错误映射（WikiDocumentError → rmcp）
    pub(crate) fn from_wiki_docs(e: engram_core::wiki_docs::WikiDocumentError) -> rmcp::ErrorData {
        use engram_core::wiki_docs::WikiDocumentError;
        match e {
            WikiDocumentError::BadRequest(m) => rmcp::ErrorData::invalid_params(m, None),
            WikiDocumentError::NotFound(m) => rmcp::ErrorData::resource_not_found(m, None),
            WikiDocumentError::Storage(m) => rmcp::ErrorData::internal_error(m, None),
        }
    }

    /// 内容目录（index）：按 page_type 分组的全库页面目录（slug/标题/入链数/首段摘要）。
    ///
    /// 何时用：回答「这个库里有什么」/为深入检索做导航——只读动态聚合，零 LLM 成本。
    pub(crate) async fn wiki_index(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(_): Parameters<wiki::WikiLibParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let idx = wiki::svc(&self.state)
            .index(lib)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(idx)
    }

    /// 问答/分析产物归档（karpathy LLM Wiki：好答案不该消失在聊天记录里）。
    ///
    /// 何时用：一段有价值的分析/对比/结论值得长期沉淀时——以 analysis 类型（0040）
    /// 落页（复用版本快照），related 列表自动建双向 wikilinks 融入链接图。
    pub(crate) async fn wiki_archive(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(ap): Parameters<wiki::WikiArchiveParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let related = ap.related.unwrap_or_default();
        let page = wiki::svc(&self.state)
            .archive_answer(lib, &ap.slug, &ap.title, &ap.content, &related)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::to_value(&page).unwrap_or(serde_json::json!({})))
    }

    /// 读取库的方向意图（EN-61）：purpose 是每库一份的「这个库收什么/不收什么」约定。
    ///
    /// 何时用：wiki write_page 之前——先读 purpose 对齐方向，避免写跑题。
    pub(crate) async fn wiki_purpose(
        &self,
        ctx: RequestContext<RoleServer>,
        _params: Parameters<wiki::WikiLibParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let purpose = wiki::svc(&self.state)
            .get_purpose(lib)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::to_value(&purpose).unwrap_or(serde_json::json!({})))
    }

    /// 删除 Wiki 页面（不可逆——连带清理双向 wikilinks；最后状态留版本快照可重建）。
    ///
    /// 何时用：页面作废/测试数据清理。只对明确表达的删除请求使用。
    pub(crate) async fn wiki_delete_page(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiDeletePageParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let deleted = wiki::svc(&self.state)
            .delete_page(lib, &params.0.slug)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::json!({
            "deleted": params.0.slug, "ok": deleted,
            "message": "已删除（最后状态留有版本快照——误删可 restore_version 重建）",
        }))
    }

    /// Wiki 域（单一入口）：世界知识库——Markdown 页面 + [[wikilink]] + 混合检索。
    /// 查证「客观知识」用 "search"；用户要求沉淀时：单条结论 "archive_query"、
    /// 整篇文档 "ingest"（异步）、明确要页面 "write_page"。操作全景：action="help"。
    #[tool(
        name = "wiki",
        annotations(
            title = "Wiki 域",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(crate) async fn wiki_tool(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(call): Parameters<dispatch::DomainCall>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        if call.action == "help" {
            let cfg = load_config(&self.state.pool).await;
            return ok_json(dispatch::render_manual("wiki", &cfg.disabled_tools));
        }
        match call.action.as_str() {
            "search" => {
                self.wiki_search(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "search", call.args)?),
                )
                .await
            }
            "list_pages" => {
                self.wiki_list_pages(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "list_pages", call.args)?),
                )
                .await
            }
            "get_page" => {
                self.wiki_get_page(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "get_page", call.args)?),
                )
                .await
            }
            "write_page" => {
                self.wiki_write_page(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "write_page", call.args)?),
                )
                .await
            }
            "ingest" => {
                self.wiki_ingest(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "ingest", call.args)?),
                )
                .await
            }
            "archive_query" => {
                self.wiki_archive_query(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "archive_query", call.args)?),
                )
                .await
            }
            "versions" => {
                self.wiki_versions(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "versions", call.args)?),
                )
                .await
            }
            "version_content" => {
                self.wiki_version_content(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "version_content", call.args)?),
                )
                .await
            }
            "restore_version" => {
                self.wiki_restore_version(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "restore_version", call.args)?),
                )
                .await
            }
            "sources" => {
                self.wiki_sources(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "sources", call.args)?),
                )
                .await
            }
            "delete_source" => {
                self.wiki_delete_source(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "delete_source", call.args)?),
                )
                .await
            }
            "graph" => {
                self.wiki_graph(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "graph", call.args)?),
                )
                .await
            }
            "lint" => {
                self.wiki_lint(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "lint", call.args)?),
                )
                .await
            }
            "lint_deep" => {
                self.wiki_lint_deep(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "lint_deep", call.args)?),
                )
                .await
            }
            "merge" => {
                self.wiki_merge(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "merge", call.args)?),
                )
                .await
            }
            "document_add" => {
                self.wiki_document_add(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "document_add", call.args)?),
                )
                .await
            }
            "document_get" => {
                self.wiki_document_get(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "document_get", call.args)?),
                )
                .await
            }
            "documents_search" => {
                self.wiki_documents_search(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "documents_search", call.args)?),
                )
                .await
            }
            "reviews" => {
                self.wiki_reviews(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "reviews", call.args)?),
                )
                .await
            }
            "review_resolve" => {
                self.wiki_review_resolve(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "review_resolve", call.args)?),
                )
                .await
            }
            "index" => {
                self.wiki_index(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "index", call.args)?),
                )
                .await
            }
            "archive" => {
                self.wiki_archive(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "archive", call.args)?),
                )
                .await
            }
            "purpose" => {
                self.wiki_purpose(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "purpose", call.args)?),
                )
                .await
            }
            "insights" => {
                self.wiki_insights(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "insights", call.args)?),
                )
                .await
            }
            "promote" => {
                self.wiki_promote(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "promote", call.args)?),
                )
                .await
            }
            "promotions" => {
                self.wiki_promotions(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "promotions", call.args)?),
                )
                .await
            }
            "delete_page" => {
                self.wiki_delete_page(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "delete_page", call.args)?),
                )
                .await
            }
            other => Err(dispatch::unknown_action("wiki", other)),
        }
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_wiki_ops() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::wiki_ops_router()
}

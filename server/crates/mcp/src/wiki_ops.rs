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
        // P004-T009：auto_ingest（自动轻量织入）已随织入流水线退役（Q005 拍板 C+）——
        // 页面维护由 Agent Harness（T010）接管；write_page 只写页本体。
        // EN-236②：写入→后续动作提示（页面已入链接图——体检与组织入口）
        if v.get("hint").is_none() {
            v["hint"] = json!(
                "页面已保存并入链接图。后续：action=\"lint_deep\" 深检本页链接质量；整理/合并用 \"merge\"，问答类结论可直接 \"archive_query\""
            );
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
        let (skipped, slug) = wiki::svc(&self.state)
            .archive_query(lib, &qp.title, &qp.question, &qp.answer)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::json!({
            "skipped": skipped,
            "slug": slug,
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
        // EN-243①：未配置要可感知——手册称「写前必读」，读到空却无提示等于没读
        let mut v = serde_json::to_value(&purpose).unwrap_or(serde_json::json!({}));
        let configured = v.get("purpose").is_some_and(|x| !x.is_null());
        v["configured"] = serde_json::json!(configured);
        if !configured {
            v["hint"] = serde_json::json!(
                "库意图未设置——用 purpose_action 配置（写页前先对齐「这个库收什么/不收什么」）"
            );
        }
        ok_json(v)
    }

    /// 设置库方向意图（EN-61 写侧）：goals/key_questions/scope/thesis 一次给全。
    ///
    /// 何时用：库初建或方向调整时——purpose 会注入 ingest/query 的 LLM 提示词，
    /// 是「这个库收什么/不收什么」的约定；设完可用 purpose 读回确认。
    pub(crate) async fn wiki_purpose_set(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiPurposeSetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let sp = params.0;
        let lib = self.resolve_wiki_lib().await?;
        let purpose = engram_core::wiki::Purpose {
            goals: sp.goals,
            key_questions: sp.key_questions,
            scope: sp.scope,
            thesis: sp.thesis,
        };
        wiki::svc(&self.state)
            .set_purpose(lib, &purpose)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::json!({
            "ok": true,
            "message": "purpose 已设置——后续 ingest/query 将按此方向对齐",
        }))
    }

    /// 重复候选（标题归一化相同页面组）——研判流入口。
    ///
    /// 何时用：lint/日常巡检发现疑似重复后，用本列表逐组研判；处置用 merge 合并（留痕）。
    pub(crate) async fn wiki_duplicates(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(_): Parameters<wiki::WikiLibParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let candidates = wiki::svc(&self.state)
            .duplicate_candidates(lib)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::json!({
            "candidates": candidates,
            "hint": "研判后用 action=\"merge\"（primary/duplicate）合并，留版本快照",
        }))
    }

    /// 查询缺口清单（零命中/低分查询=内容缺口）——织入方向与 Deep Research 的输入。
    pub(crate) async fn wiki_query_gaps(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(_): Parameters<wiki::WikiLibParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let gaps = wiki::svc(&self.state)
            .query_gaps(lib, 50)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::to_value(&gaps).unwrap_or(serde_json::json!([])))
    }

    /// 目录骨架（folder, 页数）——前端懒加载树先渲染结构；配合 list_pages 的 folder 过滤。
    pub(crate) async fn wiki_folders(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(_): Parameters<wiki::WikiLibParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let folders = wiki::svc(&self.state)
            .list_folders(lib)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::to_value(&folders).unwrap_or(serde_json::json!([])))
    }

    /// 待审提案聚合（wiki_generate 任务的最新提案事件）——人审后用 proposal_apply 合入。
    pub(crate) async fn wiki_proposals(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(_): Parameters<wiki::WikiLibParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let rows = engram_jobs::admin::latest_wiki_proposals(&self.state.pool)
            .await
            .map_err(|e| mcp_err(rmcp::model::ErrorCode::INTERNAL_ERROR, e.to_string()))?;
        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
    }

    /// 人审合入提案（把 proposals 里的提案内容写入页面）。
    pub(crate) async fn wiki_proposal_apply(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiProposalApplyParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let ap = params.0;
        let lib = self.resolve_wiki_lib().await?;
        let page = wiki::svc(&self.state)
            .apply_proposal(lib, &ap.slug, &ap.content, &ap.title, ap.via.as_deref())
            .await
            .map_err(wiki::from_wiki)?;
        let mut v = serde_json::to_value(&page).unwrap_or(serde_json::json!({}));
        v["content_omitted"] = json!(true);
        ok_json(v)
    }

    /// 确定性修复（lint 修而不只报）：死链改写/去链接化/建 stub/孤页回挂/重复合并。
    pub(crate) async fn wiki_repair(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(_): Parameters<wiki::WikiLibParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let report = wiki::svc(&self.state)
            .repair(lib)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::to_value(&report).unwrap_or(serde_json::json!({})))
    }

    /// 修复异步入队（大库友好）——任务页可查进度与历史。
    pub(crate) async fn wiki_repair_async(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(_): Parameters<wiki::WikiLibParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let job_id = engram_wiki_engine::repair::enqueue(&self.state.pool, lib)
            .await
            .map_err(|e| mcp_err(rmcp::model::ErrorCode::INTERNAL_ERROR, e.to_string()))?;
        ok_json(serde_json::json!({
            "job_id": job_id,
            "note": "修复已入队——GET /jobs/{job_id} 或任务页查进度",
        }))
    }

    /// dismiss 图洞察（key 不再出现）。
    pub(crate) async fn wiki_insight_dismiss(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiInsightDismissParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        wiki::svc(&self.state)
            .insight_dismiss(lib, &params.0.key)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::json!({ "ok": true, "dismissed": params.0.key }))
    }

    /// 重置全部 dismissed 洞察（重新可见）。
    pub(crate) async fn wiki_insight_reset(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(_): Parameters<wiki::WikiLibParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        wiki::svc(&self.state)
            .insight_reset(lib)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::json!({ "ok": true }))
    }

    /// 存量回填：重析全部页面正文重建 wiki_links（幂等）。
    pub(crate) async fn wiki_rebuild_links(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(_): Parameters<wiki::WikiLibParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let n = wiki::svc(&self.state)
            .rebuild_all_links(lib)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::json!({ "rebuilt_links": n }))
    }

    /// 存量内容页 tsv 重刷（EN-63；排除 index/log/overview 系统页；幂等）。
    pub(crate) async fn wiki_rebuild_tsv(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(_): Parameters<wiki::WikiLibParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let n = wiki::svc(&self.state)
            .backfill_tsv(lib)
            .await
            .map_err(wiki::from_wiki)?;
        ok_json(serde_json::json!({ "rebuilt_tsv": n }))
    }

    /// 重新嵌入文档缺失块（EN-32 恢复入口：embed_failed/NULL 向量补嵌；已嵌入块不重复计费）。
    pub(crate) async fn wiki_reembed(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiReembedParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let id = uuid::Uuid::parse_str(params.0.doc_id.trim()).map_err(|_| {
            mcp_err(
                rmcp::model::ErrorCode::INVALID_PARAMS,
                "doc_id 不是合法 UUID",
            )
        })?;
        let svc = engram_core::wiki_docs::WikiDocumentService::new(
            self.state.pool.clone(),
            self.state.registry(),
            self.state.data_dir.clone(),
        );
        svc.reembed(lib, id).await.map_err(Self::from_wiki_docs)?;
        ok_json(serde_json::json!({
            "queued": id,
            "hint": "补嵌 job 已入队——document_get 看 status 进度",
        }))
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
    /// 明确要页面 "write_page"。操作全景：action="help"。
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
            "ingest" => {
                self.wiki_ingest(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "ingest", call.args)?),
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
            "document_delete" => {
                self.wiki_document_delete(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "document_delete", call.args)?),
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
            "purpose_set" => {
                self.wiki_purpose_set(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "purpose_set", call.args)?),
                )
                .await
            }
            "duplicates" => {
                self.wiki_duplicates(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "duplicates", call.args)?),
                )
                .await
            }
            "query_gaps" => {
                self.wiki_query_gaps(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "query_gaps", call.args)?),
                )
                .await
            }
            "folders" => {
                self.wiki_folders(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "folders", call.args)?),
                )
                .await
            }
            "proposals" => {
                self.wiki_proposals(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "proposals", call.args)?),
                )
                .await
            }
            "proposal_apply" => {
                self.wiki_proposal_apply(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "proposal_apply", call.args)?),
                )
                .await
            }
            "repair" => {
                self.wiki_repair(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "repair", call.args)?),
                )
                .await
            }
            "repair_async" => {
                self.wiki_repair_async(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "repair_async", call.args)?),
                )
                .await
            }
            "insight_dismiss" => {
                self.wiki_insight_dismiss(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "insight_dismiss", call.args)?),
                )
                .await
            }
            "insight_reset" => {
                self.wiki_insight_reset(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "insight_reset", call.args)?),
                )
                .await
            }
            "rebuild_links" => {
                self.wiki_rebuild_links(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "rebuild_links", call.args)?),
                )
                .await
            }
            "rebuild_tsv" => {
                self.wiki_rebuild_tsv(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "rebuild_tsv", call.args)?),
                )
                .await
            }
            "reembed" => {
                self.wiki_reembed(
                    ctx,
                    Parameters(dispatch::from_args("wiki", "reembed", call.args)?),
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

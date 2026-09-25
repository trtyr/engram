//! wiki_docs 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

// 该域参数与错误桥仍在既有的 wiki 模块

#[tool_router(router = wiki_docs_router)]
impl EngramMcpServer {
    /// 把一段源文本织入 Wiki（异步：入队 LLM 流水线，自动抽取实体/概念并互链）。
    ///
    /// 何时用：有一篇完整文档 / 长文本值得沉淀进知识库时。内容相同（sha 命中）会跳过。
    /// 注意：织入是异步任务（前端「任务」页可见），立即返回 skipped 只代表入队/去重结果；
    /// 单条问答式的结论用 wiki_archive_query 更合适。
    pub(crate) async fn wiki_ingest(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiIngestParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let wp = params.0;
        let lib = self.resolve_wiki_lib().await?;
        let outcome = wiki::svc(&self.state)
            .ingest(lib, &wp.title, &wp.text)
            .await
            .map_err(wiki::from_wiki)?;
        // D27 三态：已就绪（跳过）/ 在途（勿重提也非丢失）/ 新入队——此前三者不可分，
        // sha 去重封锁重试，在途窗口任务表现如「丢失」
        use engram_core::wiki::IngestOutcome;
        let job_id = outcome.job_id();
        let (skipped, status, message) = match &outcome {
            IngestOutcome::AlreadyReady(_) => (
                true,
                "ready",
                "内容已存在（sha 命中），本次跳过".to_string(),
            ),
            IngestOutcome::InFlight(_, _) => (
                false,
                "in_flight",
                "同内容任务正在处理中——无需重复提交（sha 去重会挡住），稍后可在 Wiki 页面看到产物"
                    .to_string(),
            ),
            IngestOutcome::Enqueued(_, _) => (
                false,
                "enqueued",
                "已入队织入任务——LLM 流水线异步处理，稍后可在 Wiki 页面看到产物".to_string(),
            ),
        };
        let mut out = serde_json::json!({
            "skipped": skipped,
            "status": status,
            "source_id": outcome.source_id(),
            "async": true,
            "message": message,
        });
        // R8 观察 3：进度通道落到具体 id——GET /jobs/{job_id}（任意 scope 的 key 可读）
        if let Some(j) = job_id {
            out["job_id"] = serde_json::json!(j);
            out["message"] =
                serde_json::json!(format!("{message}；进度：GET /jobs/{j}（或任务页）"));
        }
        ok_json(out)
    }

    /// 删除一条文档 RAG 原料（document_add 返回的 id；documents 体系——非 delete_source 的 sources 体系）。
    ///
    /// 何时用：撤销一次入库（连同分块/嵌入一起删）。
    pub(crate) async fn wiki_document_delete(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<wiki::WikiDocumentDeleteParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let id = Uuid::parse_str(&params.0.doc_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "doc_id 不是合法 UUID"))?;
        let lib = self.resolve_wiki_lib().await?;
        let svc = engram_core::wiki_docs::WikiDocumentService::new(
            self.state.pool.clone(),
            self.state.registry(),
            self.state.data_dir.clone(),
        );
        svc.delete(lib, id).await.map_err(Self::from_wiki_docs)?;
        ok_json(serde_json::json!({
            "deleted": params.0.doc_id,
            "hint": "文档及其分块/嵌入已删除",
        }))
    }

    // ---------- 文档 RAG（wiki_documents）——MCP 对齐 HTTP 能力（工单「工具面不对齐」） ----------

    /// 入库文档（document_add）：text 或 url → 分块+嵌入进原文 RAG，并触发 LLM 织入。
    pub(crate) async fn wiki_document_add(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(dp): Parameters<wiki::WikiDocumentAddParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let svc = engram_core::wiki_docs::WikiDocumentService::new(
            self.state.pool.clone(),
            self.state.registry(),
            self.state.data_dir.clone(),
        );
        let source = if let Some(url) = dp.url.as_deref().filter(|u| !u.trim().is_empty()) {
            engram_core::wiki_docs::IngestSource::Url(url.trim().to_string())
        } else if let Some(text) = dp.text.as_deref().filter(|t| !t.trim().is_empty()) {
            engram_core::wiki_docs::IngestSource::Bytes {
                name: dp.name.clone().unwrap_or_else(|| "mcp-text".into()),
                content: text.as_bytes().to_vec(),
                content_type: Some("text/markdown".into()),
            }
        } else {
            return Err(rmcp::ErrorData::invalid_params(
                "text 与 url 二选一".to_string(),
                None,
            ));
        };
        let (id, deduped) = svc
            .submit(lib, source)
            .await
            .map_err(Self::from_wiki_docs)?;
        let doc = svc
            .get_document(lib, id)
            .await
            .map_err(Self::from_wiki_docs)?;
        ok_json(serde_json::json!({
            "id": doc.id,
            "title": doc.title,
            "status": doc.status,
            "deduped": deduped,
            "hint": "入库成功（异步分块/嵌入/织入）——用 document_get 看 status 进度；原文检索用 documents_search",
        }))
    }

    /// 文档状态（document_get）：看处理进度（status/error）。
    pub(crate) async fn wiki_document_get(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(dp): Parameters<wiki::WikiDocumentGetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let id = uuid::Uuid::parse_str(dp.id.trim()).map_err(|_| {
            rmcp::ErrorData::invalid_params(format!("id 不是合法 UUID: {}", dp.id), None)
        })?;
        let svc = engram_core::wiki_docs::WikiDocumentService::new(
            self.state.pool.clone(),
            self.state.registry(),
            self.state.data_dir.clone(),
        );
        let doc = svc
            .get_document(lib, id)
            .await
            .map_err(Self::from_wiki_docs)?;
        ok_json(serde_json::to_value(&doc).unwrap_or(serde_json::json!({})))
    }

    /// 原文检索（documents_search）：chunk 级 FTS+向量混合——搜原文分块，与页面级 wiki search 互补。
    pub(crate) async fn wiki_documents_search(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(dp): Parameters<wiki::WikiDocumentsSearchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let svc = engram_core::wiki_docs::WikiDocumentService::new(
            self.state.pool.clone(),
            self.state.registry(),
            self.state.data_dir.clone(),
        );
        let hits = svc
            .search(lib, &dp.query, dp.limit.unwrap_or(8).clamp(1, 50))
            .await
            .map_err(Self::from_wiki_docs)?;
        ok_json(serde_json::to_value(&hits).unwrap_or(serde_json::json!([])))
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_wiki_docs() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::wiki_docs_router()
}

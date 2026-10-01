//! wiki_docs 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

// 该域参数与错误桥仍在既有的 wiki 模块

#[tool_router(router = wiki_docs_router)]
impl EngramMcpServer {
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

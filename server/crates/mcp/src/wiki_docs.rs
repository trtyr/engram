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

    /// 入库文档（document_add）：text 或 url → 分块+嵌入进原文 RAG；完成后维护 Agent 自动接力。
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
            "hint": "入库成功（异步分块/嵌入）——jobs 工具按 job_id 看处理进度",
        }))
    }

    /// wiki ingest（P004-T010 同名换芯）：喂原料给 wiki 维护 Agent Harness（异步 job）。
    pub(crate) async fn wiki_ingest(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(ip): Parameters<wiki::WikiIngestParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        wiki::require_wiki(&p)?;
        let lib = self.resolve_wiki_lib().await?;
        let has_url = ip.url.as_deref().is_some_and(|u| !u.trim().is_empty());
        let has_text = ip.text.as_deref().is_some_and(|t| !t.trim().is_empty());
        if has_url == has_text {
            return Err(rmcp::ErrorData::invalid_params(
                "text 与 url 二选一".to_string(),
                None,
            ));
        }
        let task = engram_core::wiki_agent::AgentTask {
            lib,
            instruction: ip
                .instruction
                .clone()
                .unwrap_or_else(|| "阅读下方原料，沉淀为 wiki 知识页（建页/更新/互链）。".into()),
            source_url: ip
                .url
                .as_deref()
                .map(str::trim)
                .map(str::to_string)
                .filter(|s| !s.is_empty()),
            source_text: ip.text.clone().filter(|t| !t.trim().is_empty()),
            source_name: ip.title.clone(),
        };
        let queue = engram_jobs::JobQueue::new(self.state.pool.clone());
        let job_id = engram_core::wiki_agent::enqueue_agent_task(&queue, &task)
            .await
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))?;
        ok_json(serde_json::json!({
            "job_id": job_id,
            "accepted": true,
            "hint": "维护 Agent 已接单（异步）——jobs 工具看进度；完成后 wiki_search 查看新页",
        }))
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_wiki_docs() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::wiki_docs_router()
}

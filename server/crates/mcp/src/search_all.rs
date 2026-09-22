//! search_all 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

/// 跨域全局检索（R 报告 P1-8：把 6 次单域搜索并成 1 次）。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SearchAllParams {
    /// 检索词（各域同词并发检索）
    #[schemars(
        description = "检索词。对 key 有 scope 的域并发检索（memory/wiki/skills/todos/projects）。"
    )]
    pub query: String,
    /// R6：可选 LLM 精排（默认关）——开时五域命中合并 top-10 交 LLM 重排，响应附 reranked 视图
    #[schemars(
        description = "可选：LLM 精排（默认关）。开时五域命中合并 top-10 交 LLM 重排，响应附 reranked 视图（LLM 失败降级原分组）。"
    )]
    pub rerank: Option<bool>,
    /// 每域返回条数（默认 3）
    #[schemars(description = "每域返回条数上限，默认 3。结果只有摘要——精确检索请用单域工具。")]
    pub max_per_domain: Option<i64>,
}

#[tool_router(router = search_all_router)]
impl EngramMcpServer {
    // ---------- 跨域全局检索（常驻工具之一；不属单一 scope，按 key 实际 scope 分域执行） ----------

    /// 全局检索（R 报告 P1-8）：一次查询并发打 memory/wiki/skills/todos/projects 五域，
    /// 各返回 top-k 摘要（含命中域标注）——「6 次单域搜索」压成 1 次。
    ///
    /// 何时用：不确定信息在哪域、或要先扫一遍全库面时。
    /// 何时不用：已知域的精确检索直接用单域工具（省时省 token，且支持更多过滤参数）。
    /// 只检索本 key 有 scope 的域；命中只有摘要，全文按各域 get/read 通道按需取。
    #[tool(
        name = "search_all",
        annotations(
            title = "全局检索",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(crate) async fn search_all_tool(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SearchAllParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        let q = params.0.query.trim().to_string();
        if q.is_empty() {
            return Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                "query 不能为空——给出检索词，各域并发检索",
            ));
        }
        let max = params.0.max_per_domain.unwrap_or(3).clamp(1, 10);
        let scope_of = |s: &str| p.domain_access(s) != DomainAccess::None;
        if !["memory", "wiki", "skills", "todos", "project"]
            .iter()
            .any(|s| scope_of(s))
        {
            return Err(mcp_err(
                ErrorCode::INVALID_REQUEST,
                "本 key 没有任何可检索域的 scope——search_all 需要至少一个域的读权限",
            ));
        }

        // memory（l1/l2/实体）；检索失败（如 LLM 未配置）降级为错误标注而非整体失败
        let mem = scope_of("memory");
        let mem_fut = async {
            if !mem {
                return None;
            }
            let r = self
                .svc()
                .search(&q, &["l1", "l2", "entities"], max, false, None, None)
                .await;
            Some(match r {
                Ok(r) => json!({
                    "l1": r.l1.iter().map(|h| json!({"id": h.id, "score": h.score, "snippet": h.snippet})).collect::<Vec<_>>(),
                    "l2": r.l2.iter().map(|h| json!({"id": h.id, "title": h.title, "snippet": h.snippet})).collect::<Vec<_>>(),
                    "entities": r.entities.iter().map(|h| json!({"id": h.id, "title": h.title, "kind": h.kind})).collect::<Vec<_>>(),
                }),
                Err(e) => json!({ "error": e.to_string() }),
            })
        };
        // wiki（命中片段化，与 wiki_search 同口径）
        let wik = scope_of("wiki");
        let wiki_fut = async {
            if !wik {
                return None;
            }
            let lib = match engram_core::wiki::libraries::resolve(&self.state.pool, None).await {
                Ok(l) => l,
                Err(e) => return Some(json!({ "error": e.to_string() })),
            };
            let r = wiki::svc(&self.state).search(lib, &q, max).await;
            Some(match r {
                Ok(pages) => json!(
                    pages
                        .iter()
                        .map(|pg| {
                            wiki::snippet_page(serde_json::to_value(pg).unwrap_or(json!({})), &q)
                        })
                        .collect::<Vec<_>>()
                ),
                Err(e) => json!({ "error": e.to_string() }),
            })
        };
        // skills（名字/描述命中，标题 + 描述即可定位）
        let sk = scope_of("skills");
        let skills_fut = async {
            if !sk {
                return None;
            }
            let pattern = format!("%{q}%");
            match engram_storage::repo::skills::list_skills(
                &self.state.pool,
                Some(pattern),
                None,
                None,
            )
            .await
            {
                Ok(rows) => Some(json!(rows
                .iter()
                .take(max as usize)
                .map(|s| json!({"slug": s.slug, "name": s.name, "description": s.description}))
                .collect::<Vec<_>>())),
                Err(e) => Some(json!({ "error": e.to_string() })),
            }
        };
        // todos（标题/正文子串）
        let td = scope_of("todos");
        let todos_fut = async {
            if !td {
                return None;
            }
            match todo_svc(&self.state)
                .list(None, None, None, None, Some(&q), None, None, max)
                .await
            {
                Ok(rows) => Some(json!(
                    rows.iter()
                        .map(|t| json!({"id": t.id, "title": t.title, "status": t.status}))
                        .collect::<Vec<_>>()
                )),
                Err(e) => Some(json!({ "error": e.to_string() })),
            }
        };
        let (mem_r, wiki_r, skills_r, todos_r) =
            tokio::join!(mem_fut, wiki_fut, skills_fut, todos_fut);

        // projects（项目名/描述命中 + 各项目文档按行检索，总量封顶）
        let mut projects_val: Option<serde_json::Value> = None;
        if scope_of("project") {
            let mut hits: Vec<serde_json::Value> = Vec::new();
            match self.svc_project().list_projects(None).await {
                Ok(projects) => {
                    let ql = q.to_lowercase();
                    'outer: for pr in projects.iter().take(20) {
                        let desc = pr.description.as_deref().unwrap_or("").to_lowercase();
                        let name_hit = pr.name.to_lowercase().contains(&ql);
                        let desc_hit = desc.contains(&ql);
                        if (name_hit || desc_hit) && hits.len() < max as usize {
                            hits.push(json!({
                                "project": pr.name, "match": "项目名/描述",
                                "status": pr.status,
                            }));
                        }
                        if let Ok(doc_hits) =
                            self.svc_project().search_doc_lines(pr.id, &q, 2).await
                        {
                            for h in doc_hits {
                                if hits.len() >= max as usize + 2 {
                                    break 'outer;
                                }
                                hits.push(json!({
                                    "project": pr.name, "match": "文档行",
                                    "title": h.title, "line": h.line, "text": h.text,
                                }));
                            }
                        }
                    }
                    projects_val = Some(json!(hits));
                }
                Err(e) => projects_val = Some(json!({ "error": e.to_string() })),
            }
        }

        let mut out = json!({
            "query": q,
            "note": "各域 top-k 摘要——精确/过滤检索用单域工具；wiki 全文 get_page，memory 原文 get_session，技能全文 skills get",
        });
        if let Some(v) = &mem_r {
            out["memory"] = v.clone();
        }
        if let Some(v) = &wiki_r {
            out["wiki"] = v.clone();
        }
        if let Some(v) = &skills_r {
            out["skills"] = v.clone();
        }
        if let Some(v) = &todos_r {
            out["todos"] = v.clone();
        }
        if let Some(v) = &projects_val {
            out["projects"] = v.clone();
        }

        // R6：rerank=true 时五域命中合并 top-10 交 LLM 精排，附 reranked 视图（失败降级为无此字段）
        if params.0.rerank == Some(true) {
            let mut candidates: Vec<UnifiedHit> = Vec::new();
            let push_arr =
                |domain: &str, arr: &serde_json::Value, candidates: &mut Vec<UnifiedHit>| {
                    if let Some(items) = arr.as_array() {
                        for it in items {
                            let title = it
                                .get("title")
                                .or_else(|| it.get("name"))
                                .or_else(|| it.get("slug"))
                                .and_then(|x| x.as_str())
                                .map(|s| s.to_string());
                            let snippet = it
                                .get("snippet")
                                .or_else(|| it.get("description"))
                                .or_else(|| it.get("text"))
                                .and_then(|x| x.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_default();
                            if title.is_none() && snippet.is_empty() {
                                continue;
                            }
                            candidates.push(UnifiedHit {
                                domain: domain.to_string(),
                                id: uuid::Uuid::now_v7(),
                                title,
                                snippet,
                                score: 0.0,
                                extra: serde_json::json!({}),
                            });
                        }
                    }
                };
            if let Some(v) = mem_r.as_ref() {
                if let Some(l1) = v.get("l1") {
                    push_arr("memory", l1, &mut candidates);
                }
                if let Some(l2) = v.get("l2") {
                    push_arr("memory", l2, &mut candidates);
                }
            }
            if let Some(v) = wiki_r.as_ref() {
                push_arr("wiki", v, &mut candidates);
            }
            if let Some(v) = skills_r.as_ref() {
                push_arr("skills", v, &mut candidates);
            }
            if let Some(v) = todos_r.as_ref() {
                push_arr("todos", v, &mut candidates);
            }
            if let Some(v) = projects_val.as_ref() {
                push_arr("projects", v, &mut candidates);
            }

            let top = candidates.len().min(10);
            if top > 1 {
                match engram_core::unified::rerank_hits(&self.state.llm(), &q, &candidates[..top])
                    .await
                {
                    Ok(order) => {
                        let reranked: Vec<serde_json::Value> = order
                            .into_iter()
                            .filter_map(|i| candidates.get(i))
                            .map(|h| {
                                json!({
                                    "domain": h.domain,
                                    "title": h.title,
                                    "snippet": h.snippet,
                                })
                            })
                            .collect();
                        out["reranked"] = json!(reranked);
                        out["note"] = json!(
                            "各域 top-k 摘要 + reranked=LLM 精排序（跨域）；精确检索用单域工具"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "search_all rerank 失败——仅返回分组摘要");
                    }
                }
            }
        }

        ok_json(out)
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_search_all() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::search_all_router()
}

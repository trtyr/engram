//! search_all 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

/// 跨域全局检索（R 报告 P1-8：把 6 次单域搜索并成 1 次）。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SearchAllParams {
    /// 检索词（各域同词并发检索）
    #[schemars(
        description = "检索词。对 key 有 scope 的域并发检索（memory/wiki/todos/projects）。"
    )]
    pub query: String,
    /// R6：可选 LLM 精排（默认关）——开时四域命中合并 top-10 交 LLM 重排，响应附 reranked 视图
    #[schemars(
        description = "可选：LLM 精排（默认关）。开时四域命中合并 top-10 交 LLM 重排，响应附 reranked 视图（LLM 失败降级原分组）。"
    )]
    pub rerank: Option<bool>,
    /// 每域返回条数（默认 3）
    #[schemars(description = "每域返回条数上限，默认 3。结果只有摘要——精确检索请用单域工具。")]
    pub max_per_domain: Option<i64>,
}

#[tool_router(router = search_all_router)]
impl EngramMcpServer {
    // ---------- 跨域全局检索（常驻工具之一；不属单一 scope，按 key 实际 scope 分域执行） ----------

    /// 全局检索（R 报告 P1-8）：一次查询并发打 memory/wiki/todos/projects 四域，
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
        if !["memory", "wiki", "todos", "project"]
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
                .search(&q, &["l1", "entities"], max, false, None, None, None)
                .await;
            Some(match r {
                Ok(r) => json!({
                    "l1": r.l1.iter().map(|h| json!({"id": h.id, "score": h.score, "snippet": h.snippet})).collect::<Vec<_>>(),
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
        // EN-252：skills 域裁撤——search_all 不再并发 skills
        // todos（标题/正文子串）
        let td = scope_of("todos");
        let todos_fut = async {
            if !td {
                return None;
            }
            match todo_svc(&self.state)
                .list(None, None, None, Some(&q), None, None, max)
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
        let (mem_r, wiki_r, todos_r) = tokio::join!(mem_fut, wiki_fut, todos_fut);

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
            "note": "各域 top-k 摘要——精确/过滤检索用单域工具；wiki 全文 get_page，memory 原文 get_session",
        });
        if let Some(v) = &mem_r {
            out["memory"] = v.clone();
        }
        if let Some(v) = &wiki_r {
            out["wiki"] = v.clone();
        }
        if let Some(v) = &todos_r {
            out["todos"] = v.clone();
        }
        if let Some(v) = &projects_val {
            out["projects"] = v.clone();
        }

        // R6：rerank=true 时四域命中合并 top-10 交 LLM 精排，附 reranked 视图（失败降级为无此字段）
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
                                // P019-M5：读真实分数（旧实现恒 0.0——rerank 候选集成了拼接顺序，「top-10」实为「任意 10」）
                                score: it.get("score").and_then(|x| x.as_f64()).unwrap_or(0.0),
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
            if let Some(v) = todos_r.as_ref() {
                push_arr("todos", v, &mut candidates);
            }
            if let Some(v) = projects_val.as_ref() {
                push_arr("projects", v, &mut candidates);
            }

            // P019-M5：候选按分数降序后再取前 10——「top-10」名副其实
            let top_set = top_rerank_candidates(candidates, 10);
            let top = top_set.len();
            if top > 1 {
                match engram_core::unified::rerank_hits(&self.state.llm(), &q, &top_set).await {
                    Ok(order) => {
                        // P019-M5：order 合法性校验（对齐 unified.rs P018-T003：长度+越界+查重），
                        // 不合法丢弃 reranked 视图（out 里只剩分组摘要）
                        if engram_core::unified::is_valid_order(&order, top) {
                            let reranked: Vec<serde_json::Value> = order
                                .into_iter()
                                .filter_map(|i| top_set.get(i))
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
                        } else {
                            tracing::warn!("search_all rerank：order 长度/越界/重复，降级分组摘要");
                        }
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

/// P019-M5：rerank 候选集——按分数降序取前 n（原实现拼接顺序直接截前 10，「top-10」实为「任意 10」）。
fn top_rerank_candidates(mut hits: Vec<UnifiedHit>, n: usize) -> Vec<UnifiedHit> {
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(n);
    hits
}

#[cfg(test)]
mod top_candidates_tests {
    use super::*;

    fn hit(score: f64, title: &str) -> UnifiedHit {
        UnifiedHit {
            domain: "memory".into(),
            id: uuid::Uuid::now_v7(),
            title: Some(title.into()),
            snippet: String::new(),
            score,
            extra: serde_json::json!({}),
        }
    }

    #[test]
    fn candidates_sorted_by_score_desc_and_truncated() {
        let cands = vec![hit(0.1, "low"), hit(0.9, "hi"), hit(0.5, "mid")];
        let top = top_rerank_candidates(cands, 2);
        let titles: Vec<_> = top.iter().map(|h| h.title.as_deref().unwrap()).collect();
        assert_eq!(titles, vec!["hi", "mid"], "应按分数降序取前 2");
    }

    #[test]
    fn fewer_than_n_all_kept() {
        let cands = vec![hit(0.2, "a"), hit(0.8, "b")];
        assert_eq!(top_rerank_candidates(cands, 10).len(), 2);
    }

    /// P019-M5：rerank order 含重复索引必须拒——search_all 复用 unified::is_valid_order
    /// （P018-T003 守卫），此处锁定 search_all 复用点的契约。
    #[test]
    fn search_all_order_guard_rejects_duplicates() {
        assert!(!engram_core::unified::is_valid_order(&[0, 0, 1], 3));
        assert!(engram_core::unified::is_valid_order(&[2, 1, 0], 3));
    }
}

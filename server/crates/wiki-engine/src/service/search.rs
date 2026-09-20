//! `service` 的实现切片（架构治理 2026-09-20：自 service.rs 纯搬移，零行为变化）。

use super::*;

impl WikiService {
    /// 链接图（节点 = 页面，边 = wikilink；含 Louvain 社区 + 凝聚度；库内）。
    pub async fn graph(&self, lib: Uuid) -> Result<GraphDto, WikiError> {
        self.graph_filtered(lib, None, None, None).await
    }

    /// 规模化 task-5：图谱子图过滤——按 community（Louvain 全图编号）/ folder 前缀 /
    /// page_type 裁剪。folder/page_type 在 SQL 层过滤（万页少拉）；community 为运行时
    /// 计算，先分区再按编号裁剪节点与边，communities 元数据保留全图编号子集
    /// （前端「从全图下拉选社区 N」→ 子图里社区 ID 语义一致）。
    pub async fn graph_filtered(
        &self,
        lib: Uuid,
        community: Option<usize>,
        folder: Option<&str>,
        page_type: Option<&str>,
    ) -> Result<GraphDto, WikiError> {
        // D23：排除系统 log 页（list_pages 不可见，图里也不该出现——否则节点无法溯源）
        let nodes: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT slug, COALESCE(frontmatter->>'title', slug), page_type, folder FROM wiki_pages \
         WHERE page_type <> 'log' AND library_id = $1 \
           AND ($2::text IS NULL OR folder LIKE $2 || '%') \
           AND ($3::text IS NULL OR page_type = $3)",
        )
        .bind(lib)
        .bind(folder)
        .bind(page_type)
        .fetch_all(&self.pool)
        .await?;
        // 边随节点过滤：任一端是 log 页的边一并剔除（防悬空引用进社区发现）；边与两端都限库内。
        // 规模化 task-5 修正（万页压测）：EXISTS 子查询 + 可选参数 OR 模式在万页下计划劣化
        // （page_type 过滤 151s）——改为仅按库拉边（54869 行内存过滤秒级），节点集条件
        // （log 排除/folder/page_type）由下方的 slug 集合过滤统一承接。
        let edges: Vec<(String, String, f32)> = sqlx::query_as(
            "SELECT l.from_slug, l.to_slug, l.weight FROM wiki_links l \
         WHERE l.library_id = $1",
        )
        .bind(lib)
        .fetch_all(&self.pool)
        .await?;
        let node_slugs: Vec<String> = nodes.iter().map(|(s, _, _, _)| s.clone()).collect();
        // 社区发现（万页保护 + 成员映射）——纯 CPU 计算在助手内 spawn_blocking 隔离
        let (comms, cohesion, members) =
            discover_graph_communities(&node_slugs, &edges, community).await?;
        // community 后置裁剪：保留全图社区编号 == 选中值的节点，边随节点过滤
        let kept: std::collections::HashSet<String> = match community {
            Some(n) => nodes
                .iter()
                .filter(|(s, _, _, _)| comms.get(s.as_str()).copied().unwrap_or(0) == n)
                .map(|(s, _, _, _)| s.clone())
                .collect(),
            None => node_slugs.iter().cloned().collect(),
        };
        let comm_meta = |id: usize| {
            let m = members.get(&id);
            let size = m.map_or(0, |v| v.len());
            CommunityInfo {
                id,
                top_slug: m
                    .and_then(|v| v.first().copied())
                    .unwrap_or_default()
                    .to_string(),
                size,
                cohesion: cohesion.get(&id).copied().unwrap_or(0.0),
                sparse: size >= crate::community::SPARSE_MIN_SIZE
                    && cohesion.get(&id).copied().unwrap_or(0.0)
                        < crate::community::SPARSE_COHESION,
            }
        };
        Ok(GraphDto {
            communities: match community {
                Some(n) => vec![comm_meta(n)],
                None => cohesion.keys().copied().map(comm_meta).collect(),
            },
            nodes: nodes
                .into_iter()
                .filter(|(s, _, _, _)| kept.contains(s.as_str()))
                .map(|(slug, title, page_type, folder)| GraphNode {
                    community: comms.get(slug.as_str()).copied().unwrap_or(0),
                    slug,
                    title,
                    page_type,
                    folder,
                })
                .collect(),
            edges: edges
                .into_iter()
                .filter(|(f, t, _)| kept.contains(f.as_str()) && kept.contains(t.as_str()))
                .map(|(from_slug, to_slug, weight)| GraphEdge {
                    from_slug,
                    to_slug,
                    weight,
                })
                .collect(),
        })
    }

    /// Wiki 检索（FTS + 向量 RRF 融合；W2：向量通道落地；库内检索）。
    /// purpose 注入：检索走 LLM 时（AI 客户端读 query_context.purpose）提供方向意图——对齐 llm_wiki 的 query 注入。
    pub async fn search(
        &self,
        lib: Uuid,
        query: &str,
        limit: i64,
    ) -> Result<Vec<WikiPageDto>, WikiError> {
        self.search_opts(lib, query, limit, false).await
    }

    /// 批次④：带 LLM rerank 精排的检索（费用不敏感拍板；LLM 失败降级原序）。
    pub async fn search_reranked(
        &self,
        lib: Uuid,
        query: &str,
        limit: i64,
    ) -> Result<Vec<WikiPageDto>, WikiError> {
        self.search_opts(lib, query, limit, true).await
    }

    pub(super) async fn search_opts(
        &self,
        lib: Uuid,
        query: &str,
        limit: i64,
        rerank: bool,
    ) -> Result<Vec<WikiPageDto>, WikiError> {
        // K7：单字/纯标点无 token → 短路空结果（不再空跑 to_tsquery）
        if !engram_search::tokenize::has_query_tokens(query) {
            return Ok(vec![]);
        }
        // 规模化 task-3：查询类型路由——零成本规则分类决定 FTS/向量通道权重
        let kind = classify_query(query);
        let (w_fts, w_vec) = kind.rrf_weights();
        tracing::info!(kind = ?kind, query = %query, w_fts, w_vec, "wiki 检索路由");
        let tsq = tsv_query_smart_wiki(query, 3);
        let limit = limit.min(50);

        // W2：查询向量（无 provider / 嵌入失败 → None → 纯 FTS）；L6：经记账门面
        let qv: Option<Vec<f32>> = self
            .registry
            .embed_for(
                Purpose::Embed,
                vec![query.to_string()],
                Some(engram_distill::llm_port::embedding_dimensions()),
                None,
            )
            .await
            .ok()
            .and_then(|r| r.embeddings.first().cloned());

        // 批次⑤：初召回扩到 2×limit（给图扩展留空间），召回后沿双链 2-hop 带衰减重排
        let fetch_n = (limit * 2).min(100);
        // 批次②：真实 RRF 融合分（双通道 rank 归一和）——查询日志的区分度信号；

        let (mut pages, _top, direct_hits) = self
            .retrieve_candidates(lib, &tsq, qv, fetch_n, w_fts, w_vec, limit)
            .await?;

        // 批次② 查询日志飞轮：每次检索 UPSERT（直接命中数=0 才记零命中——图扩展补充层
        // 会让最终结果永不为空，零命中必须看直接召回；低分看真实 RRF 融合分）。
        // best-effort——记录失败不影响检索结果。
        if let Err(e) = log_query(&self.pool, lib, query, direct_hits, _top).await {
            tracing::warn!(error = %e, "检索日志记录失败（不影响检索结果）");
        }

        // 批次④ LLM rerank 精排：top-20 交模型重排（单次不重试——检索热路径；失败/越界降级原序）
        if rerank
            && pages.len() > 1
            && let Some(llm) = &self.llm
        {
            apply_llm_rerank(llm, query, &mut pages).await;
        }
        Ok(pages)
    }

    /// 初召回：FTS + ANN 双通道 RRF 融合（无查询向量时 FTS-only 降级）+ 图扩展重排。
    /// 返回 `(页面, 最高 RRF 分, 直接命中数)`——零命中判定必须看直接召回。
    #[allow(clippy::too_many_arguments)]
    async fn retrieve_candidates(
        &self,
        lib: Uuid,
        tsq: &str,
        qv: Option<Vec<f32>>,
        fetch_n: i64,
        w_fts: f64,
        w_vec: f64,
        limit: i64,
    ) -> Result<(Vec<WikiPageDto>, Option<f64>, usize), WikiError> {
        let result = if let Some(qv) = qv {
            // FTS + ANN 双候选 + RRF 融合（与 wiki 文档同款模式）；
            // CTE 与外层都按 library_id 过滤——slug 跨库可重名，外层不过滤会串库
            let raw = sqlx::query(
            "WITH fts AS (SELECT slug, ROW_NUMBER() OVER (ORDER BY ts_rank(tsv, q) DESC) AS rank \
             FROM wiki_pages, to_tsquery('simple', $2) q WHERE tsv @@ q AND library_id = $1 \
               AND page_type NOT IN ('index','log','overview') LIMIT 100), \
             vec AS (SELECT slug, ROW_NUMBER() OVER (ORDER BY embedding <=> $3) AS rank \
             FROM wiki_pages WHERE embedding IS NOT NULL AND library_id = $1 \
               AND page_type NOT IN ('index','log','overview') LIMIT 100) \
             SELECT p.*, (COALESCE($5/(60 + fts.rank), 0) + COALESCE($6/(60 + vec.rank), 0))::float8 AS rrf_score \
             FROM wiki_pages p \
             LEFT JOIN fts ON fts.slug = p.slug \
             LEFT JOIN vec ON vec.slug = p.slug \
             WHERE p.library_id = $1 AND (fts.slug IS NOT NULL OR vec.slug IS NOT NULL) \
             ORDER BY (COALESCE($5/(60 + fts.rank), 0) + COALESCE($6/(60 + vec.rank), 0)) DESC \
             LIMIT $4",
        )
        .bind(lib)
        .bind(tsq)
        .bind(pgvector::Vector::from(qv))
        .bind(fetch_n)
        .bind(w_fts)
        .bind(w_vec)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WikiError::Storage(e.to_string()))?;
            let mut rows: Vec<WikiPageDto> = Vec::with_capacity(raw.len());
            let mut top_rrf: Option<f64> = None;
            for (i, r) in raw.iter().enumerate() {
                if i == 0 {
                    top_rrf = r.try_get::<f64, _>("rrf_score").ok();
                }
                rows.push(WikiPageDto::from_row(r).map_err(|e| WikiError::Storage(e.to_string()))?);
            }
            let direct_hits = rows.len();
            let (out, _graph_top) = self.rerank_with_graph(lib, rows, limit).await?;
            (out, top_rrf, direct_hits)
        } else {
            let rows = sqlx::query_as::<_, WikiPageDto>(
                "SELECT * FROM wiki_pages, to_tsquery('simple', $2) q \
             WHERE tsv @@ q AND library_id = $1 \
               AND page_type NOT IN ('index','log','overview') \
             ORDER BY ts_rank(tsv, q) DESC LIMIT $3",
            )
            .bind(lib)
            .bind(tsq)
            .bind(fetch_n)
            .fetch_all(&self.pool)
            .await?;
            // FTS-only 降级路径：ts_rank 无跨查询可比量级，不判低分（仅零命中判定有效）
            let direct_hits = rows.len();
            let (out, _graph_top) = self.rerank_with_graph(lib, rows, limit).await?;
            (out, None, direct_hits)
        };
        Ok(result)
    }

    /// 图扩展重排（批次⑤）：初召回（已按 RRF/ts_rank 排序）→ 沿双链 2-hop 带衰减扩展 →
    /// 合并重排截 limit。seed 分用召回顺序近似 RRF（1/(60+rank)，与真实 RRF 分单调一致）；
    /// 源重叠等 4 信号已在 wiki_links.weight（relevance::rebuild_weights 每次 ingest 后重算），
    /// 扩展 bonus 因此天然带源重叠权重。
    pub(super) async fn rerank_with_graph(
        &self,
        lib: Uuid,
        rows: Vec<WikiPageDto>,
        limit: i64,
    ) -> Result<(Vec<WikiPageDto>, Option<f64>), WikiError> {
        let seeds: Vec<(String, f64)> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.slug.clone(), 1.0 / (60.0 + i as f64 + 1.0)))
            .collect();
        let seed_slugs: Vec<String> = seeds.iter().map(|(s, _)| s.clone()).collect();
        let adjacency = self.load_adjacency(lib, &seed_slugs, 2).await?;
        let expanded = crate::relevance::graph_expand_scores(&seeds, &adjacency);
        // 直接命中保留 RRF 序；扩展分 tie-break 实测调参史（基准集为尺）：
        // 0.05× → 枢纽页霸榜雪崩（92.9%→7.1%）；0.02× → MRR 0.783 仍低于基线 0.789（微扰超阈值）；
        // **0.0× → 精确持平基线**（审计缺陷①修正：不降级是硬约束）。图扩展的主价值在补充层
        // （初召回漏掉的双链强关联页）与源重叠边权（rebuild_weights），不在改排直接命中；
        // 需要质量上限的场景用 rerank（实测 100%/0.918）。
        const BONUS_SCALE: f64 = 0.0;
        let mut scored: std::collections::HashMap<String, f64> = seeds.into_iter().collect();
        for (slug, bonus) in &expanded {
            if let Some(s) = scored.get_mut(slug) {
                *s += bonus * BONUS_SCALE;
            }
        }
        let mut primary: Vec<(String, f64)> = scored.into_iter().collect();
        primary.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        // 第二梯队：图扩展捞回的补充页（语义检索漏掉但双链强关联），排直接命中之后
        let primary_set: std::collections::HashSet<&String> =
            primary.iter().map(|(s, _)| s).collect();
        let mut secondary: Vec<(String, f64)> = expanded
            .into_iter()
            .filter(|(s, _)| !primary_set.contains(s))
            .collect();
        secondary.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let mut order: Vec<(String, f64)> = primary;
        order.extend(secondary);
        order.truncate(limit.max(0) as usize);
        if order.is_empty() {
            return Ok((vec![], None));
        }
        let top_score = order.first().map(|(_, s)| *s);
        // 回查 DTO（图扩展捞回的页不在初召回 rows 里；带系统页守卫——扩展不捞 index/log/overview）
        let slugs: Vec<String> = order.iter().map(|(s, _)| s.clone()).collect();
        let dtos: Vec<WikiPageDto> = sqlx::query_as(
            "SELECT * FROM wiki_pages WHERE slug = ANY($1) AND library_id = $2 \
         AND page_type NOT IN ('index','log','overview')",
        )
        .bind(&slugs)
        .bind(lib)
        .fetch_all(&self.pool)
        .await?;
        let pos: std::collections::HashMap<String, usize> = order
            .iter()
            .enumerate()
            .map(|(i, (s, _))| (s.clone(), i))
            .collect();
        let mut out: Vec<WikiPageDto> = dtos
            .into_iter()
            .filter(|d| pos.contains_key(&d.slug))
            .collect();
        out.sort_by_key(|d| pos.get(&d.slug).copied().unwrap_or(usize::MAX));
        Ok((out, top_score))
    }

    /// 库内双链无向邻接表（图扩展用；百页级内存直载）。
    /// weight 列是 FLOAT4——SQL 层 ::float8 转，避免 sqlx 运行时解码类型错配（2026-09-19 实测炸点）。
    /// 邻域邻接（规模化 2026-09-20）：只加载 seed 的 `hops` 跳邻域内的边。
    /// 此前是每检索一次就全库拉边（万页 54869 行 ≈1.1s/次 + 全量建图）；而图扩展只在
    /// 2-hop 内给 bonus，更远的节点无论如何拿不到加分——结果不变，只省 IO。
    pub(super) async fn load_adjacency(
        &self,
        lib: Uuid,
        seeds: &[String],
        hops: usize,
    ) -> Result<std::collections::HashMap<String, Vec<(String, f64)>>, WikiError> {
        let mut adj: std::collections::HashMap<String, Vec<(String, f64)>> =
            std::collections::HashMap::new();
        let mut seen: std::collections::HashSet<String> = seeds.iter().cloned().collect();
        let mut frontier: Vec<String> = seeds.to_vec();
        for _ in 0..hops.max(1) {
            if frontier.is_empty() {
                break;
            }
            let edges: Vec<(String, String, f64)> = sqlx::query_as(
                "SELECT from_slug, to_slug, weight::float8 FROM wiki_links \
             WHERE library_id = $1 AND (from_slug = ANY($2) OR to_slug = ANY($2))",
            )
            .bind(lib)
            .bind(&frontier)
            .fetch_all(&self.pool)
            .await?;
            let mut next: Vec<String> = Vec::new();
            for (f, t, w) in edges {
                adj.entry(f.clone()).or_default().push((t.clone(), w));
                adj.entry(t.clone()).or_default().push((f.clone(), w));
                if seen.insert(f.clone()) {
                    next.push(f);
                }
                if seen.insert(t.clone()) {
                    next.push(t);
                }
            }
            frontier = next;
        }
        Ok(adj)
    }

    /// 批次② 缺口清单：零命中/低分查询（织入方向与 Deep Research 的输入）。
    pub async fn query_gaps(&self, lib: Uuid, limit: i64) -> Result<Vec<QueryGapDto>, WikiError> {
        Ok(sqlx::query_as::<_, QueryGapDto>(
            "SELECT query, calls, zero_calls, low_calls, last_top_score, last_queried_at \
         FROM wiki_query_log WHERE library_id = $1 AND (zero_calls > 0 OR low_calls > 0) \
         ORDER BY GREATEST(zero_calls, low_calls) DESC, last_queried_at DESC LIMIT $2",
        )
        .bind(lib)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// 检索上下文包（query 时 purpose 注入的载体）：purpose + 命中页面，
    /// AI 客户端把 purpose 作为 system context 前缀使用（purpose 取该库的）。
    /// 批次④：rerank 参数化——HTTP/MCP 请求级开关（默认 false，检索框速度优先）。
    pub async fn search_with_purpose(
        &self,
        lib: Uuid,
        query: &str,
        limit: i64,
        rerank: bool,
    ) -> Result<serde_json::Value, WikiError> {
        let pages = if rerank {
            self.search_reranked(lib, query, limit).await?
        } else {
            self.search(lib, query, limit).await?
        };
        // W-10（2026-09-04）：未设 purpose 时返回 null，与 GET /wiki/purpose 一致；
        // 不再用 purpose_context 的默认模板——「读当前设置」与「注入 LLM」语义分开。
        let purpose = crate::purpose::get_purpose(&self.pool, lib)
            .await
            .ok()
            .flatten();
        Ok(serde_json::json!({
            "purpose": purpose,
            "pages": pages,
        }))
    }
}

/// 社区发现 + 成员映射（万页保护：Louvain 是纯 CPU 计算——万级全图 >120s 且阻塞 tokio
/// worker，超限时降级跳过、阈值内也 spawn_blocking 隔离；超限的 community 过滤直接拒绝）。
/// 返回 `(社区编号表, 凝聚度表, 成员映射)`；成员借用 `node_slugs` 的生命周期。
async fn discover_graph_communities<'a>(
    node_slugs: &'a [String],
    edges: &[(String, String, f32)],
    community: Option<usize>,
) -> Result<
    (
        std::collections::HashMap<String, usize>,
        std::collections::HashMap<usize, f64>,
        std::collections::HashMap<usize, Vec<&'a str>>,
    ),
    WikiError,
> {
    // 万页保护（压测实测三个发现）：Louvain 是纯 CPU 计算——(1) 万级全图 >120s；
    // (2) 4077 节点子图在 debug 下分钟级且阻塞 tokio worker（连接池 acquire 等 638s、
    // 实例整体无响应）；(3) 阈值内也必须 spawn_blocking 隔离。超限的 community 过滤
    // 直接拒绝（引导先 SQL 层收窄）；全量请求降级跳过社区计算（community=0、空列表）。
    const LOUVAIN_MAX_NODES: usize = crate::community::LOUVAIN_MAX_NODES;
    let over_limit = node_slugs.len() > LOUVAIN_MAX_NODES;
    if over_limit && community.is_some() {
        return Err(WikiError::BadRequest(format!(
            "子图 {} 节点超过社区计算上限 {}——请先用 folder/page_type 参数收窄后再按社区过滤",
            node_slugs.len(),
            LOUVAIN_MAX_NODES
        )));
    }
    let e64: Vec<(String, String, f64)> = edges
        .iter()
        .map(|(f, t, w)| (f.clone(), t.clone(), *w as f64))
        .collect();
    let (comms, cohesion): (
        std::collections::HashMap<String, usize>,
        std::collections::HashMap<usize, f64>,
    ) = if over_limit {
        Default::default()
    } else {
        let slugs = node_slugs.to_vec();
        let e64c = e64.clone();
        tokio::task::spawn_blocking(move || {
            let comms = crate::community::louvain_communities(&slugs, &e64c);
            let cohesion = crate::community::community_cohesion(&slugs, &e64c, &comms);
            // louvain 返回借用 key（&str 借 slugs）——转自有 String 后才能跨闭包返回
            let owned: std::collections::HashMap<String, usize> =
                comms.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
            (owned, cohesion)
        })
        .await
        .unwrap_or_default()
    };
    let mut members: std::collections::HashMap<usize, Vec<&'a str>> =
        std::collections::HashMap::new();
    for slug in node_slugs {
        members
            .entry(comms.get(slug.as_str()).copied().unwrap_or(0))
            .or_default()
            .push(slug);
    }
    Ok((comms, cohesion, members))
}

/// LLM rerank 精排：top-20 交模型重排（Purpose::SearchRerank；单次不重试——检索热路径；
/// 失败或索引越界时降级原序，对齐后端增强线 R6 语义）。
async fn apply_llm_rerank(llm: &crate::service::LlmRef, query: &str, pages: &mut Vec<WikiPageDto>) {
    let top: Vec<WikiPageDto> = pages.iter().take(20).cloned().collect();
    let listing: String = top
        .iter()
        .enumerate()
        .map(|(i, p)| {
            format!(
                "[{}] {}\n{}",
                i,
                p.title,
                p.content.chars().take(120).collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join("\n---\n");
    let system = "你是检索重排序员。给定查询与候选列表（每项带 [索引]），按与查询的相关性从高到低输出索引。只输出 JSON：{\"order\": [索引数组]}，必须包含全部索引且不重复。";
    let user = format!("查询：{query}\n\n候选：\n{listing}");
    match llm
        .chat_json(
            engram_llm::types::Purpose::SearchRerank,
            system,
            &user,
            Uuid::now_v7(),
        )
        .await
    {
        Ok(v) => {
            let idx: Vec<usize> = v
                .get("order")
                .and_then(|o| o.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_u64().map(|n| n as usize))
                        .collect()
                })
                .unwrap_or_default();
            let n = top.len();
            if idx.len() == n && idx.iter().all(|&i| i < n) {
                let mut reordered: Vec<WikiPageDto> = idx.iter().map(|&i| top[i].clone()).collect();
                if pages.len() > n {
                    reordered.extend(pages.drain(n..));
                }
                *pages = reordered;
            } else {
                tracing::warn!("wiki rerank：order 长度/索引越界，降级原序");
            }
        }
        Err(e) => tracing::warn!(error = %e, "wiki rerank 失败，降级原序"),
    }
}

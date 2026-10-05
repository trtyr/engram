//! `memory` 的实现切片（架构治理 2026-09-20：自 memory.rs 纯搬移，零行为变化）。

use super::*;

impl MemoryService {
    /// 记忆域缺失向量统计（重嵌修复入口的状态面）。P015 场景层退役：只有 atoms。
    pub async fn embedding_status(&self) -> Result<EmbeddingStatus, MemoryError> {
        let atoms_missing = repo::embedding_missing_counts(&self.pool).await?;
        Ok(EmbeddingStatus { atoms_missing })
    }

    /// 入队重嵌（换 embedding 供应商后的修复路径；job 见 distill::reembed）。
    pub async fn reembed(&self) -> Result<(), MemoryError> {
        self.queue
            .enqueue(
                JobTemplate::new("reembed_memory")
                    .with_idempotency_key(format!("reembed-memory-{}", Uuid::now_v7().simple())),
            )
            .await
            .map(|_| ())
            .map_err(|e| MemoryError::Storage(format!("入队失败: {e}")))
    }

    // ---------- 检索 ----------

    pub(super) async fn try_embed(&self, texts: &[String]) -> Option<Vec<Vec<f32>>> {
        // L6：经记账门面（查询/场景嵌入计入用量）。
        // v2 遗留修复：embed 间歇失败（上游限流/抖动）此前被 .ok() 静默吞掉——
        // 查询无提示地降级为纯 FTS、文档静默缺向量。现在重试 3 次（退避）+ 失败显式记日志。
        for attempt in 1..=3 {
            match self
                .registry
                .embed_for(
                    Purpose::Embed,
                    texts.to_vec(),
                    Some(engram_distill::llm_port::embedding_dimensions()),
                    None,
                )
                .await
            {
                Ok(r) => return Some(r.embeddings),
                Err(e) => {
                    if attempt < 3 {
                        tracing::warn!(error = %e, attempt, "embed 失败，退避重试");
                        tokio::time::sleep(std::time::Duration::from_millis(400 * attempt as u64))
                            .await;
                    } else {
                        tracing::error!(
                            error = %e,
                            texts = texts.len(),
                            "embed 重试耗尽——查询降级为纯 FTS / 文档暂缺向量（后续 reembed 可补）"
                        );
                    }
                }
            }
        }
        None
    }

    /// v2（Qwen3-Embedding）：查询侧指令包装。Qwen3-Embedding 是非对称检索模型，
    /// 官方要求查询带 task instruction（文档侧不带）——不带指令的跨语言/短查询
    /// 召回会明显退化（实测：英文 pet 查询在阈值内找不到已有橘猫记忆）。
    /// 指令经 AGENT_MEMORY_EMBED_QUERY_INSTRUCTION 配置；未设置 = 不包装（bge-m3 等对称模型）。
    /// 文档侧（atom/scene 内容嵌入）必须保持不包装。
    pub(super) async fn try_embed_query(&self, texts: &[String]) -> Option<Vec<Vec<f32>>> {
        match QUERY_INSTRUCTION.as_ref() {
            Some(instr) => {
                let wrapped: Vec<String> = texts
                    .iter()
                    .map(|t| format!("{instr}\nQuery: {t}"))
                    .collect();
                self.try_embed(&wrapped).await
            }
            None => self.try_embed(texts).await,
        }
    }

    /// B9 命中反馈：检索命中即异步回写 hit_count（best-effort，失败只记日志）。
    /// 不刷 updated_at——hit 是使用热度而非内容变化，避免扰动「最近更新」排序。
    /// P015 场景层退役：只有 atoms。
    pub(super) fn fire_hit_feedback(&self, ids: Vec<Uuid>) {
        if ids.is_empty() {
            return;
        }
        let pool = self.pool.clone();
        tokio::spawn(async move {
            if let Err(e) = repo::bump_hit_counts(&pool, &ids).await {
                tracing::warn!(error = %e, "hit_count 回写失败（不影响检索结果）");
            }
        });
    }

    /// 分层检索。无 embedding 通道时自动退化为纯 FTS。
    #[allow(clippy::too_many_arguments)]
    /// 2026-09-12 敏感口径放开：单用户系统全部记忆可见（用户拍板「全部放进来且能检索」）。
    /// sensitive 保留为**标记**（AtomDto.sensitive 字段照常返回），不再是隐身开关；
    /// reveal 参数移除——显式 hide 需求未来以 include_hidden 形态回归。
    pub async fn search(
        &self,
        query: &str,
        layers: &[&str],
        max_items: i64,
        no_feedback: bool,
        from: Option<chrono::DateTime<chrono::Utc>>,
        to: Option<chrono::DateTime<chrono::Utc>>,
        // P015：调用方自带查询向量（None = 内部 embed）。纯向量检索时代这是确定性测试与服务编排的通道。
        query_vec: Option<&[f32]>,
    ) -> Result<SearchResponse, MemoryError> {
        if max_items < 0 {
            return Err(MemoryError::BadRequest(format!(
                "max_items 不能为负（收到 {max_items}）"
            )));
        }
        let qv = match query_vec {
            Some(v) => Some(v.to_vec()),
            None => self
                .try_embed_query(&[query.to_string()])
                .await
                .and_then(|v| v.first().cloned()),
        };
        let all = layers.is_empty();
        let want_e = all || layers.contains(&"entities");
        let want_l1 = all || layers.contains(&"l1");

        // 实体：token 命中（名字加权）——主角先行；L1 原子（P015：l2/l3 层已随场景层退役）
        let (entities, l1) = self
            .collect_layers(query, qv.as_deref(), max_items, from, to, [want_e, want_l1])
            .await?;
        // B9：命中反馈（异步 best-effort，不阻塞返回）；no_feedback=true 跳过（B6 污染防护）
        if !no_feedback {
            self.fire_hit_feedback(l1.iter().map(|h| h.id).collect());
        }
        // T020：读取生命周期——只记元数据（层命中数/体量），查询正文不落日志
        self.emit_mem_log(
            "search",
            serde_json::json!({
                "query_chars": query.chars().count(),
                "hits": {
                    "entities": entities.len(),
                    "l1_atoms": l1.len(),
                },
                "no_feedback": no_feedback,
            }),
        )
        .await;
        Ok(SearchResponse {
            entities,
            l1,
            query: query.to_string(),
        })
    }

    /// 实体层检索（token 命中，名字加权）——主角先行。
    async fn entity_hits(
        &self,
        query: &str,
        max_items: i64,
    ) -> Result<Vec<engram_search::SearchHit>, MemoryError> {
        Ok(engram_search::search_entities(&self.pool, query, max_items)
            .await
            .map_err(StoreError::from)?)
    }

    /// 实体透镜（小预算 ~20%）：AI 冷启动要知道用户世界里都有谁。
    /// 有 query 走 token 相关（纯 jieba，不依赖向量）；无 query 按密度头部。best-effort。
    /// 返回 `(实体, 累计字符数, 是否截断)`。
    async fn entity_lens(
        &self,
        query: Option<&str>,
        ent_budget: usize,
        budget_chars: usize,
        chars_used: usize,
    ) -> Result<(Vec<EntityDto>, usize, bool), MemoryError> {
        let mut chars_used = chars_used;
        let mut truncated = false;
        let entity_ids: Vec<Uuid> = match query {
            Some(q) => engram_search::search_entities(&self.pool, q, ent_budget as i64)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|h| h.id)
                .collect(),
            None => self
                .list_entities(None)
                .await
                .unwrap_or_default()
                .into_iter()
                .take(ent_budget)
                .map(|e| e.id)
                .collect(),
        };
        let mut out_entities = Vec::new();
        if !entity_ids.is_empty() {
            let rows = repo::entities_by_ids(&self.pool, &entity_ids).await?;
            // 保持命中序（query 路径相关性优先；无 query 路径密度优先）
            let by_id: std::collections::HashMap<Uuid, EntityDto> =
                rows.into_iter().map(|e| (e.id, e)).collect();
            for id in entity_ids {
                if let Some(e) = by_id.get(&id).cloned() {
                    if count_json(&e, budget_chars, &mut chars_used, &mut truncated) {
                        out_entities.push(e);
                    } else {
                        truncated = true;
                        break;
                    }
                }
            }
        }

        Ok((out_entities, chars_used, truncated))
    }

    /// L1 原子层打包：有 query 按相关性 + 新鲜度混排（30 天半衰），无 query 取最近活跃；
    /// 过期原子（valid_until 已过）不注入。返回 `(原子, 累计字符数, 是否截断)`。
    #[allow(clippy::too_many_arguments)]
    async fn pack_atoms(
        &self,
        query: Option<&str>,
        qv: Option<&[f32]>,
        remaining: usize,
        budget_chars: usize,
        chars_used: usize,
    ) -> Result<(Vec<AtomDto>, usize, bool), MemoryError> {
        let mut chars_used = chars_used;
        let mut truncated = false;
        // L1：v2 修复（N3）——条数上限 = budget_items（各层独立预算，不再被 persona/场景/实体
        // 相减挤成 0）；字符预算仍全局统一裁剪
        let atoms: Vec<AtomDto> = match query {
            Some(q) => {
                let hits = search_atoms(
                    &self.pool,
                    q,
                    qv,
                    remaining as i64,
                    true, // sensitive 口径放开（2026-09-12）
                    None,
                    None,
                )
                .await
                .map_err(StoreError::from)?;
                let ids: Vec<Uuid> = hits.iter().map(|h| h.id).collect();
                if ids.is_empty() {
                    vec![]
                } else {
                    let score_of: std::collections::HashMap<Uuid, f64> =
                        hits.iter().map(|h| (h.id, h.score)).collect();
                    let mut fetched = repo::atoms_by_ids(&self.pool, &ids).await?;
                    // 过期原子不注入（phase-2）：valid_until 已过 = 真记性不递过期记忆
                    let now = chrono::Utc::now();
                    fetched.retain(|a| a.valid_until.map(|vu| vu > now).unwrap_or(true));
                    // P10 新鲜度混排：final = 相关分 × 时间衰减（30 天半衰）——
                    // 老记忆不再凭旧高分挤掉新记忆；无 query 路径本就按新→旧。
                    fetched.sort_by(|a, b| {
                        let f = |x: &AtomDto| {
                            let age = (now - x.created_at).num_days().max(0) as f64;
                            score_of.get(&x.id).copied().unwrap_or(0.0) * (-age / 30.0).exp()
                        };
                        f(b).partial_cmp(&f(a)).unwrap_or(std::cmp::Ordering::Equal)
                    });
                    fetched
                }
            }
            None => repo::recent_active_atoms(&self.pool, remaining as i64).await?,
        };
        let mut out_atoms = Vec::new();
        for a in atoms {
            if count_json(&a, budget_chars, &mut chars_used, &mut truncated) {
                out_atoms.push(a);
            } else {
                truncated = true;
                break;
            }
        }

        Ok((out_atoms, chars_used, truncated))
    }

    /// ILIKE 补漏与 KV 权威通道（工单「库里有一搜必有」）：
    /// - KV 是精确值唯一权威源——字面量命中**恒合并**进结果最前（FTS 噪音命中不应把权威值挤出结果）；
    /// - atoms ILIKE 兜底仅在双腿零命中时触发（补漏，不打扰正常排序）。
    async fn kv_and_literal_supplement(
        &self,
        query: &str,
        want_l1: bool,
        max_items: i64,
        l1: Vec<engram_search::SearchHit>,
    ) -> Result<Vec<engram_search::SearchHit>, MemoryError> {
        let mut l1 = l1;
        // ILIKE 补漏与 KV 权威通道（工单「库里有一搜必有」）：
        // - KV 是精确值唯一权威源——字面量命中**恒合并**进结果最前（FTS 噪音命中
        //   不应把权威值挤出结果），带 stale_hint
        // - atoms ILIKE 兜底仅在双腿零命中时触发（补漏，不打扰正常排序）
        if want_l1 && query.trim().chars().count() >= 3 {
            let mut kv_hits: Vec<engram_search::SearchHit> = Vec::new();
            if let Ok(kvs) =
                repo::kv_search_literal(&self.pool, query.trim(), max_items.max(1)).await
            {
                for kv in kvs {
                    let kv = kv_stale_hint(kv);
                    kv_hits.push(engram_search::SearchHit {
                        id: kv.id,
                        score: 100.0, // 权威源置顶
                        title: Some(kv.key.clone()),
                        snippet: match &kv.stale_hint {
                            Some(h) => format!("[kv:{}] {}\n⚠ {}", kv.key, kv.value, h),
                            None => format!("[kv:{}] {}", kv.key, kv.value),
                        },
                        kind: Some("kv".into()),
                        needs_review: None,
                    });
                }
            }
            if !kv_hits.is_empty() {
                kv_hits.extend(l1);
                l1 = kv_hits;
            } else if l1.is_empty() {
                for h in repo::atoms_literal_fallback(&self.pool, max_items, query, true).await? {
                    l1.push(engram_search::SearchHit {
                        id: h.id,
                        score: 0.01,
                        title: None,
                        snippet: h.content,
                        kind: Some(h.kind),
                        needs_review: Some(h.needs_review),
                    });
                }
            }
        }
        Ok(l1)
    }

    /// 冷启动上下文包：L1 补充 + 实体透镜，预算裁剪（P015：l2/l3 段随场景层退役）。
    pub async fn context_pack(
        &self,
        query: Option<&str>,
        budget_items: usize,
        budget_chars: usize,
        no_feedback: bool,
        // P015：调用方自带查询向量（None = 内部 embed）
        query_vec: Option<&[f32]>,
    ) -> Result<ContextPack, MemoryError> {
        let mut chars_used = 0usize;
        let mut truncated = false;

        // v2 修复（N7）：按**完整序列化体积**计量（含 evidence_refs/source_refs）——
        // 此前只数正文文本，chars_used 远小于真实注入体积，字符预算形同虚设。

        // 有 query 时预计算 query 向量（L1 用）
        let qv: Option<Vec<f32>> = match query_vec {
            Some(v) => Some(v.to_vec()),
            None => match query {
                Some(q) => self
                    .try_embed_query(&[q.to_string()])
                    .await
                    .and_then(|v| v.first().cloned()),
                None => None,
            },
        };

        // 实体透镜（小预算 ~20%）：AI 冷启动要知道用户世界里都有谁
        let ent_budget = (budget_items / 5).max(2).min(budget_items.max(1));
        let (out_entities, used_e, trunc_e) = self
            .entity_lens(query, ent_budget, budget_chars, chars_used)
            .await?;
        chars_used = used_e;
        truncated |= trunc_e;

        // L1 原子（条数上限 = budget_items；字符预算全局统一裁剪）
        let (out_atoms, used_a, trunc_a) = self
            .pack_atoms(query, qv.as_deref(), budget_items, budget_chars, chars_used)
            .await?;
        chars_used = used_a;
        truncated |= trunc_a;

        let pending_review = self
            .pack_feedback_and_review(&out_atoms, no_feedback)
            .await?;

        Ok(ContextPack {
            atoms: out_atoms,
            entities: out_entities,
            pending_review,
            meta: ContextMeta {
                chars_used,
                truncated,
                query: query.map(String::from),
            },
        })
    }
    /// 分层收集：实体（名字加权）+ L1（atoms，含 KV 权威通道与 ILIKE 补漏）。
    async fn collect_layers(
        &self,
        query: &str,
        qv: Option<&[f32]>,
        max_items: i64,
        from: Option<chrono::DateTime<chrono::Utc>>,
        to: Option<chrono::DateTime<chrono::Utc>>,
        want: [bool; 2],
    ) -> Result<(Vec<SearchHit>, Vec<SearchHit>), MemoryError> {
        let [want_e, want_l1] = want;
        let entities = if want_e {
            self.entity_hits(query, max_items).await?
        } else {
            vec![]
        };
        let mut l1 = if want_l1 {
            search_atoms(
                &self.pool, query, qv, max_items,
                true, // sensitive 口径放开（2026-09-12）——标记保留、不再隐身
                from, to,
            )
            .await
            .map_err(StoreError::from)?
        } else {
            vec![]
        };
        // ILIKE 补漏与 KV 权威通道（见助手：KV 恒合并置顶、atoms 兜底仅零命中时触发）
        l1 = self
            .kv_and_literal_supplement(query, want_l1, max_items, l1)
            .await?;

        Ok((entities, l1))
    }
    /// 命中反馈（B9：读路径也计热度；no_feedback=true 跳过——B6 污染防护）+ 待审代问队列（不计热度）。
    async fn pack_feedback_and_review(
        &self,
        out_atoms: &[AtomDto],
        no_feedback: bool,
    ) -> Result<Vec<AtomDto>, MemoryError> {
        // B9：context_pack 也是使用（AI 冷启动读路径），同样计热度；
        // no_feedback=true 供 harness 注入/测试使用——不刷热度（B6 污染防护）
        if !no_feedback {
            self.fire_hit_feedback(out_atoms.iter().map(|a| a.id).collect());
        }

        // 待审代问（议题三）：队列里的低置信项带给 AI——下次对话顺口确认一句，
        // atom-patch 回写，待审从「翻网页」变「一句话」。不计热度。
        let pending_review = repo::pending_review_atoms(&self.pool).await?;
        Ok(pending_review)
    }
}

/// 序列化体积计量（v2/N7）：按完整 JSON 体积判断是否超字符预算。
pub(super) fn count_json(
    item: &impl serde::Serialize,
    budget_chars: usize,
    used: &mut usize,
    trunc: &mut bool,
) -> bool {
    let full = serde_json::to_string(item).unwrap_or_default();
    if *used + full.len() > budget_chars {
        *trunc = true;
        false
    } else {
        *used += full.len();
        true
    }
}

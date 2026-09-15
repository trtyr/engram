//! 跨域统一检索：一次查询融合 memory + wiki 两域。
//!
//! 两域各自的 score 尺度不同（memory 是 RRF 分数、wiki 是 ts_rank），
//! 直接合并排序会偏向大尺度域。这里统一用「域内 rank 归一化」：每个域（memory 的
//! l1/l2 各自独立、wiki）内按命中顺序赋 RRF 分数 `1/(60 + rank)`，
//! 使跨域分数可比，融合后按分数降序截断。

use engram_llm::ProviderRegistry;
use engram_storage::PgPool;
use uuid::Uuid;

use crate::memory::MemoryService;
use crate::wiki::WikiService;
use crate::wiki_docs::WikiDocumentService;
use engram_llm::types::Purpose;

/// 统一命中（跨域检索的最小公分母）。
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct UnifiedHit {
    /// 域标签：memory | wiki
    pub domain: String,
    pub id: Uuid,
    pub title: Option<String>,
    pub snippet: String,
    /// 域内 rank 归一化的 RRF 分数（跨域可比）
    pub score: f64,
    /// 域特有字段（layer/kind/slug/page_type/document_id/seq 等）
    #[schema(value_type = Object)]
    pub extra: serde_json::Value,
}

#[derive(Debug, thiserror::Error)]
pub enum UnifiedError {
    #[error("{0}")]
    BadRequest(String),
    #[error("存储暂时不可用: {0}")]
    Storage(String),
}

impl From<engram_storage::StoreError> for UnifiedError {
    fn from(e: engram_storage::StoreError) -> Self {
        UnifiedError::Storage(e.to_string())
    }
}

/// 跨域统一检索服务。
#[derive(Clone)]
pub struct UnifiedSearch {
    pool: PgPool,
    registry: ProviderRegistry,
    data_dir: std::path::PathBuf,
    /// R6 rerank 用 LLM 门面（可选精排）
    llm: engram_distill::llm_port::LlmRef,
}

impl UnifiedSearch {
    pub fn new(
        pool: PgPool,
        registry: ProviderRegistry,
        data_dir: impl Into<std::path::PathBuf>,
        llm: engram_distill::llm_port::LlmRef,
    ) -> Self {
        Self {
            pool,
            registry,
            data_dir: data_dir.into(),
            llm,
        }
    }

    /// 一次查询融合三域，返回按 RRF rank 分数降序的统一命中。
    /// `rerank=true`（R6）：RRF 排序后对 top 候选做一次 LLM 精排——LLM 失败/解析败
    /// 降级原序（warn 可见），检索永不因 rerank 失败而失败。默认 false = 零额外 LLM 调用。
    pub async fn search(
        &self,
        query: &str,
        limit: i64,
        rerank: bool,
    ) -> Result<Vec<UnifiedHit>, UnifiedError> {
        if query.trim().is_empty() {
            return Err(UnifiedError::BadRequest("query 不能为空".into()));
        }
        let per_domain = limit.clamp(5, 50);

        let mem = MemoryService::new(self.pool.clone(), self.registry.clone());
        let know = WikiDocumentService::new(
            self.pool.clone(),
            self.registry.clone(),
            self.data_dir.clone(),
        );
        let wiki = WikiService::new(self.pool.clone(), self.registry.clone());

        // 多库（2026-09-08）：文档与 wiki 页按库各查一份再并集（RRF 归一化不变）
        let lib_ids: Vec<Uuid> = crate::wiki::libraries::list(&self.pool)
            .await
            .iter()
            .map(|l| l.id)
            .collect();

        // 三域并行检索 + 实体层（各自降级：无 embedding 时退化为 FTS，不互相阻塞）
        let (mem_res, know_res, wiki_res, ent_res, todo_res) = tokio::join!(
            mem.search(query, &["l1", "l2"], per_domain, true, None, None),
            async {
                let mut out = Vec::new();
                for lib in &lib_ids {
                    if let Ok(mut hits) = know.search(*lib, query, per_domain).await {
                        out.append(&mut hits);
                    }
                }
                Ok::<_, UnifiedError>(out)
            },
            async {
                let mut out = Vec::new();
                for lib in &lib_ids {
                    if let Ok(mut hits) = wiki.search(*lib, query, per_domain).await {
                        out.append(&mut hits);
                    }
                }
                Ok::<_, UnifiedError>(out)
            },
            async {
                engram_search::search_entities(&self.pool, query, per_domain)
                    .await
                    .map_err(engram_storage::StoreError::from)
            },
            async move {
                engram_storage::repo::todos::search_open(&self.pool, query, per_domain)
                    .await
                    .map(|rows| {
                        rows.into_iter()
                            .map(|(id, kind, title, body, priority)| {
                                (id, format!("{kind}:{title}"), body, priority)
                            })
                            .collect::<Vec<_>>()
                    })
            },
        );

        let mut merged: Vec<UnifiedHit> = Vec::new();

        if let Ok(res) = mem_res {
            for h in res.l1 {
                merged.push(UnifiedHit {
                    domain: "memory".into(),
                    id: h.id,
                    title: h.title,
                    snippet: h.snippet,
                    score: 0.0,
                    extra: serde_json::json!({ "layer": "l1", "kind": h.kind }),
                });
            }
            for h in res.l2 {
                merged.push(UnifiedHit {
                    domain: "memory".into(),
                    id: h.id,
                    title: h.title,
                    snippet: h.snippet,
                    score: 0.0,
                    extra: serde_json::json!({ "layer": "l2", "kind": h.kind }),
                });
            }
        } else {
            tracing::warn!("统一检索：memory 域失败，跳过");
        }

        if let Ok(res) = know_res {
            for h in res {
                merged.push(UnifiedHit {
                    domain: "wiki".into(),
                    id: h.chunk_id,
                    title: Some(h.document_title),
                    snippet: h.snippet,
                    score: 0.0,
                    extra: serde_json::json!({ "document_id": h.document_id, "seq": h.seq }),
                });
            }
        } else {
            tracing::warn!("统一检索：wiki 文档域失败，跳过");
        }

        // 实体域：主角先行（palette 命中实体 → 直达星系详情）
        if let Ok(hits) = ent_res {
            for h in hits {
                merged.push(UnifiedHit {
                    domain: "entity".into(),
                    id: h.id,
                    title: h.title,
                    snippet: h.snippet,
                    score: 0.0,
                    extra: serde_json::json!({ "kind": h.kind }),
                });
            }
        } else {
            tracing::warn!("统一检索：entity 域失败，跳过");
        }

        // 待办域：open 待办的标题/正文 ILIKE 匹配
        if let Ok(hits) = todo_res {
            for t in hits {
                merged.push(UnifiedHit {
                    domain: "todo".into(),
                    id: t.0,
                    title: Some(t.1),
                    snippet: t.2.chars().take(200).collect(),
                    score: 0.0,
                    extra: serde_json::json!({ "priority": t.3 }),
                });
            }
        } else {
            tracing::warn!("统一检索：todo 域失败，跳过");
        }

        if let Ok(res) = wiki_res {
            for p in res {
                merged.push(UnifiedHit {
                    domain: "wiki".into(),
                    id: p.id,
                    title: Some(p.title),
                    snippet: p.content.chars().take(200).collect(),
                    score: 0.0,
                    extra: serde_json::json!({ "slug": p.slug, "page_type": p.page_type }),
                });
            }
        } else {
            tracing::warn!("统一检索：wiki 域失败，跳过");
        }

        // 域内 rank 归一化 + 全局排序 + 截断
        assign_rrf_scores(&mut merged);
        merged.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        merged.truncate(limit.max(0) as usize);

        if rerank && merged.len() > 1 {
            let top = merged.len().min(10);
            match rerank_hits(&self.llm, query, &merged[..top]).await {
                Ok(order) if order.len() == top && order.iter().all(|i| *i < top) => {
                    let rest: Vec<UnifiedHit> = merged.split_off(top);
                    let mut top_vec: Vec<Option<UnifiedHit>> = merged.drain(..).map(Some).collect();
                    let mut reordered: Vec<UnifiedHit> = Vec::with_capacity(top);
                    for i in order {
                        if let Some(h) = top_vec[i].take() {
                            reordered.push(h);
                        }
                    }
                    reordered.extend(top_vec.into_iter().flatten());
                    reordered.extend(rest);
                    merged = reordered;
                }
                Ok(order) if order.len() == top => {
                    tracing::warn!("统一检索 rerank：order 含越界索引，降级原序");
                }
                Ok(_) => tracing::warn!("统一检索 rerank：order 长度不符，降级原序"),
                Err(e) => tracing::warn!(error = %e, "统一检索 rerank 失败，降级原序"),
            }
        }

        Ok(merged)
    }
}

/// 域内 rank 归一化：按 `domain:layer` 分组，组内按命中顺序赋 RRF 分数 `1/(60+rank)`。
/// memory 的 l1/l2 各自独立 rank；wiki 各一组。
/// R6：LLM 精排——top 候选（title+snippet）交模型输出目标顺序（原索引数组）。
/// 任何失败返回 Err（调用方降级原序）；只做一次 chat 调用，失败不重试（检索是热路径）。
pub async fn rerank_hits(
    llm: &engram_distill::llm_port::LlmRef,
    query: &str,
    hits: &[UnifiedHit],
) -> Result<Vec<usize>, String> {
    let listing: String = hits
        .iter()
        .enumerate()
        .map(|(i, h)| {
            format!(
                "[{}] {}
{}",
                i,
                h.title.as_deref().unwrap_or("(无标题)"),
                h.snippet.chars().take(120).collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join(
            "
---
",
        );
    let system = "你是检索重排序员。给定查询与候选列表（每项带 [索引]），按与查询的相关性从高到低输出索引。只输出 JSON：{\"order\": [索引数组]}，必须包含全部索引且不重复。";
    let user = format!(
        "查询：{query}

候选：
{listing}"
    );

    // rerank 是热路径旁路——不带 job 上下文（job_id 用 NOW_v7 占位，事件流可按 purpose 过滤）
    let v = llm
        .chat_json(Purpose::SearchRerank, system, user.as_str(), Uuid::now_v7())
        .await
        .map_err(|e| e.to_string())?;
    let order: Vec<usize> = v
        .get("order")
        .and_then(|o| o.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_u64().map(|n| n as usize))
                .collect()
        })
        .ok_or_else(|| "order 字段缺失".to_string())?;
    Ok(order)
}

pub(crate) fn assign_rrf_scores(hits: &mut [UnifiedHit]) {
    use std::collections::HashMap;
    let mut rank: HashMap<String, usize> = HashMap::new();
    for h in hits.iter_mut() {
        let layer = h.extra.get("layer").and_then(|v| v.as_str()).unwrap_or("");
        let key = format!("{}:{}", h.domain, layer);
        let r = rank.entry(key).or_insert(0);
        h.score = 1.0 / (60.0 + *r as f64);
        *r += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(domain: &str, layer: Option<&str>) -> UnifiedHit {
        UnifiedHit {
            domain: domain.into(),
            id: Uuid::now_v7(),
            title: None,
            snippet: "s".into(),
            score: 0.0,
            extra: layer
                .map(|l| serde_json::json!({ "layer": l }))
                .unwrap_or(serde_json::json!({})),
        }
    }

    #[test]
    fn rrf_rank_normalizes_per_domain_layer() {
        // 两域混排：memory l1/l2、wiki（文档+页）
        let mut hits = vec![
            hit("memory", Some("l1")),
            hit("wiki", None),
            hit("wiki", None),
            hit("memory", Some("l2")),
            hit("memory", Some("l1")),
            hit("wiki", None),
        ];
        assign_rrf_scores(&mut hits);

        // 每个 domain:layer 组内：第 0 名 1/60，第 1 名 1/61
        let mem_l1: Vec<f64> = hits
            .iter()
            .filter(|h| h.domain == "memory" && h.extra["layer"] == "l1")
            .map(|h| h.score)
            .collect();
        assert_eq!(mem_l1.len(), 2);
        assert!((mem_l1[0] - 1.0 / 60.0).abs() < 1e-9);
        assert!((mem_l1[1] - 1.0 / 61.0).abs() < 1e-9);

        // 跨域第 0 名分数相等（公平）
        let first_scores: Vec<f64> = ["memory", "wiki"]
            .iter()
            .filter_map(|d| {
                hits.iter()
                    .filter(|h| &h.domain == d)
                    .map(|h| h.score)
                    .max_by(|a, b| a.partial_cmp(b).unwrap())
            })
            .collect();
        assert!(first_scores.iter().all(|s| (*s - 1.0 / 60.0).abs() < 1e-9));
    }
}

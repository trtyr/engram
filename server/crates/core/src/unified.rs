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
}

impl UnifiedSearch {
    pub fn new(
        pool: PgPool,
        registry: ProviderRegistry,
        data_dir: impl Into<std::path::PathBuf>,
    ) -> Self {
        Self {
            pool,
            registry,
            data_dir: data_dir.into(),
        }
    }

    /// 一次查询融合三域，返回按 RRF rank 分数降序的统一命中。
    pub async fn search(&self, query: &str, limit: i64) -> Result<Vec<UnifiedHit>, UnifiedError> {
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

        // 三域并行检索 + 实体层（各自降级：无 embedding 时退化为 FTS，不互相阻塞）
        let (mem_res, know_res, wiki_res, ent_res, todo_res) = tokio::join!(
            mem.search(query, &["l1", "l2"], per_domain, true, false, None, None),
            know.search(query, per_domain),
            wiki.search(query, per_domain),
            async {
                engram_search::search_entities(&self.pool, query, per_domain)
                    .await
                    .map_err(engram_storage::StoreError::from)
            },
            async { engram_storage::repo::todos::search_open(&self.pool, query, per_domain).await },
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
        Ok(merged)
    }
}

/// 域内 rank 归一化：按 `domain:layer` 分组，组内按命中顺序赋 RRF 分数 `1/(60+rank)`。
/// memory 的 l1/l2 各自独立 rank；wiki 各一组。
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

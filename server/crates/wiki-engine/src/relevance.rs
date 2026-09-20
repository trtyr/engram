//! 4 信号相关性模型（llm_wiki 对齐）：
//! 直接链 ×3.0 / 源重叠 ×4.0 / Adamic-Adar ×1.5 / 类型亲和 ×1.0
//! 落 wiki_links 权重（rebuild_links 时全量计算）。

use sqlx::PgPool;

use engram_jobs::types::JobError;

pub const W_DIRECT: f64 = 3.0;
pub const W_SOURCE_OVERLAP: f64 = 4.0;
pub const W_ADAMIC_ADAR: f64 = 1.5;
pub const W_TYPE_AFFINITY: f64 = 1.0;

/// 一对页面的 4 信号得分（纯函数，单测友好）。
pub fn relevance_score(
    direct_linked: bool,
    shared_sources: usize,
    adamic_adar: f64,
    same_type: bool,
) -> f64 {
    let mut s = 0.0;
    if direct_linked {
        s += W_DIRECT;
    }
    if shared_sources > 0 {
        s += W_SOURCE_OVERLAP;
    }
    if adamic_adar > 0.0 {
        s += W_ADAMIC_ADAR * adamic_adar.min(1.0);
    }
    if same_type {
        s += W_TYPE_AFFINITY;
    }
    s
}

/// Adamic-Adar：共同邻居的度倒数之和（归一化前）。
/// neighbors_a/b 为两页面的邻居 slug 集合；degree 查询函数给邻居度。
pub fn adamic_adar<F>(neighbors_a: &[String], neighbors_b: &[String], degree: F) -> f64
where
    F: Fn(&str) -> f64,
{
    use std::collections::HashSet;
    let set_a: HashSet<&String> = neighbors_a.iter().collect();
    let set_b: HashSet<&String> = neighbors_b.iter().collect();
    set_a
        .intersection(&set_b)
        .map(|n| 1.0 / degree(n).max(1.0).ln().max(1.0))
        .sum()
}

/// 全量重算某库的 wiki_links 权重（ingest 后调用；量大时 O(n²) 邻居对，百页级可接受）。
/// 页面集合、现有边、UPDATE/INSERT 全部按 library_id 隔离（多库 0037）。
pub async fn rebuild_weights(pool: &PgPool, lib: uuid::Uuid) -> Result<usize, JobError> {
    // 页面集合：slug + type + sources[]（库内）
    let pages: Vec<(String, String, Vec<String>)> = sqlx::query_as::<
        _,
        (String, String, Vec<String>),
    >(
        "SELECT slug, page_type, ARRAY(SELECT jsonb_array_elements_text(frontmatter->'sources')) \
             FROM wiki_pages WHERE page_type NOT IN ('index','log','overview') AND library_id = $1",
    )
    .bind(lib)
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    if pages.is_empty() {
        return Ok(0);
    }

    let slug_type: std::collections::HashMap<&str, &str> = pages
        .iter()
        .map(|(s, t, _)| (s.as_str(), t.as_str()))
        .collect();
    let slug_sources: std::collections::HashMap<&str, &Vec<String>> =
        pages.iter().map(|(s, _, src)| (s.as_str(), src)).collect();

    // 现有 wikilink 邻接（库内边）
    let links: Vec<(String, String)> =
        sqlx::query_as("SELECT from_slug, to_slug FROM wiki_links WHERE library_id = $1")
            .bind(lib)
            .fetch_all(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;

    let (out_neighbors, in_neighbors) = build_neighbor_maps(&links);
    let mut updated = recompute_direct_weights(
        pool,
        lib,
        &links,
        &slug_type,
        &slug_sources,
        &out_neighbors,
        &in_neighbors,
    )
    .await?;
    updated += add_source_overlap_edges(pool, lib, &pages, &links).await?;

    Ok(updated)
}

/// 邻接表（slug → 邻居 slug 列表）。
type NeighborMap<'a> = std::collections::HashMap<&'a str, Vec<&'a str>>;

/// 邻接表：出/入邻居（保留边表原始顺序，供 AA 与度数使用）。
fn build_neighbor_maps(links: &[(String, String)]) -> (NeighborMap<'_>, NeighborMap<'_>) {
    let mut out_neighbors: std::collections::HashMap<&str, Vec<&str>> =
        std::collections::HashMap::new();
    let mut in_neighbors: std::collections::HashMap<&str, Vec<&str>> =
        std::collections::HashMap::new();
    for (from, to) in links {
        out_neighbors
            .entry(from.as_str())
            .or_default()
            .push(to.as_str());
        in_neighbors
            .entry(to.as_str())
            .or_default()
            .push(from.as_str());
    }
    (out_neighbors, in_neighbors)
}

/// 直接链权重重算（直接链 + 源重叠 + AA + 类型），返回更新的边数。
#[allow(clippy::too_many_arguments)]
async fn recompute_direct_weights(
    pool: &PgPool,
    lib: uuid::Uuid,
    links: &[(String, String)],
    slug_type: &std::collections::HashMap<&str, &str>,
    slug_sources: &std::collections::HashMap<&str, &Vec<String>>,
    out_neighbors: &NeighborMap<'_>,
    in_neighbors: &NeighborMap<'_>,
) -> Result<usize, JobError> {
    let neighbor_list = |slug: &str| -> Vec<String> {
        let mut v: Vec<String> = out_neighbors
            .get(slug)
            .map(|l| l.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default();
        let in_list: Vec<String> = in_neighbors
            .get(slug)
            .map(|l| l.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default();
        v.extend(in_list);
        v
    };
    let degree = |slug: &str| -> f64 {
        (out_neighbors.get(slug).map_or(0, Vec::len) + in_neighbors.get(slug).map_or(0, Vec::len))
            as f64
    };

    let mut updated = 0usize;
    // 直接链：重算权重（直接链 + 源重叠 + AA + 类型）
    for (from, to) in links {
        let (Some(&ta), Some(&sa), Some(&sb)) = (
            slug_type.get(from.as_str()),
            slug_sources.get(from.as_str()),
            slug_sources.get(to.as_str()),
        ) else {
            continue;
        };
        let shared = sa.iter().filter(|x| sb.contains(x)).count();
        let aa = adamic_adar(&neighbor_list(from), &neighbor_list(to), degree);
        let w = relevance_score(
            true,
            shared,
            aa,
            ta == slug_type.get(to.as_str()).copied().unwrap_or(""),
        );
        let w = (w as f32).max(0.1);
        sqlx::query(
            "UPDATE wiki_links SET weight = $3 \
             WHERE from_slug = $1 AND to_slug = $2 AND library_id = $4",
        )
        .bind(from)
        .bind(to)
        .bind(w)
        .bind(lib)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        updated += 1;
    }
    Ok(updated)
}

/// 源重叠但无直接链的页面对：补边（weight=4+亲和，无直接链信号）。
async fn add_source_overlap_edges(
    pool: &PgPool,
    lib: uuid::Uuid,
    pages: &[(String, String, Vec<String>)],
    links: &[(String, String)],
) -> Result<usize, JobError> {
    let mut updated = 0usize;
    // 源重叠但无直接链的页面对：补边（weight=4+亲和，无直接链信号）
    for i in 0..pages.len() {
        for j in (i + 1)..pages.len() {
            let (sa_, ta_) = (pages[i].0.as_str(), pages[i].1.as_str());
            let (sb_, tb_) = (pages[j].0.as_str(), pages[j].1.as_str());
            let shared = pages[i].2.iter().filter(|x| pages[j].2.contains(x)).count();
            if shared == 0 {
                continue;
            }
            let has_link = links
                .iter()
                .any(|(f, t)| (f == sa_ && t == sb_) || (f == sb_ && t == sa_));
            if has_link {
                continue;
            }
            let w = ((W_SOURCE_OVERLAP + if ta_ == tb_ { W_TYPE_AFFINITY } else { 0.0 }) as f32)
                .max(0.1);
            // 无向语义：两条有向边都补（与 wikilink 边形态一致）；边挂库（0037 复合主键）
            for (a, b) in [(sa_, sb_), (sb_, sa_)] {
                sqlx::query(
                    "INSERT INTO wiki_links (library_id, from_slug, to_slug, weight) \
                     VALUES ($4, $1, $2, $3) \
                     ON CONFLICT (library_id, from_slug, to_slug) DO NOTHING",
                )
                .bind(a)
                .bind(b)
                .bind(w)
                .bind(lib)
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
            }
            updated += 1;
        }
    }
    Ok(updated)
}

/// 图扩展检索阶段（批次⑤，wiki 大库化 2026-09-19）：
/// RRF/FTS 初召回后沿双链 2-hop 带衰减传播——捞回语义检索漏掉的强关联页（llm_wiki 先例）。
///
/// - 边权归一：`min(weight/6, 1)`（对齐 WikiGraph 的 weight≥6 视觉饱和点）
/// - 衰减：每跳 ×0.5——1-hop bonus = seed × edge_norm × 0.5，2-hop ×0.25
/// - seed 自身不收扩展分（seed 分就是初召回分）
pub const GRAPH_DECAY: f64 = 0.5;
pub const GRAPH_EDGE_NORM_DENOM: f64 = 6.0;

pub fn graph_expand_scores(
    seeds: &[(String, f64)],
    adjacency: &std::collections::HashMap<String, Vec<(String, f64)>>,
) -> Vec<(String, f64)> {
    let edge_norm = |w: f64| (w / GRAPH_EDGE_NORM_DENOM).min(1.0);
    let mut acc: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    // 1-hop
    let mut hop1: Vec<(String, f64)> = Vec::new();
    for (seed, score) in seeds {
        if let Some(neighs) = adjacency.get(seed.as_str()) {
            for (n, w) in neighs {
                let bonus = score * edge_norm(*w) * GRAPH_DECAY;
                if bonus <= 0.0 {
                    continue;
                }
                hop1.push((n.clone(), bonus));
                *acc.entry(n.clone()).or_default() += bonus;
            }
        }
    }
    // 2-hop（从 1-hop 层继续扩散；可能传回 seed——最后统一移除）
    for (mid, mid_bonus) in &hop1 {
        if let Some(neighs) = adjacency.get(mid.as_str()) {
            for (n, w) in neighs {
                let bonus = mid_bonus * edge_norm(*w) * GRAPH_DECAY;
                if bonus <= 0.0 {
                    continue;
                }
                *acc.entry(n.clone()).or_default() += bonus;
            }
        }
    }
    for (seed, _) in seeds {
        acc.remove(seed);
    }
    let mut out: Vec<(String, f64)> = acc.into_iter().collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_signal_weights() {
        // 全命中：3+4+1.5*1+1 = 9.5
        assert!((relevance_score(true, 1, 1.0, true) - 9.5).abs() < 1e-9);
        // 仅直接链：3
        assert!((relevance_score(true, 0, 0.0, false) - 3.0).abs() < 1e-9);
        // 仅源重叠：4
        assert!((relevance_score(false, 2, 0.0, false) - 4.0).abs() < 1e-9);
        // AA 封顶 1.0：1.5
        assert!((relevance_score(false, 0, 99.0, false) - 1.5).abs() < 1e-9);
        // 类型亲和：1
        assert!((relevance_score(false, 0, 0.0, true) - 1.0).abs() < 1e-9);
        // 无信号：0
        assert!(relevance_score(false, 0, 0.0, false) == 0.0);
    }

    #[test]
    fn adamic_adar_computes() {
        // 共同邻居 c（度 3）：1/ln(3)
        let a = vec!["c".to_string(), "x".to_string()];
        let b = vec!["c".to_string(), "y".to_string()];
        let deg = |s: &str| if s == "c" { 3.0 } else { 1.0 };
        let aa = adamic_adar(&a, &b, deg);
        assert!((aa - 1.0 / 3.0_f64.ln()).abs() < 1e-9);
        // 无共同邻居
        assert!(adamic_adar(&a, &["z".to_string()], deg) == 0.0);
    }

    #[test]
    fn graph_expand_two_hop_decay() {
        // 图：seed s → a (w=6) → b (w=3)；s → c (w=3)
        let mut adj = std::collections::HashMap::new();
        adj.insert(
            "s".to_string(),
            vec![("a".to_string(), 6.0), ("c".to_string(), 3.0)],
        );
        adj.insert(
            "a".to_string(),
            vec![("s".to_string(), 6.0), ("b".to_string(), 3.0)],
        );
        adj.insert("b".to_string(), vec![("a".to_string(), 3.0)]);
        adj.insert("c".to_string(), vec![("s".to_string(), 3.0)]);
        let seeds = vec![("s".to_string(), 0.03)];
        let out = graph_expand_scores(&seeds, &adj);
        let get = |slug: &str| {
            out.iter()
                .find(|(s, _)| s == slug)
                .map(|(_, v)| *v)
                .unwrap_or(0.0)
        };
        // a：1-hop，edge_norm(6)=1 → 0.03×0.5 = 0.015
        assert!((get("a") - 0.03 * 0.5).abs() < 1e-9);
        // c：1-hop，edge_norm(3)=0.5 → 0.03×0.5×0.5 = 0.0075
        assert!((get("c") - 0.03 * 0.5 * 0.5).abs() < 1e-9);
        // b：2-hop 经 a：0.015×edge_norm(3)=0.5×0.5 = 0.00375
        assert!((get("b") - 0.015 * 0.5 * 0.5).abs() < 1e-9);
        // seed 自身不收扩展分
        assert!(get("s") == 0.0);
        // 强边优先排序：a > c > b
        assert!(get("a") > get("c") && get("c") > get("b"));
    }
}

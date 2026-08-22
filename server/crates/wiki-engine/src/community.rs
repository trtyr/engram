//! Louvain 社区发现（Rust 实现单层贪心 + 多轮迭代近似）+ 凝聚度评分。
//! 图规模（百页级）用简化 Louvain：模块度贪心节点迁移，收敛即社区。

use std::collections::HashMap;

/// 无向加权图上的 Louvain 一层（节点迁移直到模块度无提升）。
/// 返回：slug → community id（0..k）。
pub fn louvain_communities<'a>(
    nodes: &'a [String],
    edges: &[(String, String, f64)],
) -> HashMap<&'a str, usize> {
    let idx: HashMap<&str, usize> = nodes
        .iter()
        .map(|n| n.as_str())
        .enumerate()
        .map(|(i, n)| (n, i))
        .collect();
    let n = nodes.len();
    if n == 0 {
        return HashMap::new();
    }

    // 邻接（无向累计）
    let mut adj: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    let mut m2: f64 = 0.0; // 总权重×2
    for (from, to, w) in edges {
        if let (Some(&i), Some(&j)) = (idx.get(from.as_str()), idx.get(to.as_str())) {
            adj[i].push((j, *w));
            adj[j].push((i, *w));
            m2 += w;
        }
    }
    if m2 <= 0.0 {
        // 无边：每节点独立社区
        return nodes.iter().map(|s| (s.as_str(), 0)).collect();
    }

    // 初始：每节点自己的社区
    let mut comm: Vec<usize> = (0..n).collect();
    let k: Vec<f64> = (0..n)
        .map(|i| adj[i].iter().map(|(_, w)| w).sum())
        .collect();

    // 贪心节点迁移：最多 O(n) 轮（实际 2-4 轮收敛），EPS 防震荡死循环
    const EPS: f64 = 1e-12;
    let max_rounds = n.max(1);
    let mut improved = true;
    let mut round = 0usize;
    while improved && round < max_rounds {
        improved = false;
        round += 1;
        for i in 0..n {
            let mut comm_w: HashMap<usize, f64> = HashMap::new();
            for &(j, w) in &adj[i] {
                *comm_w.entry(comm[j]).or_default() += w;
            }
            let ci = comm[i];
            let ki = k[i];
            let mut best_gain = EPS;
            let mut best_comm = ci;
            // Σ_tot per community（不含 i 自身——标准 Louvain 移除语义）
            let mut sum_k: HashMap<usize, f64> = HashMap::new();
            for j in 0..n {
                if j == i {
                    continue;
                }
                *sum_k.entry(comm[j]).or_default() += k[j];
            }
            let w_ici = comm_w.get(&ci).copied().unwrap_or(0.0);
            for (&c, &w) in &comm_w {
                if c == ci {
                    continue;
                }
                // 完整 ΔQ = 迁入增益 - 留守增益（两者都不含 i 的度）
                let gain_in =
                    w / m2 - (ki * sum_k.get(&c).copied().unwrap_or(0.0)) / (2.0 * m2 * m2);
                let gain_stay =
                    w_ici / m2 - (ki * sum_k.get(&ci).copied().unwrap_or(0.0)) / (2.0 * m2 * m2);
                let dq = gain_in - gain_stay;
                if dq > best_gain {
                    best_gain = dq;
                    best_comm = c;
                }
            }
            if best_comm != ci {
                comm[i] = best_comm;
                improved = true;
            }
        }
    }
    // 重编号
    let mut remap: HashMap<usize, usize> = HashMap::new();
    for c in &comm {
        let next = remap.len();
        remap.entry(*c).or_insert(next);
    }
    for c in comm.iter_mut() {
        *c = remap[c];
    }

    nodes
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), comm[i]))
        .collect()
}

/// 社区凝聚度：社区内实际边权重 / 可能边数（无向对）。
/// 低凝聚（<0.15）在 insights 中标记为稀疏社区。
pub fn community_cohesion(
    nodes: &[String],
    edges: &[(String, String, f64)],
    communities: &HashMap<&str, usize>,
) -> HashMap<usize, f64> {
    let mut intra: HashMap<usize, f64> = HashMap::new();
    let mut sizes: HashMap<usize, usize> = HashMap::new();
    for n in nodes {
        let c = communities.get(n.as_str()).copied().unwrap_or(0);
        *sizes.entry(c).or_default() += 1;
    }
    for (from, to, w) in edges {
        if let (Some(&ca), Some(&cb)) =
            (communities.get(from.as_str()), communities.get(to.as_str()))
            && ca == cb
        {
            *intra.entry(ca).or_default() += w;
        }
    }
    let mut out = HashMap::new();
    for (&c, &sz) in &sizes {
        let possible = (sz as f64) * (sz as f64 - 1.0) / 2.0;
        let cohesion = if possible > 0.0 {
            intra.get(&c).copied().unwrap_or(0.0) / possible
        } else {
            0.0
        };
        out.insert(c, (cohesion * 1000.0).round() / 1000.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn two_clusters_discovered() {
        // 两个三角簇 + 一条弱跨簇边
        let nodes = s(&["a", "b", "c", "d", "e", "f"]);
        let edges: Vec<(String, String, f64)> = [
            ("a", "b", 3.0),
            ("b", "c", 3.0),
            ("a", "c", 3.0),
            ("d", "e", 3.0),
            ("e", "f", 3.0),
            ("d", "f", 3.0),
            ("c", "d", 0.5),
        ]
        .iter()
        .map(|(a, b, w)| (a.to_string(), b.to_string(), *w))
        .collect();
        let comms = louvain_communities(&nodes, &edges);
        assert_eq!(comms.len(), 6);
        // a/b/c 同社区；d/e/f 同社区；两组不同
        assert_eq!(comms["a"], comms["b"]);
        assert_eq!(comms["b"], comms["c"]);
        assert_eq!(comms["d"], comms["e"]);
        assert_eq!(comms["e"], comms["f"]);
        assert_ne!(comms["a"], comms["d"]);
    }

    #[test]
    fn cohesion_scores() {
        let nodes = s(&["a", "b", "c"]);
        let edges = vec![("a".into(), "b".into(), 3.0), ("b".into(), "c".into(), 3.0)];
        let comms: HashMap<&str, usize> = nodes.iter().map(|n| (n.as_str(), 0)).collect();
        let coh = community_cohesion(&nodes, &edges, &comms);
        // 3 节点 3 对，2 条边 ×3.0 权重 → 6/3=2.0
        assert!((coh[&0] - 2.0).abs() < 1e-9);
    }

    #[test]
    fn empty_graph() {
        let comms = louvain_communities(&[], &[]);
        assert!(comms.is_empty());
    }
}

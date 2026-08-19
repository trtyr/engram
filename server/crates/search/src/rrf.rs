//! RRF（Reciprocal Rank Fusion）合并。

/// 融合多个有序 id 列表。score = Σ 1/(k + rank)，rank 从 1 起。
pub fn rrf_merge(lists: &[Vec<uuid::Uuid>], k: u32) -> Vec<(uuid::Uuid, f64)> {
    let mut scores: std::collections::HashMap<uuid::Uuid, f64> = std::collections::HashMap::new();
    for list in lists {
        for (i, id) in list.iter().enumerate() {
            *scores.entry(*id).or_default() += 1.0 / (k as f64 + (i + 1) as f64);
        }
    }
    let mut ranked: Vec<_> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn both_channels_win() {
        let a = vec![Uuid::new_v4(), Uuid::new_v4()];
        let b = vec![a[1], Uuid::new_v4()];
        let merged = rrf_merge(&[a.clone(), b.clone()], 60);
        assert_eq!(merged[0].0, a[1], "双通道命中的应排第一");
        assert!(merged.iter().any(|(id, _)| *id == a[0]));
    }

    #[test]
    fn single_list_preserved() {
        let a = vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let merged = rrf_merge(std::slice::from_ref(&a), 60);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].0, a[0], "单列表保序");
    }
}

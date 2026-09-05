//! 图洞察（llm_wiki 对齐）：意外连接 / 知识缺口（孤立页/稀疏社区/桥节点）
//! + dismiss 持久化（wiki_insight_dismissals）。

use chrono::{DateTime, Utc};
use engram_jobs::types::JobError;
use sqlx::PgPool;
use uuid::Uuid;

use crate::community::{community_cohesion, louvain_communities};
use crate::service::CommunityInfo;

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct Insight {
    /// 稳定键（dismiss 用）
    pub key: String,
    pub kind: String, // surprising_connection | isolated_page | sparse_community | bridge_node
    pub title: String,
    pub detail: String,
    /// 涉及的页面 slug（前端点击高亮）
    pub slugs: Vec<String>,
    /// 预生成检索词（deep research 用）
    pub search_queries: Vec<String>,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct InsightsReport {
    pub insights: Vec<Insight>,
    pub communities: Vec<crate::service::CommunityInfo>,
    pub total_pages: usize,
}
// CommunityInfo 统一由 service.rs 定义（含 sparse 旗标）——
// 曾经两处同名结构体在 utoipa 撞名，schema 与实现互相漂移（top_slug/size 违约的根因）

pub async fn compute_insights(pool: &PgPool) -> Result<InsightsReport, JobError> {
    let pages: Vec<(String, String)> = sqlx::query_as(
        "SELECT slug, page_type FROM wiki_pages WHERE page_type NOT IN ('index','log','overview')",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    let edges: Vec<(String, String, f64)> =
        sqlx::query_as("SELECT from_slug, to_slug, weight::float8 FROM wiki_links")
            .fetch_all(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
    let dismissed: Vec<String> =
        sqlx::query_scalar("SELECT insight_key FROM wiki_insight_dismissals")
            .fetch_all(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;

    let nodes: Vec<String> = pages.iter().map(|(s, _)| s.clone()).collect();
    let type_of: std::collections::HashMap<&str, &str> = pages
        .iter()
        .map(|(s, t)| (s.as_str(), t.as_str()))
        .collect();

    // 度数
    use std::collections::HashMap;
    let mut degree: HashMap<&str, usize> = HashMap::new();
    for (f, t, _) in &edges {
        *degree.entry(f.as_str()).or_default() += 1;
        *degree.entry(t.as_str()).or_default() += 1;
    }

    // 社区
    let comms = louvain_communities(&nodes, &edges);
    let cohesion = community_cohesion(&nodes, &edges, &comms);
    let mut community_info: Vec<CommunityInfo> = {
        let mut sizes: HashMap<usize, Vec<&str>> = HashMap::new();
        for n in &nodes {
            sizes
                .entry(comms.get(n.as_str()).copied().unwrap_or(0))
                .or_default()
                .push(n);
        }
        sizes
            .into_iter()
            .map(|(id, members)| {
                let size = members.len();
                let cohesion = cohesion.get(&id).copied().unwrap_or(0.0);
                crate::service::CommunityInfo {
                    id,
                    top_slug: members.first().copied().unwrap_or_default().to_string(),
                    size,
                    cohesion,
                    sparse: size >= crate::community::SPARSE_MIN_SIZE
                        && cohesion < crate::community::SPARSE_COHESION,
                }
            })
            .collect()
    };
    community_info.sort_by_key(|c| std::cmp::Reverse(c.size));

    let mut insights: Vec<Insight> = Vec::new();

    // 1. 孤立页（degree ≤ 1）
    for (slug, _) in &pages {
        if degree.get(slug.as_str()).copied().unwrap_or(0) <= 1 {
            let key = format!("isolated_page:{slug}");
            if dismissed.iter().any(|d| d == &key) {
                continue;
            }
            insights.push(Insight {
                key,
                kind: "isolated_page".into(),
                title: format!("孤立页面：{slug}"),
                detail: "与其他页面几乎没有连接——考虑补充互链或合并到相关主题页".into(),
                slugs: vec![slug.clone()],
                search_queries: vec![slug.clone()],
            });
        }
    }

    // 2. 稀疏社区（cohesion < SPARSE_COHESION 且 ≥SPARSE_MIN_SIZE 页）
    for c in &community_info {
        if c.size >= crate::community::SPARSE_MIN_SIZE
            && c.cohesion < crate::community::SPARSE_COHESION
        {
            let key = format!("sparse_community:{}", c.id);
            if dismissed.iter().any(|d| d == &key) {
                continue;
            }
            let members: Vec<String> = nodes
                .iter()
                .filter(|n| comms.get(n.as_str()) == Some(&{ c.id }))
                .cloned()
                .collect();
            insights.push(Insight {
                key,
                kind: "sparse_community".into(),
                title: format!(
                    "稀疏知识区 #{}（{} 页，凝聚度 {:.2}）",
                    c.id, c.size, c.cohesion
                ),
                detail: "该主题群内部互链很弱——值得为这组页面补充综述或对比页把它们连起来".into(),
                slugs: members.clone(),
                search_queries: members.iter().take(2).cloned().collect(),
            });
        }
    }

    // 3. 桥节点（连接 3+ 社区）
    let mut node_communities: HashMap<&str, std::collections::HashSet<usize>> = HashMap::new();
    for (f, t, _) in &edges {
        if let (Some(&cf), Some(&ct)) = (comms.get(f.as_str()), comms.get(t.as_str()))
            && cf != ct
        {
            node_communities.entry(f.as_str()).or_default().insert(cf);
            node_communities.entry(f.as_str()).or_default().insert(ct);
            node_communities.entry(t.as_str()).or_default().insert(cf);
            node_communities.entry(t.as_str()).or_default().insert(ct);
        }
    }
    for (slug, cs) in node_communities {
        if cs.len() >= 3 {
            let key = format!("bridge_node:{slug}");
            if dismissed.iter().any(|d| d == &key) {
                continue;
            }
            insights.push(Insight {
                key,
                kind: "bridge_node".into(),
                title: format!("桥节点：{slug}（跨 {} 个知识区）", cs.len()),
                detail: "该页面是多个知识区的交汇点——更新它时影响面最大，值得保持精炼准确".into(),
                slugs: vec![slug.to_string()],
                search_queries: vec![slug.to_string()],
            });
        }
    }

    // 4. 意外连接（跨社区 + 跨类型的强边）
    let max_w = edges.iter().map(|(_, _, w)| *w).fold(0.0_f64, f64::max);
    for (f, t, w) in &edges {
        if let (Some(&cf), Some(&ct)) = (comms.get(f.as_str()), comms.get(t.as_str())) {
            let cross = cf != ct;
            let cross_type = type_of.get(f.as_str()) != type_of.get(t.as_str());
            let strong = max_w > 0.0 && w / max_w > 0.5;
            if cross && cross_type && strong {
                let key = format!("surprising_connection:{f}->{t}");
                if dismissed.iter().any(|d| d == &key) {
                    continue;
                }
                insights.push(Insight {
                    key,
                    kind: "surprising_connection".into(),
                    title: format!("意外连接：{f} ↔ {t}"),
                    detail: format!(
                        "跨社区且跨类型的强连接（权重 {w:.1}）——这两个主题的关联可能是新的洞察，也可能需要修正"
                    ),
                    slugs: vec![f.clone(), t.clone()],
                    search_queries: vec![f.clone(), t.clone()],
                });
            }
        }
    }

    Ok(InsightsReport {
        insights,
        communities: community_info,
        total_pages: pages.len(),
    })
}

pub async fn dismiss(pool: &PgPool, key: &str) -> Result<(), JobError> {
    sqlx::query(
        "INSERT INTO wiki_insight_dismissals (insight_key) VALUES ($1) ON CONFLICT DO NOTHING",
    )
    .bind(key)
    .execute(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(())
}

pub async fn reset_dismissals(pool: &PgPool) -> Result<(), JobError> {
    sqlx::query("DELETE FROM wiki_insight_dismissals")
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(())
}

#[allow(dead_code)]
fn ts_now() -> DateTime<Utc> {
    Utc::now()
}

#[allow(dead_code)]
fn uuid7() -> Uuid {
    Uuid::now_v7()
}

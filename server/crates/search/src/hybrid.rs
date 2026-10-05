//! 纯向量检索（P015）：FTS/RRF 已退役——单条 SQL 内只走向量近邻。
//! 噪声防线保留：绝对天花板（挡全库无相关）+ 相对间隔（挡逐措辞距离漂移）。

use std::sync::LazyLock;

use pgvector::Vector;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// 统一命中形态。
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct SearchHit {
    pub id: Uuid,
    /// 相关性分（1 - 余弦距离，越近越高）
    pub score: f64,
    pub title: Option<String>,
    pub snippet: String,
    /// 额外定位信息（如 atom kind / scenario topic）
    pub kind: Option<String>,
    /// O2：待人审标记——l1 命中携带（AI 引用前该向用户确认；其余层 None）
    pub needs_review: Option<bool>,
}

/// 向量近邻的距离天花板（余弦距离上限）。
/// 超过它的一律不相关——否则不相关查询会返回按名次排序的
/// 全库噪声页（v2 测试报告 H-B2：空查询/乱码/不相关词一律 20 条）。
/// 依模型几何而异（实测短中文句不相关对 ~0.5-0.67、相关对 <0.5），可用
/// `AGENT_MEMORY_VEC_FALLBACK_MAX_DISTANCE` 按部署的 embedding 模型调整。
static VEC_MAX_DISTANCE: LazyLock<f32> = LazyLock::new(|| {
    std::env::var("AGENT_MEMORY_VEC_FALLBACK_MAX_DISTANCE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.45)
});

/// 相对间隔：只保留与最近邻距离差 ≤ 此值的项——
/// 绝对天花板挡「全库无相关」，相对间隔挡「逐措辞的距离漂移」。
static VEC_RELATIVE_MARGIN: LazyLock<f32> = LazyLock::new(|| {
    std::env::var("AGENT_MEMORY_VEC_FALLBACK_RELATIVE_MARGIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.15)
});

/// score 展示舍入（3 位小数）。
fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// atoms 纯向量检索（仅 active）。`query_vec` 为 None 时空结果——
/// 向量是唯一检索通道（P015：FTS 退役），无查询向量即无从检索。
pub async fn search_atoms(
    pool: &PgPool,
    _query: &str,
    query_vec: Option<&[f32]>,
    limit: i64,
    include_sensitive: bool,
    from: Option<chrono::DateTime<chrono::Utc>>,
    to: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<Vec<SearchHit>, sqlx::Error> {
    let Some(qv) = query_vec else {
        return Ok(vec![]);
    };
    // 敏感口径（决策 001 / 2026-09-12）：主检索全暴露（include_sensitive 恒 true）；
    // include_sensitive 机制仅导出侧显式开关还在用。布尔编译为 SQL 常量——非用户输入，无注入面。
    let sens_filter = if include_sensitive {
        "true"
    } else {
        "NOT sensitive"
    };
    let qvec = Vector::from(qv.to_vec());
    let sql = "SELECT id, kind, content, needs_review, embedding <=> $1 AS dist \
         FROM atoms \
         WHERE status = 'active' AND embedding IS NOT NULL AND ("
        .to_string()
        + sens_filter
        + ") AND embedding <=> $1 <= (SELECT min(embedding <=> $1) FROM atoms \
               WHERE status = 'active' AND embedding IS NOT NULL AND ("
        + sens_filter
        + ")) + $2 \
         AND embedding <=> $1 <= $3 \
         AND ($4::timestamptz IS NULL OR COALESCE(occurred_at, created_at) >= $4) \
         AND ($5::timestamptz IS NULL OR COALESCE(occurred_at, created_at) <= $5) \
         ORDER BY dist LIMIT $6";
    let rows = sqlx::query(sql.as_str())
        .bind(&qvec)
        .bind(*VEC_RELATIVE_MARGIN)
        .bind(*VEC_MAX_DISTANCE)
        .bind(from)
        .bind(to)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let dist: f64 = r.get("dist");
            let content: String = r.get("content");
            // atoms 无 title 列——内容 ≤40 字时 title 与 snippet 重复 → None 去冗余
            let prefix: String = content.chars().take(40).collect();
            let title = if prefix == content {
                None
            } else {
                Some(prefix)
            };
            SearchHit {
                id: r.get("id"),
                score: round3((1.0f64 - dist).max(0.0)),
                title,
                snippet: content,
                kind: r.get("kind"),
                needs_review: r.get("needs_review"),
            }
        })
        .collect())
}

/// 实体检索（token 命中，名字加权）。实体无嵌入/FTS 索引且体量小——名字与摘要的
/// jieba token 直接匹配，名字命中权重 1.0、摘要命中 0.3：搜「张三」应先给实体本人。
pub async fn search_entities(
    pool: &PgPool,
    query: &str,
    limit: i64,
) -> Result<Vec<SearchHit>, sqlx::Error> {
    let tokens = crate::tokenize::tokenize(query);
    if tokens.is_empty() {
        return Ok(vec![]);
    }
    let qset: std::collections::HashSet<String> = tokens.into_iter().collect();
    let rows: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "SELECT id, name, kind, summary FROM entities WHERE merged_into IS NULL AND archived_at IS NULL LIMIT 500",
    )
    .fetch_all(pool)
    .await?;
    let mut hits: Vec<SearchHit> = rows
        .into_iter()
        .filter_map(|(id, name, kind, summary)| {
            let name_tokens: std::collections::HashSet<String> =
                crate::tokenize::tokenize(&name).into_iter().collect();
            let sum_tokens: std::collections::HashSet<String> =
                crate::tokenize::tokenize(&summary).into_iter().collect();
            let name_hits = name_tokens.intersection(&qset).count();
            let sum_hits = sum_tokens.intersection(&qset).count();
            let score = name_hits as f64 + sum_hits as f64 * 0.3;
            if score <= 0.0 {
                return None;
            }
            Some(SearchHit {
                id,
                score: round3(score),
                title: Some(name),
                snippet: summary,
                kind: Some(kind),
                needs_review: None,
            })
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(limit.max(0) as usize);
    Ok(hits)
}

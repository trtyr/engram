//! 混合检索：单条 SQL 内做 FTS + ANN + RRF 融合（D0009/D0010）。

use std::sync::LazyLock;

use pgvector::Vector;
use sqlx::{PgPool, QueryBuilder, Row};
use uuid::Uuid;

use crate::tokenize::{has_query_tokens, tsv_query_smart};

/// 统一命中形态。
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct SearchHit {
    pub id: Uuid,
    /// RRF 融合分
    pub score: f64,
    pub title: Option<String>,
    pub snippet: String,
    /// 额外定位信息（如 atom kind / scenario topic）
    pub kind: Option<String>,
    /// O2：待人审标记——l1 命中携带（AI 引用前该向用户确认；其余层 None）
    pub needs_review: Option<bool>,
}

const RRF_K: i32 = 60;

/// score 展示舍入（3 位小数）：RRF 融合分 16 位小数对排序判断毫无增益，纯耗 token。
fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// FTS 零命中时的向量腿兜底阈值（余弦距离上限）。
/// 词法证据缺席时，只保留语义强相关项——否则不相关查询会返回按名次排序的
/// 全库噪声页（v2 测试报告 H-B2：空查询/乱码/不相关词一律 20 条）。
/// 依模型几何而异（实测短中文句不相关对 ~0.5-0.67、相关对 <0.5），可用
/// `AGENT_MEMORY_VEC_FALLBACK_MAX_DISTANCE` 按部署的 embedding 模型调整。
static VEC_FALLBACK_MAX_DISTANCE: LazyLock<f32> = LazyLock::new(|| {
    std::env::var("AGENT_MEMORY_VEC_FALLBACK_MAX_DISTANCE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.45)
});

/// 兜底相对间隔：兜底候选中只保留与最近邻距离差 ≤ 此值的项——
/// 绝对天花板挡「全库无相关」，相对间隔挡「逐措辞的距离漂移」。
static VEC_FALLBACK_RELATIVE_MARGIN: LazyLock<f32> = LazyLock::new(|| {
    std::env::var("AGENT_MEMORY_VEC_FALLBACK_RELATIVE_MARGIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.15)
});

/// FTS 词法命中预检（同表同口径过滤 + tsv @@ q）。`filter` 由调用方按表结构给出。
async fn fts_has_match(
    pool: &PgPool,
    table: &str,
    query: &str,
    filter: &str,
) -> Result<bool, sqlx::Error> {
    let sql = format!(
        "SELECT EXISTS(SELECT 1 FROM {table}, to_tsquery('simple', $1) q \
         WHERE ({filter}) AND tsv @@ q)"
    );
    sqlx::query_scalar(&sql)
        .bind(tsv_query_smart(query, 3))
        .fetch_one(pool)
        .await
}

/// atoms 混合检索（仅 active）。`query_vec` None 时退化为纯 FTS。
pub async fn search_atoms(
    pool: &PgPool,
    query: &str,
    query_vec: Option<&[f32]>,
    limit: i64,
    include_sensitive: bool,
    from: Option<chrono::DateTime<chrono::Utc>>,
    to: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<Vec<SearchHit>, sqlx::Error> {
    // K7：无 token 且无查询向量 → 短路空结果（单字/纯标点不再空跑 to_tsquery）
    if query_vec.is_none() && !has_query_tokens(query) {
        return Ok(vec![]);
    }
    // P3：sensitive 原子默认排除（医疗/感情/财务），reveal 才进结果。
    // 布尔编译为 SQL 常量——非用户输入，无注入面。
    let sens_filter = if include_sensitive {
        "true"
    } else {
        "NOT sensitive"
    };
    // v2 修复（H-B2）：FTS 零命中 → 向量腿收紧阈值，宁缺毋滥
    let fts_matched = fts_has_match(
        pool,
        "atoms",
        query,
        &format!("status = 'active' AND ({sens_filter})"),
    )
    .await?;
    let has_vec = query_vec.is_some();
    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
        "WITH fts AS (SELECT id, ROW_NUMBER() OVER (ORDER BY ts_rank(tsv, q) DESC) AS rank \
         FROM atoms, to_tsquery('simple', ",
    );
    qb.push_bind(tsv_query_smart(query, 3));
    qb.push(") q WHERE status = 'active' AND (");
    qb.push(sens_filter);
    qb.push(") AND tsv @@ q LIMIT 100) ");

    if has_vec {
        if fts_matched {
            qb.push(", vec AS (SELECT id, ROW_NUMBER() OVER (ORDER BY embedding <=> ");
            qb.push_bind(Vector::from(query_vec.unwrap().to_vec()));
            qb.push(") AS rank FROM atoms WHERE status = 'active' AND embedding IS NOT NULL LIMIT 100) ");
        } else {
            // v2 遗留修复：兜底双条件——绝对天花板（挡全库无相关的查询）+ 相对间隔
            // （只留与最近邻一档距离内的项，容忍逐措辞的绝对距离漂移：
            //   「宠物」查询下橘猫即使绝对距离偏大也保留；完全无关查询全体超天花板 → 空）
            qb.push(", vec_raw AS (SELECT id, embedding <=> ");
            qb.push_bind(Vector::from(query_vec.unwrap().to_vec()));
            qb.push(
                " AS dist FROM atoms WHERE status = 'active' AND embedding IS NOT NULL \
                     AND embedding <=> ",
            );
            qb.push_bind(Vector::from(query_vec.unwrap().to_vec()));
            qb.push(format!(" <= {})", *VEC_FALLBACK_MAX_DISTANCE));
            qb.push(
                ", vec AS (SELECT id, ROW_NUMBER() OVER (ORDER BY dist) AS rank \
                     FROM vec_raw WHERE dist <= (SELECT min(dist) FROM vec_raw) + ",
            );
            qb.push_bind(*VEC_FALLBACK_RELATIVE_MARGIN);
            qb.push(") ");
        }
    }

    qb.push("SELECT a.id, a.kind, a.content, a.needs_review, (COALESCE(1.0/(");
    qb.push_bind(RRF_K);
    qb.push(" + fts.rank), 0)");
    if has_vec {
        qb.push(" + COALESCE(1.0/(");
        qb.push_bind(RRF_K);
        qb.push(" + vec.rank), 0)");
    }
    // 过期降权（phase-2）：valid_until 已过的原子分数减半排后——不消失，历史价值还在
    qb.push(") * CASE WHEN a.valid_until IS NOT NULL AND a.valid_until < now() THEN 0.5::float8 ELSE 1.0::float8 END AS score FROM atoms a LEFT JOIN fts ON fts.id = a.id ");
    if has_vec {
        qb.push("LEFT JOIN vec ON vec.id = a.id ");
    }
    qb.push("WHERE a.status = 'active' AND (");
    qb.push(sens_filter);
    qb.push(") AND (fts.id IS NOT NULL");
    if has_vec {
        qb.push(" OR vec.id IS NOT NULL");
    }
    qb.push(")");
    // 时间范围过滤（phase-2）：occurred_at 优先 NULL fallback created_at
    if let Some(f) = from {
        qb.push(" AND COALESCE(a.occurred_at, a.created_at) >= ");
        qb.push_bind(f);
    }
    if let Some(t) = to {
        qb.push(" AND COALESCE(a.occurred_at, a.created_at) <= ");
        qb.push_bind(t);
    }
    qb.push(" ORDER BY score DESC LIMIT ");
    qb.push_bind(limit);

    let rows = qb.build().fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            // v2 修复（N1）：atoms 无 title 列——取内容前缀做标题；内容 ≤40 字时
            // 前缀 == 全文，title 与 snippet 完全重复（R 报告 P1-4）→ 置 None 去冗余
            let content: String = r.get("content");
            let prefix: String = content.chars().take(40).collect();
            let title = if prefix == content {
                None
            } else {
                Some(prefix)
            };
            SearchHit {
                id: r.get("id"),
                score: round3(r.get::<f64, _>("score")),
                title,
                snippet: content,
                kind: r.get("kind"),
                needs_review: r.get("needs_review"),
            }
        })
        .collect())
}

/// scenarios 混合检索。
pub async fn search_scenarios(
    pool: &PgPool,
    query: &str,
    query_vec: Option<&[f32]>,
    limit: i64,
) -> Result<Vec<SearchHit>, sqlx::Error> {
    // K7：无 token 且无查询向量 → 短路空结果
    if query_vec.is_none() && !has_query_tokens(query) {
        return Ok(vec![]);
    }
    // v2 修复（H-B2）：FTS 零命中 → 向量腿收紧阈值（与 atoms 同策略）
    let fts_matched = fts_has_match(pool, "scenarios", query, "true").await?;
    let has_vec = query_vec.is_some();
    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
        "WITH fts AS (SELECT id, ROW_NUMBER() OVER (ORDER BY ts_rank(tsv, q) DESC) AS rank \
         FROM scenarios, to_tsquery('simple', ",
    );
    qb.push_bind(tsv_query_smart(query, 3));
    qb.push(") q WHERE tsv @@ q LIMIT 100) ");

    if has_vec {
        if fts_matched {
            qb.push(", vec AS (SELECT id, ROW_NUMBER() OVER (ORDER BY embedding <=> ");
            qb.push_bind(Vector::from(query_vec.unwrap().to_vec()));
            qb.push(") AS rank FROM scenarios WHERE embedding IS NOT NULL LIMIT 100) ");
        } else {
            // v2 遗留修复：与 atoms 同款兜底双条件（绝对天花板 + 相对间隔）
            qb.push(", vec_raw AS (SELECT id, embedding <=> ");
            qb.push_bind(Vector::from(query_vec.unwrap().to_vec()));
            qb.push(" AS dist FROM scenarios WHERE embedding IS NOT NULL AND embedding <=> ");
            qb.push_bind(Vector::from(query_vec.unwrap().to_vec()));
            qb.push(format!(" <= {})", *VEC_FALLBACK_MAX_DISTANCE));
            qb.push(
                ", vec AS (SELECT id, ROW_NUMBER() OVER (ORDER BY dist) AS rank \
                     FROM vec_raw WHERE dist <= (SELECT min(dist) FROM vec_raw) + ",
            );
            qb.push_bind(*VEC_FALLBACK_RELATIVE_MARGIN);
            qb.push(") ");
        }
    }

    qb.push("SELECT s.id, s.topic, s.summary, COALESCE(1.0/(");
    qb.push_bind(RRF_K);
    qb.push(" + fts.rank), 0)");
    if has_vec {
        qb.push(" + COALESCE(1.0/(");
        qb.push_bind(RRF_K);
        qb.push(" + vec.rank), 0)");
    }
    qb.push("::float8 AS score FROM scenarios s LEFT JOIN fts ON fts.id = s.id ");
    if has_vec {
        qb.push("LEFT JOIN vec ON vec.id = s.id ");
    }
    qb.push("WHERE fts.id IS NOT NULL");
    if has_vec {
        qb.push(" OR vec.id IS NOT NULL");
    }
    qb.push(" ORDER BY score DESC LIMIT ");
    qb.push_bind(limit);

    let rows = qb.build().fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|r| SearchHit {
            id: r.get("id"),
            score: round3(r.get::<f64, _>("score")),
            title: r.get::<Option<String>, _>("topic"),
            snippet: r.get("summary"),
            kind: None,
            needs_review: None,
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

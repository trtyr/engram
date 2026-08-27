//! 混合检索：单条 SQL 内做 FTS + ANN + RRF 融合（D0009/D0010）。

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
}

const RRF_K: i32 = 60;

/// atoms 混合检索（仅 active）。`query_vec` None 时退化为纯 FTS。
pub async fn search_atoms(
    pool: &PgPool,
    query: &str,
    query_vec: Option<&[f32]>,
    limit: i64,
) -> Result<Vec<SearchHit>, sqlx::Error> {
    // K7：无 token 且无查询向量 → 短路空结果（单字/纯标点不再空跑 to_tsquery）
    if query_vec.is_none() && !has_query_tokens(query) {
        return Ok(vec![]);
    }
    let has_vec = query_vec.is_some();
    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
        "WITH fts AS (SELECT id, ROW_NUMBER() OVER (ORDER BY ts_rank(tsv, q) DESC) AS rank \
         FROM atoms, to_tsquery('simple', ",
    );
    qb.push_bind(tsv_query_smart(query, 3));
    qb.push(") q WHERE status = 'active' AND tsv @@ q LIMIT 100) ");

    if has_vec {
        qb.push(", vec AS (SELECT id, ROW_NUMBER() OVER (ORDER BY embedding <=> ");
        qb.push_bind(Vector::from(query_vec.unwrap().to_vec()));
        qb.push(
            ") AS rank FROM atoms WHERE status = 'active' AND embedding IS NOT NULL LIMIT 100) ",
        );
    }

    qb.push("SELECT a.id, a.kind, a.content, COALESCE(1.0/(");
    qb.push_bind(RRF_K);
    qb.push(" + fts.rank), 0)");
    if has_vec {
        qb.push(" + COALESCE(1.0/(");
        qb.push_bind(RRF_K);
        qb.push(" + vec.rank), 0)");
    }
    qb.push("::float8 AS score FROM atoms a LEFT JOIN fts ON fts.id = a.id ");
    if has_vec {
        qb.push("LEFT JOIN vec ON vec.id = a.id ");
    }
    qb.push("WHERE a.status = 'active' AND (fts.id IS NOT NULL");
    if has_vec {
        qb.push(" OR vec.id IS NOT NULL");
    }
    qb.push(") ORDER BY score DESC LIMIT ");
    qb.push_bind(limit);

    let rows = qb.build().fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|r| SearchHit {
            id: r.get("id"),
            score: r.get::<f64, _>("score"),
            title: None,
            snippet: r.get("content"),
            kind: r.get("kind"),
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
    let has_vec = query_vec.is_some();
    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
        "WITH fts AS (SELECT id, ROW_NUMBER() OVER (ORDER BY ts_rank(tsv, q) DESC) AS rank \
         FROM scenarios, to_tsquery('simple', ",
    );
    qb.push_bind(tsv_query_smart(query, 3));
    qb.push(") q WHERE tsv @@ q LIMIT 100) ");

    if has_vec {
        qb.push(", vec AS (SELECT id, ROW_NUMBER() OVER (ORDER BY embedding <=> ");
        qb.push_bind(Vector::from(query_vec.unwrap().to_vec()));
        qb.push(") AS rank FROM scenarios WHERE embedding IS NOT NULL LIMIT 100) ");
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
            score: r.get::<f64, _>("score"),
            title: r.get::<Option<String>, _>("topic"),
            snippet: r.get("summary"),
            kind: None,
        })
        .collect())
}

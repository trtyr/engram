//! wiki 文档域仓储：wiki_documents / wiki_chunks 两表读写 + 混合检索。
//!
//! 事务说明：本域无跨表事务需求（摄取管道按 job 单语句推进）；
//! 混合检索（fts/vec CTE + RRF）的动态 QueryBuilder 原样落在本仓储。
//!
//! 多库（0037）：两表均带 `library_id`（FK → wiki_libraries），
//! 本域全部函数要求调用方传入 `lib: Uuid`（库 id），SQL 一律按库过滤/写入：
//! 单表查询按本表 library_id，跨表 JOIN 按 documents.library_id；
//! sha 去重从全局唯一收窄为 (library_id, sha256) 库内唯一。

use chrono::{DateTime, Utc};
use pgvector::Vector;
use sqlx::Row;
use uuid::Uuid;

use crate::PgPool;
use crate::error::StoreResult;
use crate::models::wiki_docs::{ChunkHitRow, DocumentDto};

// ---------- 文档（wiki_documents） ----------

/// 文档列表（库内）：status 过滤 + created_at 游标分页（新→旧）。
pub async fn list_documents(
    pool: &PgPool,
    lib: Uuid,
    status: Option<&str>,
    cursor: Option<DateTime<Utc>>,
    limit: i64,
) -> StoreResult<Vec<DocumentDto>> {
    let rows = sqlx::query_as::<_, DocumentDto>(
        "SELECT * FROM wiki_documents \
         WHERE library_id = $1 \
           AND ($2::text IS NULL OR status = $2) \
           AND ($3::timestamptz IS NULL OR created_at < $3) \
         ORDER BY created_at DESC LIMIT $4",
    )
    .bind(lib)
    .bind(status)
    .bind(cursor)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 库内按 id 取文档；不存在（含不属于该库）返回 None。
pub async fn get_document(pool: &PgPool, lib: Uuid, id: Uuid) -> StoreResult<Option<DocumentDto>> {
    sqlx::query_as::<_, DocumentDto>(
        "SELECT * FROM wiki_documents WHERE library_id = $1 AND id = $2",
    )
    .bind(lib)
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

/// 文档的块列表（原文顺序）：(seq, content, embed_failed)。
pub async fn list_chunks(
    pool: &PgPool,
    lib: Uuid,
    document_id: Uuid,
    limit: i64,
) -> StoreResult<Vec<(i32, String, bool)>> {
    let rows = sqlx::query_as(
        "SELECT seq, content, embed_failed FROM wiki_chunks \
         WHERE library_id = $1 AND document_id = $2 ORDER BY seq LIMIT $3",
    )
    .bind(lib)
    .bind(document_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 文档当前状态；不存在（含不属于该库）返回 None。
pub async fn get_document_status(
    pool: &PgPool,
    lib: Uuid,
    id: Uuid,
) -> StoreResult<Option<String>> {
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM wiki_documents WHERE library_id = $1 AND id = $2")
            .bind(lib)
            .bind(id)
            .fetch_optional(pool)
            .await?;
    Ok(status)
}

/// 删除文档（级联 chunks），返回落盘原始文件路径（供服务层清文件）；不存在（含不属于该库）返回 None。
pub async fn delete_document_returning_path(
    pool: &PgPool,
    lib: Uuid,
    id: Uuid,
) -> StoreResult<Option<String>> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        "DELETE FROM wiki_documents WHERE library_id = $1 AND id = $2 RETURNING raw_path",
    )
    .bind(lib)
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|(raw_path,)| raw_path))
}

// ---------- 摄取（enqueue_ingest 幂等/自愈/回滚） ----------

/// K6：INSERT ... ON CONFLICT (library_id, sha256) 单往返幂等——库内 sha 去重，
/// 返回 Some(新id) / None（该库内 sha 已存在；跨库同 sha 各自独立入库）。
#[allow(clippy::too_many_arguments)]
pub async fn insert_document_sha(
    pool: &PgPool,
    lib: Uuid,
    id: Uuid,
    title: &str,
    source_uri: &str,
    mime: Option<&str>,
    raw_path: &str,
    sha: &str,
) -> StoreResult<Option<Uuid>> {
    let inserted: Option<Uuid> = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO wiki_documents (id, library_id, title, source_uri, mime, raw_path, sha256, status) \
         VALUES ($1, $2, $3, $4, $5, NULLIF($6,''), $7, 'pending') \
         ON CONFLICT (library_id, sha256) DO NOTHING RETURNING id",
    )
    .bind(id)
    .bind(lib)
    .bind(title)
    .bind(source_uri)
    .bind(mime)
    .bind(raw_path)
    .bind(sha)
    .fetch_optional(pool)
    .await?;
    Ok(inserted)
}

/// 幂等命中后按 (library_id, sha256) 定位既有文档 id。
pub async fn find_document_id_by_sha(pool: &PgPool, lib: Uuid, sha: &str) -> StoreResult<Uuid> {
    let existing: Uuid =
        sqlx::query_scalar("SELECT id FROM wiki_documents WHERE library_id = $1 AND sha256 = $2")
            .bind(lib)
            .bind(sha)
            .fetch_one(pool)
            .await?;
    Ok(existing)
}

/// K1 自愈：failed / 非终态卡死（>5 分钟无 pending·running 活 job）→ 原子重置为 pending。
/// 命中返回文档 id（服务层重新入队），未命中（ready 或在途）返回 None。
pub async fn heal_stuck_document(pool: &PgPool, lib: Uuid, id: Uuid) -> StoreResult<Option<Uuid>> {
    let healed: Option<Uuid> = sqlx::query_scalar::<_, Uuid>(
        "UPDATE wiki_documents SET status = 'pending', error = NULL, updated_at = now() \
         WHERE library_id = $1 AND id = $2 AND ( \
            status = 'failed' \
            OR (status <> 'ready' \
                AND updated_at < now() - interval '5 minutes' \
                AND NOT EXISTS ( \
                    SELECT 1 FROM jobs \
                    WHERE kind IN ('parse_document','chunk_document','embed_document') \
                      AND status IN ('pending','running') \
                      AND payload->>'document_id' = $2::text)) \
         ) RETURNING id",
    )
    .bind(lib)
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(healed)
}

/// K2 回滚：入队失败时删除刚建的文档行（尽力而为，调用方忽略结果）。
pub async fn delete_document_quiet(pool: &PgPool, lib: Uuid, id: Uuid) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM wiki_documents WHERE library_id = $1 AND id = $2")
        .bind(lib)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

// ---------- 管道状态推进 ----------

/// 状态推进（parsing/chunking/embedding 同形状）。
pub async fn update_document_status(
    pool: &PgPool,
    lib: Uuid,
    doc_id: Uuid,
    status: &str,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE wiki_documents SET status = $3, updated_at = now() \
         WHERE library_id = $1 AND id = $2",
    )
    .bind(lib)
    .bind(doc_id)
    .bind(status)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 标记失败（status=failed + error，尽力而为时调用方忽略结果）。
pub async fn mark_failed_document(
    pool: &PgPool,
    lib: Uuid,
    doc_id: Uuid,
    error: &str,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE wiki_documents SET status = 'failed', error = $3, updated_at = now() \
         WHERE library_id = $1 AND id = $2",
    )
    .bind(lib)
    .bind(doc_id)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 终态 ready（清 error）。
pub async fn set_ready_document(pool: &PgPool, lib: Uuid, doc_id: Uuid) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE wiki_documents SET status = 'ready', error = NULL, updated_at = now() \
         WHERE library_id = $1 AND id = $2",
    )
    .bind(lib)
    .bind(doc_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// URL 抓取成功后落抓取产物（raw_path + mime/title COALESCE）。
pub async fn update_document_fetch_result(
    pool: &PgPool,
    lib: Uuid,
    doc_id: Uuid,
    raw_path: &str,
    mime: Option<&str>,
    title: Option<&str>,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE wiki_documents SET raw_path = $3, mime = COALESCE($4, mime), title = COALESCE($5, title), updated_at = now() \
         WHERE library_id = $1 AND id = $2",
    )
    .bind(lib)
    .bind(doc_id)
    .bind(raw_path)
    .bind(mime)
    .bind(title)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// parse_job 读源：source_uri / raw_path / mime（raw_path 可空：URL 文档摄取前无本地文件）。
pub async fn get_document_source(
    pool: &PgPool,
    lib: Uuid,
    doc_id: Uuid,
) -> StoreResult<Option<(String, Option<String>, Option<String>)>> {
    let row = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT source_uri, raw_path, mime FROM wiki_documents WHERE library_id = $1 AND id = $2",
    )
    .bind(lib)
    .bind(doc_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

// ---------- 块（wiki_chunks） ----------

/// 分块入库（重跑覆盖同 (document_id, seq) 块）；块继承所属文档的库。
pub async fn insert_chunk(
    pool: &PgPool,
    lib: Uuid,
    id: Uuid,
    document_id: Uuid,
    seq: i32,
    content: &str,
    tsv_text: &str,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "INSERT INTO wiki_chunks (id, library_id, document_id, seq, content, tsv) \
         VALUES ($1, $2, $3, $4, $5, to_tsvector('simple', $6)) \
         ON CONFLICT (document_id, seq) DO UPDATE SET content = $5, tsv = to_tsvector('simple', $6)",
    )
    .bind(id)
    .bind(lib)
    .bind(document_id)
    .bind(seq)
    .bind(content)
    .bind(tsv_text)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// K8：待嵌入块（NULL 向量或 embed_failed），按 seq 序。
pub async fn missing_chunks(
    pool: &PgPool,
    lib: Uuid,
    document_id: Uuid,
) -> StoreResult<Vec<(Uuid, String)>> {
    let rows = sqlx::query_as(
        "SELECT id, content FROM wiki_chunks \
         WHERE library_id = $1 AND document_id = $2 \
           AND (embedding IS NULL OR embed_failed) ORDER BY seq",
    )
    .bind(lib)
    .bind(document_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 块总数。
pub async fn count_chunks(pool: &PgPool, lib: Uuid, document_id: Uuid) -> StoreResult<i64> {
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM wiki_chunks WHERE library_id = $1 AND document_id = $2",
    )
    .bind(lib)
    .bind(document_id)
    .fetch_one(pool)
    .await?;
    Ok(total)
}

/// 写入块向量（嵌入成功）。
pub async fn set_chunk_embedding(
    pool: &PgPool,
    lib: Uuid,
    chunk_id: Uuid,
    embedding: Vec<f32>,
) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE wiki_chunks SET embedding = $3, embed_failed = false \
         WHERE library_id = $1 AND id = $2",
    )
    .bind(lib)
    .bind(chunk_id)
    .bind(Vector::from(embedding))
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 标记块嵌入失败（降级 FTS）。
pub async fn set_chunk_failed(pool: &PgPool, lib: Uuid, chunk_id: Uuid) -> StoreResult<u64> {
    let res =
        sqlx::query("UPDATE wiki_chunks SET embed_failed = true WHERE library_id = $1 AND id = $2")
            .bind(lib)
            .bind(chunk_id)
            .execute(pool)
            .await?;
    Ok(res.rows_affected())
}

// ---------- 混合检索 ----------

/// 混合检索 chunks（FTS + 向量 + RRF，带文档引用），库内检索：
/// 两个 CTE 池与外层 WHERE 均按 library_id 收窄（fts/vec 的 top-200 池不跨库）。
/// `query` 是已 token 化的 tsquery 表达式（core 的 tsv_query_smart 产物——tokenize 归 core，
/// 本 crate 不依赖 engram_search）；`qv` 为查询向量（None = 无嵌入，退化为纯 FTS）。
pub async fn search_chunks(
    pool: &PgPool,
    lib: Uuid,
    query: &str,
    qv: Option<Vec<f32>>,
    limit: i64,
) -> StoreResult<Vec<ChunkHitRow>> {
    let has_vec = qv.is_some();

    let mut qb: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(
        "WITH fts AS (SELECT c.id, ROW_NUMBER() OVER (ORDER BY ts_rank(c.tsv, q) DESC) AS rank \
         FROM wiki_chunks c, to_tsquery('simple', ",
    );
    qb.push_bind(query);
    qb.push(") q WHERE c.tsv @@ q AND c.library_id = ");
    qb.push_bind(lib);
    qb.push(" LIMIT 200) ");

    if has_vec {
        qb.push(", vec AS (SELECT c.id, ROW_NUMBER() OVER (ORDER BY c.embedding <=> ");
        qb.push_bind(Vector::from(qv.clone().unwrap()));
        qb.push(") AS rank FROM wiki_chunks c WHERE c.embedding IS NOT NULL AND c.library_id = ");
        qb.push_bind(lib);
        qb.push(" LIMIT 200) ");
    }

    qb.push("SELECT c.id, c.document_id, c.seq, c.content, c.embed_failed, COALESCE(1.0/(60 + fts.rank), 0)");
    if has_vec {
        qb.push(" + COALESCE(1.0/(60 + vec.rank), 0)");
    }
    qb.push(
        "::float8 AS score, d.title \
         FROM wiki_chunks c JOIN wiki_documents d ON d.id = c.document_id \
         LEFT JOIN fts ON fts.id = c.id ",
    );
    if has_vec {
        qb.push("LEFT JOIN vec ON vec.id = c.id ");
    }
    qb.push("WHERE d.library_id = ");
    qb.push_bind(lib);
    qb.push(" AND (fts.id IS NOT NULL");
    if has_vec {
        qb.push(" OR vec.id IS NOT NULL");
    }
    qb.push(") ORDER BY score DESC LIMIT ");
    qb.push_bind(limit);

    let rows = qb.build().fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|r| ChunkHitRow {
            id: r.get("id"),
            document_id: r.get("document_id"),
            seq: r.get("seq"),
            content: r.get("content"),
            embed_failed: r.get("embed_failed"),
            score: r.get("score"),
            title: r.get("title"),
        })
        .collect())
}

/// R9：命中间块的相邻块内容（同文档 seq±1，各截 200 字）。
/// 返回 (document_id, seq) → 片段；查不到的键（文档首尾块）自然缺席。
pub async fn neighbor_snippets(
    pool: &PgPool,
    keys: &[(Uuid, i32)],
) -> StoreResult<std::collections::HashMap<(Uuid, i32), String>> {
    if keys.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let mut qb: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(
        "SELECT document_id, seq, content FROM wiki_chunks WHERE (document_id, seq) IN (",
    );
    qb.push_values(keys, |mut b, (doc, seq)| {
        b.push_bind(doc).push_bind(seq);
    });
    qb.push(")");
    let rows = qb.build().fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                (r.get::<Uuid, _>("document_id"), r.get::<i32, _>("seq")),
                r.get::<String, _>("content").chars().take(200).collect(),
            )
        })
        .collect())
}

/// 文档所属库（摄取 job 链回查 library 用；文档不存在返回 None）。
pub async fn document_library(pool: &PgPool, doc_id: Uuid) -> StoreResult<Option<Uuid>> {
    sqlx::query_scalar("SELECT library_id FROM wiki_documents WHERE id = $1")
        .bind(doc_id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

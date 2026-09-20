//! `memory` 的实现切片（架构治理 2026-09-21：自 memory.rs 纯搬移，零行为变化）。

use super::*;

pub async fn insert_session(
    pool: &PgPool,
    id: Uuid,
    agent: &str,
    turns: &Value,
    sensitive: bool,
    metadata: &Value,
) -> StoreResult<SessionDto> {
    let row = sqlx::query_as::<_, SessionDto>(
        "INSERT INTO raw_sessions (id, agent, content, sensitive, metadata) VALUES ($1, $2, $3, $4, $5) RETURNING *",
    )
    .bind(id)
    .bind(agent)
    .bind(sqlx::types::Json(turns))
    .bind(sensitive)
    .bind(sqlx::types::Json(metadata))
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// 幂等写入（公网多Agent P001 步骤1）：client_ref 命中唯一索引时返回既有会话，不新建。
/// api_key_id/key_name_snapshot 为写者归因；key 删除后 api_key_id 置 NULL、快照名保留。
#[allow(clippy::too_many_arguments)] // 归因三参为可选直通位——重构收益低于可读性损失
pub async fn insert_session_identity(
    pool: &PgPool,
    id: Uuid,
    agent: &str,
    turns: &Value,
    sensitive: bool,
    metadata: &Value,
    api_key_id: Option<Uuid>,
    key_name_snapshot: Option<&str>,
    client_ref: Option<&str>,
) -> StoreResult<SessionDto> {
    let inserted = sqlx::query_as::<_, SessionDto>(
        "INSERT INTO raw_sessions (id, agent, content, sensitive, metadata, api_key_id, key_name_snapshot, client_ref) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         ON CONFLICT (client_ref) WHERE client_ref IS NOT NULL DO NOTHING RETURNING *",
    )
    .bind(id)
    .bind(agent)
    .bind(sqlx::types::Json(turns))
    .bind(sensitive)
    .bind(sqlx::types::Json(metadata))
    .bind(api_key_id)
    .bind(key_name_snapshot)
    .bind(client_ref)
    .fetch_optional(pool)
    .await?;
    if let Some(row) = inserted {
        return Ok(row);
    }
    // 幂等命中：client_ref 已存在 → 返回既有会话（不新建）
    let existing =
        sqlx::query_as::<_, SessionDto>("SELECT * FROM raw_sessions WHERE client_ref = $1")
            .bind(client_ref)
            .fetch_one(pool)
            .await?;
    Ok(existing)
}

pub async fn insert_session_import(
    pool: &PgPool,
    id: Uuid,
    agent: &str,
    turns: &Value,
) -> StoreResult<SessionDto> {
    let row = sqlx::query_as::<_, SessionDto>(
        "INSERT INTO raw_sessions (id, agent, content, metadata) VALUES ($1, $2, $3, $4) RETURNING *",
    )
    .bind(id)
    .bind(agent)
    .bind(sqlx::types::Json(turns))
    .bind(serde_json::json!({"source": "import"}))
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn find_session(pool: &PgPool, id: Uuid) -> StoreResult<Option<SessionDto>> {
    sqlx::query_as::<_, SessionDto>("SELECT * FROM raw_sessions WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn list_sessions(
    pool: &PgPool,
    agent: Option<&str>,
    cursor: Option<DateTime<Utc>>,
    limit: i64,
) -> StoreResult<Vec<SessionDto>> {
    Ok(sqlx::query_as::<_, SessionDto>(
        "SELECT * FROM raw_sessions \
         WHERE ($1::text IS NULL OR agent = $1) AND ($2::timestamptz IS NULL OR created_at < $2) \
         ORDER BY created_at DESC LIMIT $3",
    )
    .bind(agent)
    .bind(cursor)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

/// P12：原子 jsonb 数组拼接——单语句在行级天然串行，并发 append 不丢更新
/// （分立的读-改-写在池连接上无法持锁，后写会覆盖前写的合并结果）。
pub async fn append_session_update(
    pool: &PgPool,
    id: Uuid,
    turns: &Value,
    agent: Option<&str>,
) -> StoreResult<SessionDto> {
    let row = sqlx::query_as::<_, SessionDto>(
        "UPDATE raw_sessions SET content = content || $2, agent = COALESCE($3, agent) WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(sqlx::types::Json(turns))
    .bind(agent)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn delete_session(pool: &PgPool, id: Uuid) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM raw_sessions WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

pub async fn session_distill_status(pool: &PgPool, id: Uuid) -> StoreResult<Option<String>> {
    let status: Option<String> =
        sqlx::query_scalar("SELECT distill_status FROM raw_sessions WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    Ok(status)
}

pub async fn void_session_update(pool: &PgPool, id: Uuid) -> StoreResult<Option<SessionDto>> {
    // unvoid 通道（R 报告 P2）：作废前把 distill_status 存进 metadata.pre_void_distill——
    // 恢复时照原样还原（pending 回 pending 可继续蒸馏；done 回 done），无需加列
    let row = sqlx::query_as::<_, SessionDto>(
        "UPDATE raw_sessions SET distill_status = 'void', \
         metadata = COALESCE(metadata,'{}'::jsonb) || jsonb_build_object('pre_void_distill', distill_status) \
         WHERE id = $1 AND distill_status IN ('pending','done') RETURNING *",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 恢复 void 会话：distill_status 还原自 metadata.pre_void_distill；
/// 历史遗留（修复前作废、无存档标记）按「有无源为它的原子」启发式推断 done/pending。
pub async fn unvoid_session_update(pool: &PgPool, id: Uuid) -> StoreResult<Option<SessionDto>> {
    let row = sqlx::query_as::<_, SessionDto>(
        "UPDATE raw_sessions SET \
         distill_status = COALESCE( \
             NULLIF(metadata->>'pre_void_distill', ''), \
             CASE WHEN EXISTS ( \
                 SELECT 1 FROM atoms a, jsonb_array_elements(a.source_refs) e \
                 WHERE e->>'session_id' = raw_sessions.id::text) \
             THEN 'done' ELSE 'pending' END), \
         metadata = metadata - 'pre_void_distill' \
         WHERE id = $1 AND distill_status = 'void' RETURNING *",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 节律积压：pending（排除 off）会话数 + 最老一条的创建时间。
pub async fn session_backlog(pool: &PgPool) -> StoreResult<(i64, Option<DateTime<Utc>>)> {
    let pending: (i64, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT count(*), min(created_at) FROM raw_sessions WHERE distill_status = 'pending' \
         AND COALESCE(metadata->>'distill','') <> 'off'",
    )
    .fetch_one(pool)
    .await?;
    Ok(pending)
}

/// 节律心跳：复用 jobs 审计行（kind=rhythm_heartbeat）取最近一条。
pub async fn rhythm_last_heartbeat(pool: &PgPool) -> StoreResult<Option<(DateTime<Utc>, String)>> {
    let heartbeat: Option<(DateTime<Utc>, String)> = sqlx::query_as(
        "SELECT created_at, payload->>'by' AS by FROM jobs \
         WHERE kind = 'rhythm_heartbeat' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(heartbeat)
}

/// P2 进程崩溃自愈：上次运行中被认领（processing）的会话退回 pending。
/// 返回受影响行数。
pub async fn reset_processing_sessions(pool: &PgPool) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE raw_sessions SET distill_status = 'pending' WHERE distill_status = 'processing'",
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 全量导出（P4 数据主权）：全部会话（含 sensitive——export 显式决策面）。
pub async fn list_all_sessions(pool: &PgPool) -> StoreResult<Vec<SessionDto>> {
    let rows = sqlx::query_as("SELECT * FROM raw_sessions ORDER BY created_at")
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// 会话蒸馏状态 + metadata（蒸馏回执用）。
pub async fn session_distill_meta(
    pool: &PgPool,
    session_id: Uuid,
) -> StoreResult<Option<(String, serde_json::Value)>> {
    sqlx::query_as("SELECT distill_status, metadata FROM raw_sessions WHERE id = $1")
        .bind(session_id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

/// 会话列表轻量行（浏览/定位用，不含正文大字段）：
/// turns = 轮次数，preview = 首条消息前 80 字。
pub async fn list_sessions_meta(
    pool: &PgPool,
    agent: Option<&str>,
    cursor: Option<DateTime<Utc>>,
    limit: i64,
) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object( \
            'id', s.id, 'agent', s.agent, 'distill_status', s.distill_status, \
            'sensitive', s.sensitive, 'created_at', s.created_at, 'metadata', s.metadata, \
            'turns', jsonb_array_length(s.content), \
            'preview', left(COALESCE(s.content->0->>'text', ''), 80) \
         ) FROM raw_sessions s \
         WHERE ($1::text IS NULL OR s.agent = $1) \
           AND ($2::timestamptz IS NULL OR s.created_at < $2) \
         ORDER BY s.created_at DESC LIMIT $3",
    )
    .bind(agent)
    .bind(cursor)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

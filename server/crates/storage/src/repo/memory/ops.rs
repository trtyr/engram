//! `memory` 的实现切片（架构治理 2026-09-21：自 memory.rs 纯搬移，零行为变化）。

use super::*;

/// P11/SEC-E 按 agent 清场：先归档该 agent 会话产出的 active 原子再删会话
/// （顺序敏感：删会话后 JOIN 不可判归属）。返回 (erased_sessions, archived_atoms)。
pub async fn purge_agent_tx(pool: &PgPool, agent: &str) -> StoreResult<(i64, i64)> {
    let mut tx = pool.begin().await?;
    let archived = sqlx::query(
        "UPDATE atoms SET status = 'archived', updated_at = now() \
         WHERE status = 'active' AND EXISTS ( \
            SELECT 1 FROM jsonb_array_elements(atoms.source_refs) e \
            JOIN raw_sessions s ON s.id::text = e->>'session_id' \
            WHERE s.agent = $1)",
    )
    .bind(agent)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let erased = sqlx::query("DELETE FROM raw_sessions WHERE agent = $1")
        .bind(agent)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    Ok((erased as i64, archived as i64))
}

/// F1/F2 deep purge：记忆域四层 + 实体链一键清空，单事务，返回五计数。
/// TRUNCATE CASCADE 一发解 FK——调用层负责 erase scope + confirm 双因子。
pub async fn purge_deep(pool: &PgPool) -> StoreResult<Value> {
    let mut tx = pool.begin().await?;
    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT count(*) FROM raw_sessions), \
            (SELECT count(*) FROM atoms), \
            (SELECT count(*) FROM entities WHERE merged_into IS NULL)",
    )
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("TRUNCATE atom_entities, entities, atoms, raw_sessions CASCADE")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(serde_json::json!({
        "sessions": counts.0, "atoms": counts.1, "entities": counts.2,
    }))
}

/// 全局记忆时间轴：原子（occurred_at 优先）/场景/实体按时间倒序合并。
pub async fn timeline(pool: &PgPool, limit: i64) -> StoreResult<Vec<TimelineEvent>> {
    Ok(sqlx::query_as::<_, TimelineEvent>(
        "SELECT a.id, COALESCE(a.occurred_at, a.created_at) AS at, 'atom' AS kind, a.content \
         FROM atoms a WHERE a.status = 'active' \
         UNION ALL \
         SELECT e.id, e.created_at AS at, 'entity' AS kind, e.name \
         FROM entities e WHERE e.merged_into IS NULL AND e.archived_at IS NULL \
         ORDER BY at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

/// 记忆域缺失向量统计（重嵌修复入口的状态面）。P015 场景层退役：只有 atoms。
pub async fn embedding_missing_counts(pool: &PgPool) -> StoreResult<i64> {
    let atoms_missing: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM atoms WHERE status = 'active' AND embedding IS NULL",
    )
    .fetch_one(pool)
    .await?;
    Ok(atoms_missing)
}

/// 编辑/清空类审计（T001/Q001 拍板 2026-10-03「系统里没有 job，只有日志」）：
/// 改写 logs 时间线（target=audit.<kind>，fields.audit=true，domain=memory），
/// jobs 表不再收伪造 succeeded 行；历史审计行原地留存不回填。
pub async fn audit(pool: &PgPool, kind: &str, payload: Value) {
    let fields = serde_json::json!({ "audit": true, "actor": "user", "payload": payload });
    crate::repo::logs::emit_log(
        pool,
        "info",
        &format!("audit.{kind}"),
        &format!("审计：{kind}"),
        Some(fields),
        "memory",
    )
    .await
    .ok();
}

//! `transfer` 的实现切片（架构治理 2026-09-21：自 transfer.rs 纯搬移，零行为变化）。

use super::*;

pub async fn import_session(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let res = sqlx::query(
        "INSERT INTO raw_sessions (id, agent, content, distill_status, sensitive, created_at, metadata) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (id) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(str_of(v, "agent", "unknown"))
    .bind(v.get("content").cloned().unwrap_or(serde_json::json!([])))
    .bind(str_of(v, "distill_status", "pending"))
    .bind(v.get("sensitive").and_then(|x| x.as_bool()).unwrap_or(false))
    // P019-M4：created_at NOT NULL DEFAULT——显式绑 NULL 不走 DEFAULT 会被 23502 拒，
    // 缺字段的包整域中断；对齐 import_entity 的 now() 兜底模式。
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
    .bind(v.get("metadata").cloned().unwrap_or(serde_json::json!({})))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn import_atom(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let content = str_of(v, "content", "");
    let res = sqlx::query(
        "INSERT INTO atoms (id, kind, content, confidence, status, superseded_by, sensitive, occurred_at, valid_until, source_refs, created_at, updated_at, strength, source_kind) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) ON CONFLICT (id) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(str_of(v, "kind", "fact"))
    .bind(&content)
    .bind(v.get("confidence").and_then(|x| x.as_f64()).unwrap_or(0.9) as f32)
    .bind(str_of(v, "status", "active"))
    .bind(id_of(v, "superseded_by"))
    .bind(v.get("sensitive").and_then(|x| x.as_bool()).unwrap_or(false))
    .bind(ts(v, "occurred_at"))
    .bind(ts(v, "valid_until"))
    .bind(v.get("source_refs").cloned().unwrap_or(serde_json::json!([])))
    .bind(ts(v, "created_at"))
    .bind(ts(v, "updated_at"))
    // P019-M4：断言强度两列随包保真——旧实现漏列，所有导入原子静默回落 DDL 默认
    // strength='fact'（inference 被升级成用户明示，0042 设计失效）。默认值对齐 DDL。
    .bind(str_of(v, "strength", "fact"))
    .bind(str_of(v, "source_kind", "agent_inferred"))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 回填 atoms 自引用 superseded_by（导入期置 NULL，全量入库后统一回填——t9 往返演练抓出）。
pub async fn backfill_atom_superseded_by(
    pool: &PgPool,
    id: uuid::Uuid,
    superseded_by: uuid::Uuid,
) -> StoreResult<()> {
    sqlx::query("UPDATE atoms SET superseded_by = $2 WHERE id = $1")
        .bind(id)
        .bind(superseded_by)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn import_entity(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let res = sqlx::query(
        "INSERT INTO entities (id, name, kind, summary, manually_edited, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (id) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(str_of(v, "name", ""))
    .bind(str_of(v, "kind", "topic"))
    .bind(str_of(v, "summary", ""))
    .bind(v.get("manually_edited").and_then(|x| x.as_bool()))
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
    .bind(ts(v, "updated_at").unwrap_or_else(Utc::now))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn import_entity_relation(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let res = sqlx::query(
        "INSERT INTO entity_relations (id, from_id, to_id, rel_type, weight, source, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (id) DO NOTHING",
    )
    .bind(id_of(v, "id"))
    .bind(id_of(v, "from_id"))
    .bind(id_of(v, "to_id"))
    .bind(str_of(v, "rel_type", "related_to"))
    .bind(v.get("weight").and_then(|x| x.as_i64()).unwrap_or(1) as i32)
    .bind(str_of(v, "source", "manual"))
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
    .bind(ts(v, "updated_at").unwrap_or_else(Utc::now))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// atom↔entity 关联（2026-09-22 上云核账补齐）：圈子图的边，重建要重跑 LLM 抽取——
/// 必须随包（本机 296 行 vs 云机 0）。须在 atoms 与 entities 都入库后调用。
pub async fn import_atom_entity(pool: &PgPool, v: &Value) -> StoreResult<bool> {
    let res = sqlx::query(
        "INSERT INTO atom_entities (atom_id, entity_id, created_at) \
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
    )
    .bind(id_of(v, "atom_id"))
    .bind(id_of(v, "entity_id"))
    .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// KV 导入（key 唯一冲突跳过；tsv 由生成列自算）。
pub async fn import_kv_entries(pool: &PgPool, items: &[Value]) -> StoreResult<(usize, usize)> {
    let mut imported = 0usize;
    let mut skipped = 0usize;
    for v in items {
        let tags: Vec<String> = v
            .get("tags")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let res = sqlx::query(
            "INSERT INTO kv_entries (id, key, value, context, tags, source, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (key) DO NOTHING",
        )
        .bind(v.get("id").and_then(|x| x.as_str()).and_then(|s| Uuid::parse_str(s).ok()).unwrap_or_else(Uuid::now_v7))
        .bind(str_of(v, "key", ""))
        .bind(str_of(v, "value", ""))
        .bind(str_of(v, "context", ""))
        .bind(tags)
        .bind(str_of(v, "source", "user_stated"))
        // P019-M4：NOT NULL 列缺字段兜底 now()（对齐 import_entity 模式）
        .bind(ts(v, "created_at").unwrap_or_else(Utc::now))
        .bind(ts(v, "updated_at").unwrap_or_else(Utc::now))
        .execute(pool)
        .await?;
        if res.rows_affected() > 0 {
            imported += 1;
        } else {
            skipped += 1;
        }
    }
    Ok((imported, skipped))
}

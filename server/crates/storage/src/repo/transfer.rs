//! 迁移（transfer）仓储：各域导出读取 + 幂等导入 upsert。
//!
//! 导入语义：冲突跳过（主键/slug/name 已存在即 skip），保证迁移（空机全量进）
//! 与合并（两机并集）都可重复执行；逐条 imported/skipped 由编排层汇总。
//! 派生列不迁移：atoms/wiki 的 embedding 留空（reembed 可补），tsv 导入时按
//! content 重新生成；wiki_links 图边由织入流程重算。

mod export;
mod import_memory;
mod import_ops;
mod import_wiki;
mod util;
pub use export::*;
pub use import_memory::*;
pub use import_ops::*;
pub use import_wiki::*;
pub(crate) use util::*;

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::PgPool;
use crate::error::StoreResult;

// ---------- 导出读取 ----------

/// memory 域五表 + 实体关系（不含派生列 embedding/tsv/hit_count）。
pub async fn export_memory(
    pool: &PgPool,
) -> StoreResult<(
    Vec<Value>,
    Vec<Value>,
    Vec<Value>,
    Vec<Value>,
    Vec<Value>,
    Vec<Value>,
)> {
    let sessions: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(s) FROM raw_sessions s ORDER BY s.created_at")
            .fetch_all(pool)
            .await?;
    let atoms: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(a) - 'embedding' - 'tsv' - 'hit_count' FROM atoms a ORDER BY a.created_at",
    )
    .fetch_all(pool)
    .await?;
    let scenarios: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(s) - 'embedding' - 'hit_count' FROM scenarios s ORDER BY s.created_at",
    )
    .fetch_all(pool)
    .await?;
    let persona: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(p) FROM persona_aspects p ORDER BY p.aspect, p.version",
    )
    .fetch_all(pool)
    .await?;
    let entities: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(e) FROM entities e WHERE e.merged_into IS NULL ORDER BY e.created_at",
    )
    .fetch_all(pool)
    .await?;
    let relations: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(r) FROM entity_relations r ORDER BY r.created_at")
            .fetch_all(pool)
            .await?;
    Ok((sessions, atoms, scenarios, persona, entities, relations))
}

// ---------- 导入（幂等，冲突跳过） ----------

// ---------- 待办域（0035） ----------

// ---------- KV / wiki_promotions 域（公网加固 t9 往返演练补齐 v1 覆盖缺口） ----------

/// KV 全量导出（0042；tsv 为生成列不迁移）。
pub async fn export_kv_entries(pool: &PgPool) -> StoreResult<Vec<Value>> {
    let rows: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(k) FROM kv_entries k ORDER BY k.key")
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

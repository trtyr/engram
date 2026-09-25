//! consolidate：近重复合并 + stale 降权（每周定时 + 手动）。
//!
//! 分步（架构治理 2026-09-20，纯搬移 + 抽函数）：近重复聚类合并 → 实体档案补写 →
//! 关系回溯（含第 2 层常识边）→ stale 降权 → 事件与自续入队。`run()` 只做编排。
//! 解析部分（合并项、关系项）为纯函数，可不依赖库与 LLM 单测。

use engram_jobs::JobContext;
use engram_jobs::types::{JobError, JobTemplate};
use serde_json::{Value, json};
use sqlx::Row as _;
use std::collections::HashMap;
use std::fmt::Write as _;
use uuid::Uuid;

use crate::entity_portraits::fill_entity_portraits;
use crate::llm_port::LlmRef;
use crate::prompts;

/// 关系回溯可落库的关系类型白名单（其余一律丢弃）。
const REL_TYPES: [&str; 5] = [
    "member_of",
    "located_in",
    "works_on",
    "part_of",
    "related_to",
];
/// 第 2 层（常识边）允许新建的实体种类。
const COMMON_KINDS: [&str; 5] = ["person", "project", "topic", "group", "place"];

/// 近重复合并项（已过滤缺 keep_id / 空 victims）。
struct Merge {
    keep: Uuid,
    victims: Vec<Uuid>,
}

/// 关系回溯项（已过滤空端点与非白名单类型）。
struct Relation {
    from: String,
    to: String,
    rel_type: String,
    /// 记忆明示（distill）vs 世界常识（world_knowledge，第 2 层语境边）。
    source: &'static str,
    to_kind: String,
}

pub async fn run(ctx: JobContext, llm: LlmRef) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();

    let merged = merge_near_duplicates(&ctx, llm.as_ref()).await?;

    // 2.5 实体档案（v2 抽为共享函数：organize 链路末尾也生成，实体摘要不再长期为空）
    let portraits = fill_entity_portraits(&ctx, &llm, 10)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "实体档案批量生成失败（跳过，不影响主链）");
            0
        });
    if portraits > 0 {
        tracing::info!(portraits, "实体画像已生成");
    }

    let backfilled = backfill_relations(&ctx, llm.as_ref()).await?;
    let stale = downweight_stale(pool).await?;

    ctx.emit(
        &format!(
            "整理：合并 {merged} 条近重复，降权 {stale} 条 stale，实体档案 {portraits} 份，关系回溯 {backfilled} 条"
        ),
        None,
    )
    .await
    .ok();

    schedule_next(&ctx).await?;
    Ok(json!({"merged": merged, "stale_downweighted": stale}))
}

// ---------- 1. 近重复合并 ----------

async fn merge_near_duplicates(
    ctx: &JobContext,
    llm: &dyn crate::llm_port::DistillLlm,
) -> Result<usize, JobError> {
    let pool = ctx.pool();
    let rows = fetch_near_duplicates(pool).await?;
    if rows.is_empty() {
        return Ok(0);
    }
    let user = build_merge_prompt(pool, &rows).await?;
    let out = crate::llm_port::chat_json_retrying(
        ctx,
        llm,
        engram_llm::types::Purpose::Consolidate,
        &prompts::consolidate_system(),
        &user,
        ctx.job.id,
    )
    .await?;
    apply_merges(pool, &out).await
}

/// 近重复聚类：每条 active atom 的 top-3 向量近邻（余弦距离 < 0.25 视为疑似）。
/// LATERAL 输出补 embedding 列（外层 array_agg 排序需要）+ 内层按距离排序取真 top-3。
async fn fetch_near_duplicates(
    pool: &sqlx::PgPool,
) -> Result<Vec<(Uuid, String, Vec<Uuid>)>, JobError> {
    sqlx::query_as::<_, (Uuid, String, Vec<Uuid>)>(
        "WITH near AS ( \
            SELECT a.id, a.content, array_agg(b.id ORDER BY a.embedding <=> b.embedding) AS nbrs \
            FROM atoms a JOIN LATERAL ( \
                SELECT b.id, b.embedding FROM atoms b \
                WHERE b.status = 'active' AND b.id != a.id AND b.embedding IS NOT NULL \
                  AND NOT b.sensitive \
                  AND a.embedding IS NOT NULL AND a.embedding <=> b.embedding < 0.25 \
                ORDER BY a.embedding <=> b.embedding LIMIT 3 \
            ) b ON true \
            WHERE a.status = 'active' AND a.embedding IS NOT NULL AND NOT a.sensitive \
            GROUP BY a.id, a.content \
         ) SELECT id, content, nbrs FROM near",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))
}

/// 组提示：同一原子只进一组（seen 去重），近邻内容按 id 回查。
async fn build_merge_prompt(
    pool: &sqlx::PgPool,
    rows: &[(Uuid, String, Vec<Uuid>)],
) -> Result<String, JobError> {
    let mut user = String::new();
    let mut seen = std::collections::HashSet::new();
    for (id, content, nbrs) in rows {
        if seen.contains(id) {
            continue;
        }
        for n in nbrs {
            seen.insert(*n);
        }
        seen.insert(*id);
        writeln!(user, "组：").ok();
        writeln!(user, "  id={id} {content}").ok();
        for n in nbrs {
            let c: String = sqlx::query_scalar("SELECT content FROM atoms WHERE id = $1")
                .bind(n)
                .fetch_one(pool)
                .await
                .unwrap_or_default();
            writeln!(user, "  id={n} {c}").ok();
        }
    }
    Ok(user)
}

/// 纯函数：解析合并项（缺 keep_id / 空 victims 一律跳过）。
fn parse_merges(out: &Value) -> Vec<Merge> {
    let merges = out
        .get("merges")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut parsed = Vec::with_capacity(merges.len());
    for m in merges {
        let Some(keep) = m
            .get("keep_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
        else {
            continue;
        };
        let victims: Vec<Uuid> = m
            .get("merge_ids")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                    .collect()
            })
            .unwrap_or_default();
        if victims.is_empty() {
            continue;
        }
        parsed.push(Merge { keep, victims });
    }
    parsed
}

/// 并入：victim 的 source_refs 并入 keep，victim 置 archived。
/// R1：victim 同时补 superseded_by → keep——近重复合并也是取代关系，取代链不断。
async fn apply_merges(pool: &sqlx::PgPool, out: &Value) -> Result<usize, JobError> {
    let mut merged = 0usize;
    for m in parse_merges(out) {
        let res = sqlx::query(
            "UPDATE atoms a SET status = 'archived', superseded_by = $2, updated_at = now() \
             FROM ( \
                SELECT id, source_refs FROM atoms WHERE id = ANY($1) AND status = 'active' \
             ) v \
             WHERE a.id = v.id \
             RETURNING v.source_refs",
        )
        .bind(&m.victims)
        .bind(m.keep)
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        let extra: Vec<Value> = res
            .into_iter()
            .map(|r| r.get::<Value, _>("source_refs"))
            .collect();
        sqlx::query(
            "UPDATE atoms SET source_refs = source_refs || $2::jsonb, hit_count = hit_count + 1, updated_at = now() WHERE id = $1",
        )
        .bind(m.keep)
        .bind(sqlx::types::Json(&extra))
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        merged += m.victims.len();
    }
    Ok(merged)
}

// ---------- 2.6 关系回溯 ----------

/// 存量实体（有原子）间抽关系，不依赖 session 重放：
/// 实体 + 各自原子拼成上下文，LLM 一次抽所有关系，映射名字→id 落库（best-effort）。
async fn backfill_relations(
    ctx: &JobContext,
    llm: &dyn crate::llm_port::DistillLlm,
) -> Result<usize, JobError> {
    let pool = ctx.pool();
    let entities = fetch_relation_entities(pool).await?;
    if entities.len() < 2 {
        return Ok(0);
    }
    let user = build_relation_prompt(pool, &entities).await?;
    let out = match crate::llm_port::chat_json_retrying(
        ctx,
        llm,
        engram_llm::types::Purpose::Consolidate,
        &prompts::relation_backfill_system(),
        &user,
        ctx.job.id,
    )
    .await
    {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(error = %e, "关系回溯抽取失败（跳过，不影响主链）");
            return Ok(0);
        }
    };
    let relations = parse_relations(&out);
    let mut name_id: HashMap<String, Uuid> = entities
        .iter()
        .map(|(id, name, _)| (name.clone(), *id))
        .collect();
    let mut backfilled = 0usize;
    for rel in relations {
        let Some(&fid) = name_id.get(&rel.from) else {
            continue;
        };
        let Some(tid) = resolve_to_entity(pool, &mut name_id, &rel.to, &rel.to_kind).await else {
            continue;
        };
        if fid == tid {
            continue;
        }
        if insert_relation(pool, fid, tid, &rel.rel_type, rel.source).await {
            backfilled += 1;
        }
    }
    Ok(backfilled)
}

async fn fetch_relation_entities(
    pool: &sqlx::PgPool,
) -> Result<Vec<(Uuid, String, String)>, JobError> {
    sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT e.id, e.name, e.kind FROM entities e \
         JOIN atom_entities ae ON ae.entity_id = e.id \
         WHERE e.merged_into IS NULL \
         GROUP BY e.id, e.name, e.kind \
         ORDER BY count(ae.atom_id) DESC LIMIT 40",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))
}

async fn build_relation_prompt(
    pool: &sqlx::PgPool,
    entities: &[(Uuid, String, String)],
) -> Result<String, JobError> {
    let mut lines: Vec<String> = Vec::new();
    for (eid, name, kind) in entities {
        let atoms: Vec<String> = sqlx::query_scalar(
            "SELECT a.content FROM atoms a JOIN atom_entities ae ON ae.atom_id = a.id \
             WHERE ae.entity_id = $1 AND NOT a.sensitive \
             ORDER BY a.created_at DESC LIMIT 10",
        )
        .bind(eid)
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        lines.push(format!("{name}[{kind}]：{}", atoms.join("；")));
    }
    Ok(lines.join("\n"))
}

/// 纯函数：解析关系项（空端点、非白名单类型一律丢弃）。
fn parse_relations(out: &Value) -> Vec<Relation> {
    let rels = out
        .get("relations")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let mut parsed = Vec::with_capacity(rels.len());
    for r in &rels {
        let from = r.get("from").and_then(|v| v.as_str()).map(|s| s.trim());
        let to = r.get("to").and_then(|v| v.as_str()).map(|s| s.trim());
        let rel_type = r
            .get("rel_type")
            .and_then(|v| v.as_str())
            .unwrap_or("related_to");
        // 常识边层级模型（收录哲学线 task-8）：source_hint 区分记忆明示 vs 世界常识
        let source = match r.get("source_hint").and_then(|v| v.as_str()) {
            Some("world_knowledge") => "world_knowledge",
            _ => "distill",
        };
        let to_kind = r
            .get("to_kind")
            .and_then(|v| v.as_str())
            .unwrap_or("topic")
            .to_string();
        let (Some(f), Some(t)) = (from, to) else {
            continue;
        };
        if f.is_empty() || t.is_empty() || !REL_TYPES.contains(&rel_type) {
            continue;
        }
        parsed.push(Relation {
            from: f.to_string(),
            to: t.to_string(),
            rel_type: rel_type.to_string(),
            source,
            to_kind,
        });
    }
    parsed
}

/// to 解析：列表内实体直取；列表外 → 第 2 层新实体（常识边拉入语境，一跳为止——
/// 它没有记忆挂链，图上仅通过常识边可见，永不升级为记忆）。
async fn resolve_to_entity(
    pool: &sqlx::PgPool,
    name_id: &mut HashMap<String, Uuid>,
    to: &str,
    to_kind: &str,
) -> Option<Uuid> {
    if let Some(&id) = name_id.get(to) {
        return Some(id);
    }
    if !COMMON_KINDS.contains(&to_kind) {
        return None;
    }
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM entities \
         WHERE lower(btrim(name)) = lower(btrim($1)) AND kind = $2 AND merged_into IS NULL LIMIT 1",
    )
    .bind(to)
    .bind(to_kind)
    .fetch_optional(pool)
    .await
    .unwrap_or(None);
    if let Some(id) = existing {
        name_id.insert(to.to_string(), id);
        return Some(id);
    }
    let new_id = Uuid::now_v7();
    match sqlx::query("INSERT INTO entities (id, name, kind) VALUES ($1, $2, $3)")
        .bind(new_id)
        .bind(to)
        .bind(to_kind)
        .execute(pool)
        .await
    {
        Ok(_) => {
            name_id.insert(to.to_string(), new_id);
            Some(new_id)
        }
        Err(e) => {
            tracing::warn!(error = %e, "第 2 层实体落库失败（跳过该关系）");
            None
        }
    }
}

/// 落关系（同名三元组命中即权重 +1）。返回是否成功。
async fn insert_relation(
    pool: &sqlx::PgPool,
    fid: Uuid,
    tid: Uuid,
    rel_type: &str,
    source: &str,
) -> bool {
    match sqlx::query(
        "INSERT INTO entity_relations (id, from_id, to_id, rel_type, weight, source) \
         VALUES ($1, $2, $3, $4, 1, $5) \
         ON CONFLICT (from_id, to_id, rel_type) DO UPDATE SET weight = entity_relations.weight + 1, updated_at = now()",
    )
    .bind(Uuid::now_v7())
    .bind(fid)
    .bind(tid)
    .bind(rel_type)
    .bind(source)
    .execute(pool)
    .await
    {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(error = %e, "关系回溯落库失败（不影响主链）");
            false
        }
    }
}

// ---------- 3. stale 降权 + 自续 ----------

/// stale 降权：90 天零命中 + 低置信 → confidence * 0.7。
/// B10：不再刷新 updated_at（旧写法降权动作自身刷新时间戳，条件自锁只能降一次）；
/// 判龄改 created_at（原子出生日，不受任何后续写动作影响）。
async fn downweight_stale(pool: &sqlx::PgPool) -> Result<u64, JobError> {
    let stale = sqlx::query(
        "UPDATE atoms SET confidence = confidence * 0.7 \
         WHERE status = 'active' AND hit_count = 0 AND confidence < 0.9 \
           AND created_at < now() - interval '90 days' AND confidence * 0.7 > 0.2",
    )
    .execute(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?
    .rows_affected();
    Ok(stale)
}

/// 下周再跑（幂等键按 ISO 周）；R3：full 整理完顺带画像退休检查（persona 内部自查，无陈旧则 no-op）。
async fn schedule_next(ctx: &JobContext) -> Result<(), JobError> {
    let next = chrono::Utc::now() + chrono::Duration::days(7);
    let week = next.format("%G-W%V").to_string();
    ctx.enqueue_next(
        JobTemplate::new("consolidate")
            .with_idempotency_key(format!("consolidate-{week}"))
            .with_due(next),
    )
    .await?;
    ctx.enqueue_next(JobTemplate::new("distill_persona").with_payload(json!({"scenario_ids": []})))
        .await?;
    Ok(())
}

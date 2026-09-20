//! 场景快照收敛（F3）：源数据归档/标敏感后 L2 不留化石。
//!
//! 架构治理 2026-09-20 自 organize.rs 摘出（独立关注点：快照与源数据一致性，
//! 与「LLM 组织新原子」正交）。规则：atom_refs 含非活跃成员的场景——活跃≥1 →
//! 依据活跃成员重算（atom_refs 重写为仅活跃成员，无需额外标记列）；=0 → 解散
//! （成员原子 scenario_id 置空后删除场景）。上限 20 个/轮，防一次蒸馏被打爆。

use engram_jobs::JobContext;
use engram_jobs::types::JobError;
use serde_json::Value;
use std::fmt::Write as _;
use uuid::Uuid;

use crate::llm_port::DistillLlm;
use crate::prompts;

/// 快照收敛结果。
#[derive(Default)]
pub struct Converge {
    pub dissolved: usize,
    pub recomputed: usize,
    pub touched: Vec<Uuid>,
    pub removed_texts: Vec<String>,
}

/// 快照收敛：逐场景解散或重算（重算失败留待下一轮，不影响主链）。
pub async fn converge_snapshots(
    ctx: &JobContext,
    llm: &dyn DistillLlm,
) -> Result<Converge, JobError> {
    let pool = ctx.pool();
    let stale = fetch_stale_scenarios(pool).await?;
    let mut out = Converge::default();
    for (sid, topic, members) in &stale {
        let members = parse_members(members);
        if members.is_empty() {
            dissolve_scenario(pool, *sid, &mut out.removed_texts).await?;
            out.dissolved += 1;
            out.touched.push(*sid);
            continue;
        }
        // 重算：依据活跃成员重写快照（best-effort——LLM 失败留给下一轮）。
        collect_removed_texts(pool, *sid, &mut out.removed_texts).await?;
        match recompute_scenario(ctx, llm, *sid, topic, &members).await {
            Ok(()) => {
                out.recomputed += 1;
                out.touched.push(*sid);
            }
            Err(e) => tracing::warn!(scenario = %sid, error = %e, "场景重算失败，留待下一轮"),
        }
    }
    Ok(out)
}

pub async fn emit_converge(ctx: &JobContext, converge: &Converge) {
    if converge.dissolved + converge.recomputed > 0 {
        ctx.emit(
            &format!(
                "快照收敛：重算 {} 个场景，解散 {} 个场景",
                converge.recomputed, converge.dissolved
            ),
            None,
        )
        .await
        .ok();
    }
}

async fn fetch_stale_scenarios(
    pool: &sqlx::PgPool,
) -> Result<Vec<(Uuid, String, Option<Value>)>, JobError> {
    sqlx::query_as(
        "SELECT s.id, s.topic, \
         (SELECT jsonb_agg(jsonb_build_object('id', a.id::text, 'kind', a.kind, 'content', a.content)) \
          FROM jsonb_array_elements_text(s.atom_refs) r \
          JOIN atoms a ON a.id = r::uuid AND a.status = 'active' AND NOT a.sensitive) AS members \
         FROM scenarios s \
         WHERE EXISTS ( \
            SELECT 1 FROM jsonb_array_elements_text(s.atom_refs) r \
            LEFT JOIN atoms a ON a.id = r::uuid \
            WHERE a.id IS NULL OR a.status != 'active' OR a.sensitive) \
         ORDER BY s.updated_at DESC LIMIT 20",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))
}

/// 纯函数：从 members JSON 阵列抽出 (id, kind, content)（坏元素静默丢弃）。
pub fn parse_members(members: &Option<Value>) -> Vec<(Uuid, String, String)> {
    members
        .as_ref()
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|o| {
                    let id = o.get("id")?.as_str()?.parse::<Uuid>().ok()?;
                    let kind = o.get("kind")?.as_str()?.to_string();
                    let content = o.get("content")?.as_str()?.to_string();
                    Some((id, kind, content))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 被移除表述（归档/标敏感成员内容）——传给画像分面明确剔除（F4 治）。
async fn collect_removed_texts(
    pool: &sqlx::PgPool,
    sid: Uuid,
    removed_texts: &mut Vec<String>,
) -> Result<(), JobError> {
    let removed: Vec<String> = sqlx::query_scalar(
        "SELECT a.content FROM scenarios s, jsonb_array_elements_text(s.atom_refs) r(id) \
         JOIN atoms a ON a.id = r.id::uuid \
         WHERE s.id = $1 AND (a.status != 'active' OR a.sensitive) LIMIT 20",
    )
    .bind(sid)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    removed_texts.extend(removed);
    Ok(())
}

/// 解散：无活跃成员——场景退休（成员原子摘链后删除场景）。
async fn dissolve_scenario(
    pool: &sqlx::PgPool,
    sid: Uuid,
    removed_texts: &mut Vec<String>,
) -> Result<(), JobError> {
    collect_removed_texts(pool, sid, removed_texts).await?;
    sqlx::query("UPDATE atoms SET scenario_id = NULL WHERE scenario_id = $1")
        .bind(sid)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    sqlx::query("DELETE FROM scenarios WHERE id = $1")
        .bind(sid)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(())
}

/// 依据活跃成员重算场景快照（LLM 重写 topic/summary/body + 重嵌入）。
async fn recompute_scenario(
    ctx: &JobContext,
    llm: &dyn DistillLlm,
    sid: Uuid,
    topic: &str,
    members: &[(Uuid, String, String)],
) -> Result<(), JobError> {
    let pool = ctx.pool();
    let active_ids: Vec<Uuid> = members.iter().map(|m| m.0).collect();
    let mut user = String::new();
    writeln!(user, "场景主题「{topic}」，当前活跃成员原子：").ok();
    for (id, kind, content) in members {
        writeln!(user, "id={id} [{kind}] {content}").ok();
    }
    let out = crate::llm_port::chat_json_retrying(
        ctx,
        llm,
        engram_llm::types::Purpose::Organize,
        &prompts::scenario_refresh_system(),
        &user,
        ctx.job.id,
    )
    .await?;

    let topic2 = out.get("topic").and_then(|v| v.as_str()).unwrap_or(topic);
    let summary = out.get("summary").and_then(|v| v.as_str()).unwrap_or("");
    let body = out.get("body").and_then(|v| v.as_str()).unwrap_or("");
    let text = format!("{topic2}\n{summary}\n{body}");
    sqlx::query(
        "UPDATE scenarios SET topic = $2, summary = $3, body = $4, \
         atom_refs = $5::jsonb, embedding = $6, tsv = to_tsvector('simple', $7), \
         version = version + 1, updated_at = now() WHERE id = $1",
    )
    .bind(sid)
    .bind(topic2)
    .bind(summary)
    .bind(body)
    .bind(sqlx::types::Json(&active_ids))
    .bind(pgvector::Vector::from(
        llm.embed(std::slice::from_ref(&text), ctx.job.id)
            .await?
            .first()
            .cloned()
            .unwrap_or_default(),
    ))
    .bind(engram_search::tokenize::tsv_text(&text))
    .execute(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_members_drops_malformed_elements() {
        let members = Some(json!([
            {"id": Uuid::from_u128(1).to_string(), "kind": "fact", "content": "甲"},
            {"id": "不是 uuid", "kind": "fact", "content": "乙"},
            {"kind": "fact", "content": "缺 id"},
        ]));
        let got = parse_members(&members);
        assert_eq!(got.len(), 1, "只应保留结构完整的成员");
        assert_eq!(got[0].1, "fact");
        assert_eq!(got[0].2, "甲");
        assert!(parse_members(&None).is_empty());
    }
}

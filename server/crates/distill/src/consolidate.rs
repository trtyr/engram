//! consolidate：近重复合并 + stale 降权（每周定时 + 手动）。

use agent_memory_jobs::JobContext;
use agent_memory_jobs::types::{JobError, JobTemplate};
use serde_json::json;
use sqlx::Row as _;
use uuid::Uuid;

use crate::llm_port::LlmRef;
use crate::prompts;

pub async fn run(ctx: JobContext, llm: LlmRef) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();

    // 1. 近重复聚类：每条 active atom 的 top-3 向量近邻（余弦距离 < 0.25 视为疑似）
    let rows: Vec<(Uuid, String, Vec<Uuid>)> = sqlx::query_as::<_, (Uuid, String, Vec<Uuid>)>(
        "WITH near AS ( \
            SELECT a.id, a.content, array_agg(b.id ORDER BY a.embedding <=> b.embedding) AS nbrs \
            FROM atoms a JOIN LATERAL ( \
                SELECT id FROM atoms b \
                WHERE b.status = 'active' AND b.id != a.id AND b.embedding IS NOT NULL \
                  AND a.embedding IS NOT NULL AND a.embedding <=> b.embedding < 0.25 \
                LIMIT 3 \
            ) b ON true \
            WHERE a.status = 'active' AND a.embedding IS NOT NULL \
            GROUP BY a.id, a.content \
         ) SELECT id, content, nbrs FROM near",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    let mut merged = 0usize;
    if !rows.is_empty() {
        let mut user = String::new();
        use std::fmt::Write as _;
        let mut seen = std::collections::HashSet::new();
        for (id, content, nbrs) in &rows {
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

        let out = crate::llm_port::chat_json_retrying(
            &ctx,
            llm.as_ref(),
            agent_memory_llm::types::Purpose::Consolidate,
            &prompts::consolidate_system(),
            &user,
            ctx.job.id,
        )
        .await?;

        let merges = out
            .get("merges")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for m in merges {
            let keep = m
                .get("keep_id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok());
            let Some(keep) = keep else { continue };
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
            // 并入：victim 的 source_refs 并入 keep，victim 置 archived
            let res = sqlx::query(
                "UPDATE atoms a SET status = 'archived', updated_at = now() \
                 FROM ( \
                    SELECT id, source_refs FROM atoms WHERE id = ANY($1) AND status = 'active' \
                 ) v \
                 WHERE a.id = v.id \
                 RETURNING v.source_refs",
            )
            .bind(&victims)
            .fetch_all(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
            let extra: Vec<serde_json::Value> = res
                .into_iter()
                .map(|r| r.get::<serde_json::Value, _>("source_refs"))
                .collect();
            sqlx::query(
                "UPDATE atoms SET source_refs = source_refs || $2::jsonb, hit_count = hit_count + 1, updated_at = now() WHERE id = $1",
            )
            .bind(keep)
            .bind(sqlx::types::Json(&extra))
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
            merged += victims.len();
        }
    }

    // 2. stale 降权：90 天零命中 + 低置信 → confidence * 0.7
    // B10：不再刷新 updated_at（旧写法降权动作自身刷新时间戳，条件自锁只能降一次）；
    // 判龄改 created_at（原子出生日，不受任何后续写动作影响）
    let stale = sqlx::query(
        "UPDATE atoms SET confidence = confidence * 0.7 \
         WHERE status = 'active' AND hit_count = 0 AND confidence < 0.9 \
           AND created_at < now() - interval '90 days' AND confidence * 0.7 > 0.2",
    )
    .execute(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?
    .rows_affected();

    ctx.emit(
        &format!("整理：合并 {merged} 条近重复，降权 {stale} 条 stale"),
        None,
    )
    .await
    .ok();

    // 3. 下周再跑（幂等键按 ISO 周）
    let next = chrono::Utc::now() + chrono::Duration::days(7);
    let week = next.format("%G-W%V").to_string();
    ctx.enqueue_next(
        JobTemplate::new("consolidate")
            .with_idempotency_key(format!("consolidate-{week}"))
            .with_due(next),
    )
    .await?;

    Ok(json!({"merged": merged, "stale_downweighted": stale}))
}

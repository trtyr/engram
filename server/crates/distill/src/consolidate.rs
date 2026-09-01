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
    //    修：LATERAL 输出补 embedding 列（外层 array_agg 排序需要）+ 内层按距离排序取真 top-3
    let rows: Vec<(Uuid, String, Vec<Uuid>)> = sqlx::query_as::<_, (Uuid, String, Vec<Uuid>)>(
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
            // R1：victim 同时补 superseded_by → keep——近重复合并也是取代关系，
            // 取代链不断（此前归档孤儿无指向，"谁取代谁"不可溯）。
            let res = sqlx::query(
                "UPDATE atoms a SET status = 'archived', superseded_by = $2, updated_at = now() \
                 FROM ( \
                    SELECT id, source_refs FROM atoms WHERE id = ANY($1) AND status = 'active' \
                 ) v \
                 WHERE a.id = v.id \
                 RETURNING v.source_refs",
            )
            .bind(&victims)
            .bind(keep)
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

    // 2.5 实体档案：有原子且摘要滞后（空摘要或有更新原子）→ LLM 聚合切片。
    //     门槛从 ≥3 降到 ≥1：稀疏实体（当前数据每个实体常只有 1 条原子）也应获得画像；
    //     LIMIT 10 + 密度降序保证高密度优先，best-effort 失败不拖垮主链。
    let portrait_candidates: Vec<(Uuid, String, String)> =
        sqlx::query_as::<_, (Uuid, String, String)>(
            "SELECT e.id, e.name, e.kind FROM entities e \
         JOIN atom_entities ae ON ae.entity_id = e.id \
         JOIN atoms a ON a.id = ae.atom_id \
         WHERE e.merged_into IS NULL AND NOT e.manually_edited \
         GROUP BY e.id, e.name, e.kind, e.summary, e.updated_at \
         HAVING count(ae.atom_id) >= 1 AND (e.summary = '' OR max(a.created_at) > e.updated_at) \
         ORDER BY count(ae.atom_id) DESC LIMIT 10",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

    let mut portraits = 0usize;
    for (eid, name, kind) in &portrait_candidates {
        let atoms: Vec<String> = sqlx::query_scalar(
            "SELECT a.content FROM atoms a JOIN atom_entities ae ON ae.atom_id = a.id \
             WHERE ae.entity_id = $1 AND a.status = 'active' AND NOT a.sensitive \
         ORDER BY a.created_at DESC LIMIT 20",
        )
        .bind(eid)
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        let user = format!(
            "实体：{name}（{kind}）\n\n涉及记忆：\n{}",
            atoms
                .iter()
                .map(|c| format!("- {c}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let portrait = async {
            let out = crate::llm_port::chat_json_retrying(
                &ctx,
                llm.as_ref(),
                agent_memory_llm::types::Purpose::Consolidate,
                &prompts::entity_portrait_system(),
                &user,
                ctx.job.id,
            )
            .await?;
            let summary = out
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if summary.is_empty() {
                return Ok::<Option<String>, JobError>(None); // 模型没给 → 跳过，不算失败
            }
            sqlx::query("UPDATE entities SET summary = $2, updated_at = now() WHERE id = $1")
                .bind(eid)
                .bind(&summary)
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
            Ok(Some(summary))
        };
        match portrait.await {
            Ok(Some(_)) => portraits += 1,
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error = %e, entity = %name, "实体档案生成失败（跳过，不影响主链）");
            }
        }
    }

    // 2.6 关系回溯：存量实体（有原子）间抽关系，不依赖 session 重放。
    //     实体 + 各自原子拼成上下文，LLM 一次抽所有关系，映射名字→id 落库（best-effort）。
    let rel_entities: Vec<(Uuid, String, String)> = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT e.id, e.name, e.kind FROM entities e \
         JOIN atom_entities ae ON ae.entity_id = e.id \
         WHERE e.merged_into IS NULL \
         GROUP BY e.id, e.name, e.kind \
         ORDER BY count(ae.atom_id) DESC LIMIT 40",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    let mut backfilled = 0usize;
    if rel_entities.len() >= 2 {
        let mut lines: Vec<String> = Vec::new();
        for (eid, name, kind) in &rel_entities {
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
        let user = lines.join("\n");
        match crate::llm_port::chat_json_retrying(
            &ctx,
            llm.as_ref(),
            agent_memory_llm::types::Purpose::Consolidate,
            &prompts::relation_backfill_system(),
            &user,
            ctx.job.id,
        )
        .await
        {
            Ok(out) => {
                if let Some(rels) = out.get("relations").and_then(|r| r.as_array()) {
                    let name_id: std::collections::HashMap<&str, Uuid> = rel_entities
                        .iter()
                        .map(|(id, name, _)| (name.as_str(), *id))
                        .collect();
                    for r in rels {
                        let from = r.get("from").and_then(|v| v.as_str()).map(|s| s.trim());
                        let to = r.get("to").and_then(|v| v.as_str()).map(|s| s.trim());
                        let rel_type = r
                            .get("rel_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("related_to");
                        if let (Some(f), Some(t)) = (from, to)
                            && !f.is_empty()
                            && !t.is_empty()
                            && matches!(
                                rel_type,
                                "member_of" | "located_in" | "works_on" | "part_of" | "related_to"
                            )
                            && let (Some(&fid), Some(&tid)) = (name_id.get(f), name_id.get(t))
                            && fid != tid
                        {
                            match sqlx::query(
                                "INSERT INTO entity_relations (id, from_id, to_id, rel_type, weight, source) \
                                 VALUES ($1, $2, $3, $4, 1, 'distill') \
                                 ON CONFLICT (from_id, to_id, rel_type) DO UPDATE SET weight = entity_relations.weight + 1, updated_at = now()",
                            )
                            .bind(Uuid::now_v7())
                            .bind(fid)
                            .bind(tid)
                            .bind(rel_type)
                            .execute(pool)
                            .await
                            {
                                Ok(_) => backfilled += 1,
                                Err(e) => tracing::warn!(error = %e, "关系回溯落库失败（不影响主链）"),
                            }
                        }
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "关系回溯抽取失败（跳过，不影响主链）"),
        }
    }

    // 3. stale 降权：90 天零命中 + 低置信 → confidence * 0.7
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
        &format!("整理：合并 {merged} 条近重复，降权 {stale} 条 stale，实体档案 {portraits} 份，关系回溯 {backfilled} 条"),
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
    // R3：full 整理完顺带画像退休检查（persona 内部自查分面年龄，无陈旧则 no-op）
    ctx.enqueue_next(JobTemplate::new("distill_persona").with_payload(json!({"scenario_ids": []})))
        .await?;

    Ok(json!({"merged": merged, "stale_downweighted": stale}))
}

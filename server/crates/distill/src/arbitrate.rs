//! arbitrate：候选 × 既有相似 → 新增 / 去重 / 矛盾取代。

use agent_memory_jobs::JobContext;
use agent_memory_jobs::types::{JobError, JobTemplate};
use serde_json::json;
use std::fmt::Write as _;
use uuid::Uuid;

use crate::llm_port::LlmRef;
use crate::prompts;

struct AtomRow {
    id: Uuid,
    content: String,
}

pub async fn run(ctx: JobContext, llm: LlmRef) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();

    // 1. 读候选（payload 指定；兜底全部 candidate）
    let ids: Vec<Uuid> = ctx
        .job
        .payload
        .0
        .get("candidate_ids")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect()
        })
        .unwrap_or_default();

    let candidates: Vec<AtomRow> = if ids.is_empty() {
        sqlx::query_as::<_, (Uuid, String)>(
            "SELECT id, content FROM atoms WHERE status = 'candidate' ORDER BY created_at LIMIT 200",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?
        .into_iter()
        .map(|(id, content)| AtomRow { id, content })
        .collect()
    } else {
        sqlx::query_as::<_, (Uuid, String)>(
            "SELECT id, content FROM atoms WHERE id = ANY($1) AND status = 'candidate'",
        )
        .bind(&ids)
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?
        .into_iter()
        .map(|(id, content)| AtomRow { id, content })
        .collect()
    };

    if candidates.is_empty() {
        return Ok(json!({"promoted": [], "duplicates": [], "superseded": []}));
    }

    // 2. 每条候选取 top-5 相似 active atoms（向量余弦；B2：候选无嵌入时 FTS 兜底，
    //    不再因 NULL <=> NULL 整行过滤而直通转正跳过仲裁）
    let mut user = String::new();
    let mut no_similar: Vec<Uuid> = Vec::new();
    for (i, c) in candidates.iter().enumerate() {
        let has_emb: bool = sqlx::query_scalar(
            "SELECT embedding IS NOT NULL FROM atoms WHERE id = $1",
        )
        .bind(c.id)
        .fetch_one(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

        let similar: Vec<(Uuid, String)> = if has_emb {
            sqlx::query_as(
                "SELECT id, content FROM atoms \
                 WHERE status = 'active' AND embedding IS NOT NULL \
                 ORDER BY embedding <=> (SELECT embedding FROM atoms WHERE id = $1) \
                 LIMIT 5",
            )
            .bind(c.id)
            .fetch_all(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?
        } else {
            // FTS 兜底：写入与查询同源 jieba 分词，语义相近的既有原子可命中
            sqlx::query_as(
                "SELECT id, content FROM atoms, to_tsquery('simple', $1) q \
                 WHERE status = 'active' AND tsv @@ q \
                 ORDER BY ts_rank(tsv, q) DESC LIMIT 5",
            )
            .bind(agent_memory_search::tokenize::tsv_query_smart(&c.content, 3))
            .fetch_all(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?
        };

        writeln!(user, "候选[{}]: id={} 内容={}", i, c.id, c.content).ok();
        if similar.is_empty() {
            no_similar.push(c.id);
        } else {
            for (sid, scontent) in similar {
                writeln!(user, "  既有 id={} 内容={}", sid, scontent).ok();
            }
        }
    }
    let _ = &mut user;

    // 3. 无相似的直接转正；有相似的交 LLM 仲裁
    let mut promoted = no_similar.clone();
    for id in &no_similar {
        sqlx::query("UPDATE atoms SET status = 'active' WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
    }

    let mut duplicates: Vec<Uuid> = Vec::new();
    let mut superseded: Vec<(Uuid, Uuid)> = Vec::new(); // (new, old)

    if candidates.len() > no_similar.len() {
        let out = crate::llm_port::chat_json_retrying(
            &ctx,
            llm.as_ref(),
            agent_memory_llm::types::Purpose::Arbitrate,
            &prompts::arbitrate_system(),
            &user,
            ctx.job.id,
        )
        .await?;

        let verdicts = out
            .get("verdicts")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let valid_ids: std::collections::HashSet<Uuid> = candidates.iter().map(|c| c.id).collect();
        for v in verdicts {
            let Some(cid) = v
                .get("candidate_id")
                .and_then(|x| x.as_str())
                .and_then(|s| Uuid::parse_str(s).ok())
            else {
                continue;
            };
            if !valid_ids.contains(&cid) {
                continue; // 防幻觉 id
            }
            let disp = v
                .get("disposition")
                .and_then(|x| x.as_str())
                .unwrap_or("new");
            let target = v
                .get("target_id")
                .and_then(|x| x.as_str())
                .and_then(|s| Uuid::parse_str(s).ok());
            match disp {
                "duplicate" => {
                    // B6：判重不再物理删除——归档 + superseded_by 指向既有条，
                    // 保留审计与恢复能力（LLM 误判时可追溯），UI 的 active 过滤天然屏蔽
                    if let Some(t) = target {
                        sqlx::query(
                            "UPDATE atoms SET status = 'archived', superseded_by = $2, updated_at = now() \
                             WHERE id = $1 AND status = 'candidate'",
                        )
                        .bind(cid)
                        .bind(t)
                        .execute(pool)
                        .await
                        .map_err(|e| JobError::Retryable(e.to_string()))?;
                        sqlx::query("UPDATE atoms SET hit_count = hit_count + 1, updated_at = now() WHERE id = $1")
                            .bind(t)
                            .execute(pool)
                            .await
                            .map_err(|e| JobError::Retryable(e.to_string()))?;
                    } else {
                        // 无 target 的 duplicate（异常裁决）：仅归档保留，不动任何既有条
                        sqlx::query(
                            "UPDATE atoms SET status = 'archived', updated_at = now() \
                             WHERE id = $1 AND status = 'candidate'",
                        )
                        .bind(cid)
                        .execute(pool)
                        .await
                        .map_err(|e| JobError::Retryable(e.to_string()))?;
                    }
                    duplicates.push(cid);
                }
                "contradicts" => {
                    if let Some(t) = target {
                        // 候选转正 + 旧条 superseded
                        sqlx::query(
                            "UPDATE atoms SET status = 'active', updated_at = now() WHERE id = $1",
                        )
                        .bind(cid)
                        .execute(pool)
                        .await
                        .map_err(|e| JobError::Retryable(e.to_string()))?;
                        sqlx::query(
                            "UPDATE atoms SET status = 'superseded', superseded_by = $1, updated_at = now() WHERE id = $2 AND status = 'active'",
                        )
                        .bind(cid)
                        .bind(t)
                        .execute(pool)
                        .await
                        .map_err(|e| JobError::Retryable(e.to_string()))?;
                        superseded.push((cid, t));
                        promoted.push(cid);
                    } else {
                        // 无 target 的 contradicts 视为 new
                        sqlx::query("UPDATE atoms SET status = 'active' WHERE id = $1")
                            .bind(cid)
                            .execute(pool)
                            .await
                            .map_err(|e| JobError::Retryable(e.to_string()))?;
                        promoted.push(cid);
                    }
                }
                _ => {
                    sqlx::query("UPDATE atoms SET status = 'active' WHERE id = $1")
                        .bind(cid)
                        .execute(pool)
                        .await
                        .map_err(|e| JobError::Retryable(e.to_string()))?;
                    promoted.push(cid);
                }
            }
        }
        // LLM 漏判的候选兜底转正（不能让 candidate 滞留）
        let leftover: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM atoms WHERE id = ANY($1) AND status = 'candidate'")
                .bind(candidates.iter().map(|c| c.id).collect::<Vec<_>>())
                .fetch_all(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
        for id in leftover {
            sqlx::query("UPDATE atoms SET status = 'active' WHERE id = $1")
                .bind(id)
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
            promoted.push(id);
        }
    }

    ctx.emit(
        &format!(
            "仲裁：转正 {} / 去重 {} / 取代 {}",
            promoted.len(),
            duplicates.len(),
            superseded.len()
        ),
        None,
    )
    .await
    .ok();

    // 4. 链式入队组织（仅有转正时）
    if !promoted.is_empty() {
        ctx.enqueue_next(
            JobTemplate::new("organize_scenarios").with_payload(json!({"atom_ids": promoted})),
        )
        .await?;
    }

    Ok(json!({
        "promoted": promoted,
        "duplicates": duplicates,
        "superseded": superseded.into_iter().map(|(n, o)| json!({"new": n, "old": o})).collect::<Vec<_>>(),
    }))
}

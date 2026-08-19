//! organize：未归组 L1 → 新建/更新 L2 场景块。

use agent_memory_jobs::JobContext;
use agent_memory_jobs::types::{JobError, JobTemplate};
use serde_json::json;
use std::fmt::Write as _;
use uuid::Uuid;

use crate::llm_port::LlmRef;
use crate::prompts;

pub async fn run(ctx: JobContext, llm: LlmRef) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();

    // 1. 原子（payload 指定 ∪ 少量历史未归组，防漏）
    let ids: Vec<Uuid> = ctx
        .job
        .payload
        .0
        .get("atom_ids")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect()
        })
        .unwrap_or_default();

    let atoms: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, kind, content FROM atoms \
         WHERE status = 'active' AND (id = ANY($1) OR scenario_id IS NULL) \
         ORDER BY created_at LIMIT 300",
    )
    .bind(&ids)
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    if atoms.is_empty() {
        return Ok(json!({"scenario_ids": []}));
    }

    // 2. 既有场景清单
    let scenarios: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, topic, summary FROM scenarios ORDER BY updated_at DESC LIMIT 100",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    let mut user = String::new();
    writeln!(user, "== 新原子 ==").ok();
    for (id, kind, content) in &atoms {
        writeln!(user, "id={} [{kind}] {content}", id).ok();
    }
    writeln!(user, "\n== 既有场景 ==").ok();
    for (id, topic, summary) in &scenarios {
        writeln!(user, "id={id} 主题「{topic}」：{summary}").ok();
    }

    // 3. LLM 组织
    let out = crate::llm_port::chat_json_retrying(
        &ctx,
        llm.as_ref(),
        agent_memory_llm::types::Purpose::Organize,
        &prompts::organize_system(),
        &user,
        ctx.job.id,
    )
    .await?;

    let actions = out
        .get("actions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let atom_ids_set: std::collections::HashSet<Uuid> = atoms.iter().map(|a| a.0).collect();
    let scenario_ids_all: std::collections::HashSet<Uuid> = scenarios.iter().map(|s| s.0).collect();

    let mut touched: Vec<Uuid> = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    let mut actions_out: Vec<(Uuid, Vec<Uuid>)> = Vec::new(); // (scenario_id, atom_ids)

    for a in &actions {
        let action = a.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let topic = a
            .get("topic")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let summary = a
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let body = a
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let atom_ids: Vec<Uuid> = a
            .get("atom_ids")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                    .filter(|id| atom_ids_set.contains(id))
                    .collect()
            })
            .unwrap_or_default();

        match action {
            "create" if !topic.is_empty() && !atom_ids.is_empty() => {
                let id = Uuid::now_v7();
                sqlx::query(
                    "INSERT INTO scenarios (id, topic, summary, body, atom_refs) \
                     VALUES ($1, $2, $3, $4, $5)",
                )
                .bind(id)
                .bind(&topic)
                .bind(&summary)
                .bind(&body)
                .bind(sqlx::types::Json(&atom_ids))
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
                texts.push(format!("{topic}\n{summary}\n{body}"));
                actions_out.push((id, atom_ids));
                touched.push(id);
            }
            "update" => {
                let Some(sid) = a
                    .get("scenario_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .filter(|id| scenario_ids_all.contains(id))
                else {
                    continue;
                };
                // 追加原子引用（并集）+ 版本递增
                sqlx::query(
                    "UPDATE scenarios SET \
                        summary = $2, body = $3, \
                        atom_refs = ( \
                            SELECT COALESCE(jsonb_agg(DISTINCT x), '[]'::jsonb) FROM ( \
                                SELECT jsonb_array_elements(atom_refs) AS x FROM scenarios WHERE id = $1 \
                                UNION ALL SELECT $4::jsonb \
                            ) sub ), \
                        version = version + 1, updated_at = now() \
                     WHERE id = $1",
                )
                .bind(sid)
                .bind(&summary)
                .bind(&body)
                .bind(sqlx::types::Json(&atom_ids))
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
                let cur: String = sqlx::query_scalar("SELECT summary FROM scenarios WHERE id = $1")
                    .bind(sid)
                    .fetch_one(pool)
                    .await
                    .map_err(|e| JobError::Retryable(e.to_string()))?;
                texts.push(format!("{cur}\n{summary}\n{body}"));
                actions_out.push((sid, atom_ids));
                if !touched.contains(&sid) {
                    touched.push(sid);
                }
            }
            _ => {}
        }
    }

    // 4. 回填 atom.scenario_id + 刷新场景 embedding/tsv
    if !actions_out.is_empty() {
        let embeddings = llm.embed(&texts, ctx.job.id).await?;
        for (i, (sid, _)) in actions_out.iter().enumerate() {
            sqlx::query(
                "UPDATE scenarios SET embedding = $2, tsv = to_tsvector('simple', $3) WHERE id = $1",
            )
            .bind(sid)
            .bind(pgvector::Vector::from(embeddings.get(i).cloned().unwrap_or_default()))
            .bind(agent_memory_search::tokenize::tsv_text(&texts[i]))
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
        }
        for (sid, atom_ids) in &actions_out {
            sqlx::query("UPDATE atoms SET scenario_id = $2 WHERE id = ANY($1)")
                .bind(atom_ids)
                .bind(sid)
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
        }
    }

    ctx.emit(&format!("组织：{} 个场景有变动", touched.len()), None)
        .await
        .ok();

    // 5. 链式入队画像（仅 L2 有变动时）
    if !touched.is_empty() {
        ctx.enqueue_next(
            JobTemplate::new("distill_persona").with_payload(json!({"scenario_ids": touched})),
        )
        .await?;
    }

    Ok(json!({"scenario_ids": touched}))
}

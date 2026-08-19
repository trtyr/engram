//! persona：变动 L2 → L3 画像分面新版本（版本化 + 证据链）。

use agent_memory_jobs::JobContext;
use agent_memory_jobs::types::JobError;
use serde_json::json;
use std::fmt::Write as _;
use uuid::Uuid;

use crate::llm_port::LlmRef;
use crate::prompts;

const ASPECTS: [&str; 7] = [
    "identity",
    "preferences",
    "skills",
    "constraints",
    "communication_style",
    "goals",
    "routines",
];

pub async fn run(ctx: JobContext, llm: LlmRef) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();

    let scenario_ids: Vec<Uuid> = ctx
        .job
        .payload
        .0
        .get("scenario_ids")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect()
        })
        .unwrap_or_default();
    if scenario_ids.is_empty() {
        return Ok(json!({"updated": []}));
    }

    // 1. 变动场景 + 当前画像（每个 aspect 的最新版本）
    let scenarios: Vec<(String, String)> =
        sqlx::query_as("SELECT topic, summary FROM scenarios WHERE id = ANY($1)")
            .bind(&scenario_ids)
            .fetch_all(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;

    let current: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT DISTINCT ON (aspect) aspect, content, id::text \
         FROM persona_aspects ORDER BY aspect, version DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    let mut user = String::new();
    writeln!(user, "== 有变动的场景 ==").ok();
    for (topic, summary) in &scenarios {
        writeln!(user, "「{topic}」：{summary}").ok();
    }
    writeln!(user, "\n== 当前画像（各分面最新版）==").ok();
    for (aspect, content, _) in &current {
        writeln!(user, "[{aspect}] {content}").ok();
    }

    // 2. LLM 更新画像
    let out = crate::llm_port::chat_json_retrying(
        &ctx,
        llm.as_ref(),
        agent_memory_llm::types::Purpose::Persona,
        &prompts::persona_system(),
        &user,
        ctx.job.id,
    )
    .await?;

    let aspects = out
        .get("aspects")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut updated = Vec::new();
    for a in aspects {
        let aspect = a
            .get("aspect")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let content = a
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if !ASPECTS.contains(&aspect.as_str()) || content.is_empty() {
            continue;
        }
        // 证据链：本批场景（含其 atom_refs）
        let evidence: Vec<serde_json::Value> = sqlx::query_as::<_, (serde_json::Value,)>(
            "SELECT atom_refs FROM scenarios WHERE id = ANY($1)",
        )
        .bind(&scenario_ids)
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?
        .into_iter()
        .map(|(r,)| r)
        .collect();

        let row = sqlx::query_as::<_, (i32,)>(
            "INSERT INTO persona_aspects (id, aspect, content, evidence_refs, version, prompt_version) \
             SELECT $1, $2, $3, $4::jsonb, COALESCE((SELECT MAX(version) FROM persona_aspects WHERE aspect = $2), 0) + 1, $5 \
             RETURNING version",
        )
        .bind(Uuid::now_v7())
        .bind(&aspect)
        .bind(&content)
        .bind(sqlx::types::Json(&json!({"scenarios": scenario_ids, "atoms": evidence})))
        .bind(prompts::P_PERSONA.1.to_string())
        .fetch_one(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        updated.push(json!({"aspect": aspect, "version": row.0}));
    }

    ctx.emit(&format!("画像更新 {} 个分面", updated.len()), None)
        .await
        .ok();
    Ok(json!({"updated": updated}))
}

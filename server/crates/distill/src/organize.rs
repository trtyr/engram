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

    // F3 快照收敛（2026-08-31 终极清空测试）：atom_refs 含非 active 成员的场景
    // ——活跃≥1 → 依据活跃成员重算（atom_refs 重写为仅活跃成员，重算后自然收敛，
    // 无需额外标记列）；=0 → 解散（成员原子的 scenario_id 置空后删除场景）。
    // 源数据清空/归档后 L2 不再是化石。上限 20 个/轮，防一次蒸馏被打爆。
    let stale_scenarios: Vec<(Uuid, String, Option<serde_json::Value>)> = sqlx::query_as(
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
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    let mut dissolved: usize = 0;
    let mut recomputed: usize = 0;
    let mut converge_touched: Vec<Uuid> = Vec::new();
    // F4 治：收敛时被移除的表述（归档/标敏感成员内容）——传给画像分面明确剔除
    let mut removed_texts: Vec<String> = Vec::new();
    for (sid, topic, members) in &stale_scenarios {
        let members: Vec<(Uuid, String, String)> = members
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
            .unwrap_or_default();

        if members.is_empty() {
            // 解散：无活跃成员——场景退休；先收被移除表述（供画像剔除）
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
            dissolved += 1;
            converge_touched.push(*sid);
            continue;
        }

        // 重算：依据活跃成员重写快照（best-effort——LLM 失败留给下一轮）。
        // 先收被移除表述（旧成员中非活跃/敏感的——画像剔除用，与解散分支同收法）
        let active_ids: Vec<Uuid> = members.iter().map(|m| m.0).collect();
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
        let mut user = String::new();
        writeln!(user, "场景主题「{topic}」，当前活跃成员原子：").ok();
        for (id, kind, content) in &members {
            writeln!(user, "id={id} [{kind}] {content}").ok();
        }
        let out = match crate::llm_port::chat_json_retrying(
            &ctx,
            llm.as_ref(),
            agent_memory_llm::types::Purpose::Organize,
            &prompts::scenario_refresh_system(),
            &user,
            ctx.job.id,
        )
        .await
        {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(scenario = %sid, error = %e, "场景重算失败，留待下一轮");
                continue;
            }
        };
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
        .bind(agent_memory_search::tokenize::tsv_text(&text))
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        recomputed += 1;
        converge_touched.push(*sid);
    }
    if dissolved + recomputed > 0 {
        ctx.emit(
            &format!("快照收敛：重算 {recomputed} 个场景，解散 {dissolved} 个场景"),
            None,
        )
        .await
        .ok();
    }

    // F4 治：converge_only=true → 只跑收敛段（归档/标敏感触发的刷新），不进主组织流程
    let converge_only = ctx
        .job
        .payload
        .0
        .get("converge_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if converge_only {
        if !converge_touched.is_empty() {
            removed_texts.sort();
            removed_texts.dedup();
            removed_texts.truncate(40);
            ctx.enqueue_next(JobTemplate::new("distill_persona").with_payload(json!({
                "scenario_ids": converge_touched,
                "removed_texts": removed_texts,
            })))
            .await?;
        }
        return Ok(
            json!({"scenario_ids": converge_touched, "converged": true, "converge_only": true}),
        );
    }

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
        // F4 防：敏感原子不进组织素材（与聚类同规则，扩展到摘要链）
        "SELECT id, kind, content FROM atoms \
         WHERE status = 'active' AND NOT sensitive AND (id = ANY($1) OR scenario_id IS NULL) \
         ORDER BY created_at LIMIT 300",
    )
    .bind(&ids)
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    if atoms.is_empty() {
        // 无新原子也要让收敛结果链下去（解散/重算同样该触发画像刷新）
        if !converge_touched.is_empty() {
            ctx.enqueue_next(JobTemplate::new("distill_persona").with_payload(json!({
                "scenario_ids": converge_touched,
                "removed_texts": removed_texts,
            })))
            .await?;
        }
        return Ok(json!({"scenario_ids": converge_touched, "converged": true}));
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
                // A6：新旧两侧都 jsonb_array_elements 展开成标量再 agg——此前新侧
                // 整个数组当单个元素并进，产出 [["id"]] 嵌套混型（2026-08-31 测试方实测）。
                sqlx::query(
                    "UPDATE scenarios SET \
                        summary = $2, body = $3, \
                        atom_refs = ( \
                            SELECT COALESCE(jsonb_agg(DISTINCT x), '[]'::jsonb) FROM ( \
                                SELECT jsonb_array_elements(atom_refs) AS x FROM scenarios WHERE id = $1 \
                                UNION ALL SELECT jsonb_array_elements($4::jsonb) \
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

    // 5. 链式入队画像（L2 有变动时；收敛触达的场景并入）
    let mut all_touched = touched;
    for sid in converge_touched {
        if !all_touched.contains(&sid) {
            all_touched.push(sid);
        }
    }
    if !all_touched.is_empty() {
        removed_texts.sort();
        removed_texts.dedup();
        removed_texts.truncate(40);
        ctx.enqueue_next(JobTemplate::new("distill_persona").with_payload(json!({
            "scenario_ids": all_touched,
            "removed_texts": removed_texts,
        })))
        .await?;
    }

    Ok(json!({"scenario_ids": all_touched}))
}

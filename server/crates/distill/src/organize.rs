//! organize：未归组 L1 → 新建/更新 L2 场景块。
//!
//! 分步（架构治理 2026-09-20，纯搬移 + 抽函数）：快照收敛（F3）→ 取原子与既有场景
//! → 提示构建 → LLM 组织 → 动作落库 → 向量/反向引用回填 → 链式入队画像 → 实体画像补写。
//! `run()` 只做编排；每步一个函数（≤60 行）。

use engram_jobs::JobContext;
use engram_jobs::types::{JobError, JobTemplate};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::fmt::Write as _;
use uuid::Uuid;

use crate::llm_port::LlmRef;
use crate::prompts;
use crate::scenario_converge::{converge_snapshots, emit_converge};

/// LLM 组织动作（已过滤幻觉 id）。
struct Action {
    kind: String,
    topic: String,
    summary: String,
    body: String,
    atom_ids: Vec<Uuid>,
    scenario_id: Option<Uuid>,
}

/// 落库后的动作回执（场景 id ↔ 其原子集，供向量回填与反向引用）。
struct Applied {
    scenario_id: Uuid,
    atom_ids: Vec<Uuid>,
}

pub async fn run(ctx: JobContext, llm: LlmRef) -> Result<serde_json::Value, JobError> {
    let mut converge = converge_snapshots(&ctx, llm.as_ref()).await?;
    emit_converge(&ctx, &converge).await;

    // F4 治：converge_only=true → 只跑收敛段（归档/标敏感触发的刷新），不进主组织流程
    if payload_bool(&ctx, "converge_only") {
        return converge_only_reply(&ctx, &mut converge).await;
    }

    let atoms = fetch_atoms(&ctx).await?;
    if atoms.is_empty() {
        return no_atoms_reply(&ctx, &mut converge).await;
    }

    let scenarios = fetch_scenarios(&ctx).await?;
    let user = build_organize_prompt(&atoms, &scenarios);
    let actions = organize_with_llm(&ctx, llm.as_ref(), &user, &atoms, &scenarios).await?;

    let mut touched: Vec<Uuid> = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    let applied = apply_actions(&ctx, &actions, &mut touched, &mut texts).await?;
    refresh_embeddings(&ctx, llm.as_ref(), &applied, &texts).await?;

    ctx.emit(&format!("组织：{} 个场景有变动", touched.len()), None)
        .await
        .ok();

    let mut all_touched = touched;
    for sid in converge.touched {
        if !all_touched.contains(&sid) {
            all_touched.push(sid);
        }
    }
    enqueue_persona(&ctx, &all_touched, &mut converge.removed_texts).await?;

    // v2（N5）：实体画像随 organize 链路生成——新实体的 summary 不再等 full consolidate。
    // 小限额 best-effort，失败不影响主链。
    let portraits = match crate::entity_portraits::fill_entity_portraits(&ctx, &llm, 5).await {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(error = %e, "organize 末尾实体画像生成失败（跳过）");
            0
        }
    };

    Ok(json!({"scenario_ids": all_touched, "portraits": portraits}))
}

fn payload_bool(ctx: &JobContext, key: &str) -> bool {
    ctx.job
        .payload
        .0
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

// ---------- 主组织流程 ----------

/// 原子（payload 指定 ∪ 少量历史未归组，防漏；敏感原子不进组织素材）。
async fn fetch_atoms(ctx: &JobContext) -> Result<Vec<(Uuid, String, String)>, JobError> {
    let ids = payload_uuids(ctx, "atom_ids");
    sqlx::query_as(
        "SELECT id, kind, content FROM atoms \
         WHERE status = 'active' AND NOT sensitive AND (id = ANY($1) OR scenario_id IS NULL) \
         ORDER BY created_at LIMIT 300",
    )
    .bind(&ids)
    .fetch_all(ctx.pool())
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))
}

async fn fetch_scenarios(ctx: &JobContext) -> Result<Vec<(Uuid, String, String)>, JobError> {
    sqlx::query_as("SELECT id, topic, summary FROM scenarios ORDER BY updated_at DESC LIMIT 100")
        .fetch_all(ctx.pool())
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))
}

fn payload_uuids(ctx: &JobContext, key: &str) -> Vec<Uuid> {
    ctx.job
        .payload
        .0
        .get(key)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect()
        })
        .unwrap_or_default()
}

fn build_organize_prompt(
    atoms: &[(Uuid, String, String)],
    scenarios: &[(Uuid, String, String)],
) -> String {
    let mut user = String::new();
    writeln!(user, "== 新原子 ==").ok();
    for (id, kind, content) in atoms {
        writeln!(user, "id={} [{kind}] {content}", id).ok();
    }
    writeln!(user, "\n== 既有场景 ==").ok();
    for (id, topic, summary) in scenarios {
        writeln!(user, "id={id} 主题「{topic}」：{summary}").ok();
    }
    user
}

async fn organize_with_llm(
    ctx: &JobContext,
    llm: &dyn crate::llm_port::DistillLlm,
    user: &str,
    atoms: &[(Uuid, String, String)],
    scenarios: &[(Uuid, String, String)],
) -> Result<Vec<Action>, JobError> {
    let out = crate::llm_port::chat_json_retrying(
        ctx,
        llm,
        engram_llm::types::Purpose::Organize,
        &prompts::organize_system(),
        user,
        ctx.job.id,
    )
    .await?;
    let atom_ids_set: HashSet<Uuid> = atoms.iter().map(|a| a.0).collect();
    let scenario_ids_all: HashSet<Uuid> = scenarios.iter().map(|s| s.0).collect();
    Ok(parse_actions(&out, &atom_ids_set, &scenario_ids_all))
}

/// 纯函数：解析 LLM 组织动作（过滤幻觉原子 id / 未知场景 id）。
fn parse_actions(
    out: &Value,
    atom_ids_set: &HashSet<Uuid>,
    scenario_ids_all: &HashSet<Uuid>,
) -> Vec<Action> {
    let actions = out
        .get("actions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut parsed = Vec::with_capacity(actions.len());
    for a in &actions {
        let field = |k: &str| {
            a.get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string()
        };
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
        let scenario_id = a
            .get("scenario_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .filter(|id| scenario_ids_all.contains(id));
        parsed.push(Action {
            kind: field("action"),
            topic: field("topic"),
            summary: field("summary"),
            body: field("body"),
            atom_ids,
            scenario_id,
        });
    }
    parsed
}

/// 动作落库：create 新建场景 / update 并集追加引用 + 版本递增。
async fn apply_actions(
    ctx: &JobContext,
    actions: &[Action],
    touched: &mut Vec<Uuid>,
    texts: &mut Vec<String>,
) -> Result<Vec<Applied>, JobError> {
    let pool = ctx.pool();
    let mut applied: Vec<Applied> = Vec::new();
    for a in actions {
        match a.kind.as_str() {
            "create" if !a.topic.is_empty() && !a.atom_ids.is_empty() => {
                let id = Uuid::now_v7();
                sqlx::query(
                    "INSERT INTO scenarios (id, topic, summary, body, atom_refs) \
                     VALUES ($1, $2, $3, $4, $5)",
                )
                .bind(id)
                .bind(&a.topic)
                .bind(&a.summary)
                .bind(&a.body)
                .bind(sqlx::types::Json(&a.atom_ids))
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
                texts.push(format!("{}\n{}\n{}", a.topic, a.summary, a.body));
                applied.push(Applied {
                    scenario_id: id,
                    atom_ids: a.atom_ids.clone(),
                });
                touched.push(id);
            }
            "update" => {
                let Some(sid) = a.scenario_id else { continue };
                update_scenario(pool, sid, a).await?;
                let cur: String = sqlx::query_scalar("SELECT summary FROM scenarios WHERE id = $1")
                    .bind(sid)
                    .fetch_one(pool)
                    .await
                    .map_err(|e| JobError::Retryable(e.to_string()))?;
                texts.push(format!("{cur}\n{}\n{}", a.summary, a.body));
                applied.push(Applied {
                    scenario_id: sid,
                    atom_ids: a.atom_ids.clone(),
                });
                if !touched.contains(&sid) {
                    touched.push(sid);
                }
            }
            _ => {}
        }
    }
    Ok(applied)
}

/// A6：新旧两侧都 jsonb_array_elements 展开成标量再 agg——否则新侧整个数组当单个元素
/// 并进，产出 [["id"]] 嵌套混型（2026-08-31 测试方实测）。
async fn update_scenario(pool: &sqlx::PgPool, sid: Uuid, a: &Action) -> Result<(), JobError> {
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
    .bind(&a.summary)
    .bind(&a.body)
    .bind(sqlx::types::Json(&a.atom_ids))
    .execute(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(())
}

/// 刷新场景 embedding/tsv + 回填 atom.scenario_id。
async fn refresh_embeddings(
    ctx: &JobContext,
    llm: &dyn crate::llm_port::DistillLlm,
    applied: &[Applied],
    texts: &[String],
) -> Result<(), JobError> {
    if applied.is_empty() {
        return Ok(());
    }
    let pool = ctx.pool();
    let embeddings = llm.embed(texts, ctx.job.id).await?;
    for (i, item) in applied.iter().enumerate() {
        sqlx::query(
            "UPDATE scenarios SET embedding = $2, tsv = to_tsvector('simple', $3) WHERE id = $1",
        )
        .bind(item.scenario_id)
        .bind(pgvector::Vector::from(
            embeddings.get(i).cloned().unwrap_or_default(),
        ))
        .bind(engram_search::tokenize::tsv_text(&texts[i]))
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
    }
    for item in applied {
        sqlx::query("UPDATE atoms SET scenario_id = $2 WHERE id = ANY($1)")
            .bind(&item.atom_ids)
            .bind(item.scenario_id)
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
    }
    Ok(())
}

/// 链式入队画像（L2 有变动时）：removed_texts 排序去重截断后随 payload 传给 persona。
async fn enqueue_persona(
    ctx: &JobContext,
    scenario_ids: &[Uuid],
    removed_texts: &mut Vec<String>,
) -> Result<(), JobError> {
    if scenario_ids.is_empty() {
        return Ok(());
    }
    removed_texts.sort();
    removed_texts.dedup();
    removed_texts.truncate(40);
    ctx.enqueue_next(JobTemplate::new("distill_persona").with_payload(json!({
        "scenario_ids": scenario_ids,
        "removed_texts": removed_texts,
    })))
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn uu(n: u8) -> Uuid {
        Uuid::from_u128(n as u128)
    }

    #[test]
    fn parse_actions_keeps_valid_and_filters_hallucinated_ids() {
        let atoms: HashSet<Uuid> = [uu(1), uu(2)].into_iter().collect();
        let scenarios: HashSet<Uuid> = [uu(9)].into_iter().collect();
        let out = json!({"actions": [
            {"action": "create", "topic": " 项目 ", "summary": "s", "body": "b",
             "atom_ids": [uu(1).to_string(), uu(5).to_string()]},
            {"action": "update", "scenario_id": uu(9).to_string(), "summary": "s2",
             "atom_ids": [uu(2).to_string()]},
            {"action": "update", "scenario_id": uu(7).to_string()},
        ]});
        let got = parse_actions(&out, &atoms, &scenarios);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].kind, "create");
        assert_eq!(got[0].topic, "项目", "首尾空白应裁掉");
        assert_eq!(got[0].atom_ids, vec![uu(1)], "幻觉原子 id 应被过滤");
        assert_eq!(got[1].scenario_id, Some(uu(9)));
        assert_eq!(got[1].atom_ids, vec![uu(2)]);
        assert_eq!(got[2].scenario_id, None, "未知场景 id 应被拒");
    }

    #[test]
    fn parse_actions_tolerates_missing_payload() {
        let empty: HashSet<Uuid> = HashSet::new();
        assert!(parse_actions(&json!({}), &empty, &empty).is_empty());
        assert!(parse_actions(&json!({"actions": "非数组"}), &empty, &empty).is_empty());
    }
}

/// converge_only 快速通道：只跑收敛段（归档/标敏感触发的刷新），不进主组织流程。
async fn converge_only_reply(
    ctx: &JobContext,
    converge: &mut crate::scenario_converge::Converge,
) -> Result<serde_json::Value, JobError> {
    if !converge.touched.is_empty() {
        enqueue_persona(&ctx, &converge.touched, &mut converge.removed_texts).await?;
    }
    return Ok(json!({
        "scenario_ids": converge.touched,
        "converged": true,
        "converge_only": true
    }));
}

/// 无新原子：仍让收敛结果链下去（解散/重算同样该触发画像刷新）。
async fn no_atoms_reply(
    ctx: &JobContext,
    converge: &mut crate::scenario_converge::Converge,
) -> Result<serde_json::Value, JobError> {
    // 无新原子也要让收敛结果链下去（解散/重算同样该触发画像刷新）
    if !converge.touched.is_empty() {
        enqueue_persona(&ctx, &converge.touched, &mut converge.removed_texts).await?;
    }
    return Ok(json!({"scenario_ids": converge.touched, "converged": true}));
}

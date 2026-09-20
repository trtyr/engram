//! persona：变动 L2 → L3 画像分面新版本（版本化 + 证据链）。
//!
//! 分步（架构治理 2026-09-20，纯搬移 + 抽函数）：入参解析与退休判定 → 取数（场景/当前分面/钉住）
//! → 提示构建 → LLM 更新 → 证据链组装 → 分面落库 → 事件与回报。`run()` 只做编排。
//!
//! 两条确定性清退路径必须保留：
//! - R3 退休：无新素材且分面超 7 天未更新 → 用近期场景强制重写一次；
//! - F3/F4 化石治理：素材全空（或移除表述与分面重叠 ≥2）→ 分面写空版本（宁缺毋滥的尽头是空）。

use engram_jobs::JobContext;
use engram_jobs::types::JobError;
use serde_json::Value;
use serde_json::json;
use std::collections::{HashMap, HashSet};
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

/// 本次运行入参（payload 解析 + 全量重建展开）。
struct Inputs {
    scenario_ids: Vec<Uuid>,
    stale_refresh: Vec<String>,
    removed_texts: Vec<String>,
}

/// 入参解析的两种结局：可继续 / 已提前收尾。
enum Resolved {
    Ready(Inputs),
    Done(Value),
}

/// 待落库的分面（LLM 产出，已校验 aspect 合法且内容非空）。
struct AspectEntry {
    aspect: String,
    content: String,
    evidence_scenarios: Value,
}

pub async fn run(ctx: JobContext, llm: LlmRef) -> Result<serde_json::Value, JobError> {
    let pool = ctx.pool();
    let inputs = match resolve_inputs(&ctx).await? {
        Resolved::Ready(i) => i,
        Resolved::Done(v) => return Ok(v),
    };

    // F4 治边界：素材全空 + 有移除表述 → 确定性清退（模型见零素材会静默跳过，清退必须确定性执行）
    let scenarios = load_scenarios(pool, &inputs.scenario_ids).await?;
    if scenarios.is_empty() && !inputs.removed_texts.is_empty() {
        let retired = retire_by_removal(pool, &inputs.removed_texts).await?;
        tracing::info!(?retired, "画像清退：素材全空，重叠分面写空版本");
        return Ok(json!({"retired_by_removal": retired}));
    }

    let current = load_current_aspects(pool).await?;
    let pinned = load_pinned(pool).await?;
    let user = build_persona_prompt(
        &scenarios,
        &current,
        &inputs.stale_refresh,
        &inputs.removed_texts,
    );
    let aspects = persona_with_llm(&ctx, llm.as_ref(), &user).await?;

    let (scenario_atoms, atom_sessions) = load_evidence_maps(pool, &inputs.scenario_ids).await?;
    let updated = store_aspects(
        pool,
        &aspects,
        &pinned,
        &inputs.scenario_ids,
        &scenario_atoms,
        &atom_sessions,
    )
    .await?;

    ctx.emit(&format!("画像更新 {} 个分面", updated.len()), None)
        .await
        .ok();
    Ok(json!({"updated": updated}))
}

// ---------- 入参解析与退休判定 ----------

async fn resolve_inputs(ctx: &JobContext) -> Result<Resolved, JobError> {
    let pool = ctx.pool();
    let mut scenario_ids = payload_uuids(ctx, "scenario_ids");
    let mut stale_refresh: Vec<String> = Vec::new();

    // 全量重建（收录哲学线 task-10）：素材 = 全部场景，所有非钉住分面视为 stale（强制重写提示生效）
    if payload_bool(ctx, "full_rebuild") {
        let pinned_now = load_pinned(pool).await?;
        scenario_ids = sqlx::query_scalar("SELECT id FROM scenarios ORDER BY updated_at DESC")
            .fetch_all(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
        if scenario_ids.is_empty() {
            return Ok(Resolved::Done(json!({
                "updated": [],
                "note": "全量重建：无场景素材"
            })));
        }
        stale_refresh = ASPECTS
            .iter()
            .map(|s| s.to_string())
            .filter(|a| !pinned_now.contains(a))
            .collect();
        tracing::info!(
            n = scenario_ids.len(),
            "画像全量重建：全部场景重算所有非钉住分面"
        );
    }

    if scenario_ids.is_empty() {
        // R3 画像退休：无新素材时检查分面年龄——超 7 天未更新的分面用近期场景强制重写一次
        // （剔除过期内容：过期的相对时间/失效计划/不再成立的习惯）。说过的话比不说话更伤信任。
        let stale: Vec<String> = sqlx::query_scalar(
            "WITH latest AS (SELECT DISTINCT ON (aspect) aspect, created_at \
             FROM persona_aspects ORDER BY aspect, version DESC) \
             SELECT aspect FROM latest WHERE created_at < now() - interval '7 days'",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        if stale.is_empty() {
            return Ok(Resolved::Done(json!({"updated": []})));
        }
        tracing::info!(?stale, "画像退休：陈旧分面以近期场景重写");
        stale_refresh = stale;
        scenario_ids =
            sqlx::query_scalar("SELECT id FROM scenarios ORDER BY updated_at DESC LIMIT 30")
                .fetch_all(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
        if scenario_ids.is_empty() {
            // F3 素材全空：分面写空版本（历史不可变；UI 过滤空分面不展示）——
            // 源数据没了，画像不该继续说旧话（终极清空测试 F3 化石问题）。
            let retired = write_empty_versions(pool, &stale_refresh, "f3-empty").await?;
            tracing::info!(?retired, "画像退休：素材全空，分面写空版本");
            return Ok(Resolved::Done(json!({"retired_empty": retired})));
        }
    }

    Ok(Resolved::Ready(Inputs {
        scenario_ids,
        stale_refresh,
        removed_texts: payload_strings(ctx, "removed_texts"),
    }))
}

fn payload_bool(ctx: &JobContext, key: &str) -> bool {
    ctx.job
        .payload
        .0
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
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

fn payload_strings(ctx: &JobContext, key: &str) -> Vec<String> {
    ctx.job
        .payload
        .0
        .get(key)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// 给若干分面各写一个空内容新版本（版本号 = 当前最大 + 1）。
async fn write_empty_versions(
    pool: &sqlx::PgPool,
    aspects: &[String],
    prompt_version: &str,
) -> Result<Vec<String>, JobError> {
    let mut retired: Vec<String> = Vec::new();
    for aspect in aspects {
        let next_v: Option<Option<i32>> =
            sqlx::query_scalar("SELECT MAX(version) FROM persona_aspects WHERE aspect = $1")
                .bind(aspect)
                .fetch_optional(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
        let v = next_v.flatten().map(|x| x + 1).unwrap_or(1);
        sqlx::query(
            "INSERT INTO persona_aspects (id, aspect, content, evidence_refs, version, prompt_version) \
             VALUES ($1, $2, '', '[]'::jsonb, $3, $4)",
        )
        .bind(Uuid::now_v7())
        .bind(aspect)
        .bind(v)
        .bind(prompt_version)
        .execute(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        retired.push(aspect.clone());
    }
    Ok(retired)
}

/// F4 确定性清退：分面内容与被移除表述的 token 重叠 ≥2 → 写空版本（宁缺毋滥的尽头是空）。
async fn retire_by_removal(
    pool: &sqlx::PgPool,
    removed_texts: &[String],
) -> Result<Vec<String>, JobError> {
    let cur_facets: Vec<(String, String)> = sqlx::query_as(
        "SELECT DISTINCT ON (aspect) aspect, content \
         FROM persona_aspects ORDER BY aspect, version DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    let removed_all = removed_texts.join(" ");
    let removed_tokens: HashSet<String> = engram_search::tokenize::tokenize(&removed_all)
        .into_iter()
        .collect();
    let doomed: Vec<String> = cur_facets
        .iter()
        .filter(|(_, content)| !content.trim().is_empty())
        .filter(|(_, content)| {
            engram_search::tokenize::tokenize(content)
                .into_iter()
                .filter(|t| removed_tokens.contains(t))
                .count()
                >= 2
        })
        .map(|(aspect, _)| aspect.clone())
        .collect();
    write_empty_versions(pool, &doomed, "f4-removed-empty").await
}

// ---------- 取数 ----------

async fn load_scenarios(
    pool: &sqlx::PgPool,
    scenario_ids: &[Uuid],
) -> Result<Vec<(String, String)>, JobError> {
    sqlx::query_as("SELECT topic, summary FROM scenarios WHERE id = ANY($1)")
        .bind(scenario_ids)
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))
}

/// 当前画像：每个 aspect 的最新版本（含 id，供提示展示与钉住判定）。
async fn load_current_aspects(
    pool: &sqlx::PgPool,
) -> Result<Vec<(String, String, String)>, JobError> {
    sqlx::query_as(
        "SELECT DISTINCT ON (aspect) aspect, content, id::text \
         FROM persona_aspects ORDER BY aspect, version DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))
}

/// 编辑能力：用户钉住（manually_edited=true）的分面，蒸馏输出落库前丢弃——确定性保护。
/// 钉住分面仍作上下文喂给模型（保持整体一致性），但产出不落库。
async fn load_pinned(pool: &sqlx::PgPool) -> Result<HashSet<String>, JobError> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT aspect FROM ( \
            SELECT aspect, manually_edited, \
                   row_number() OVER (PARTITION BY aspect ORDER BY version DESC) AS rn \
            FROM persona_aspects) t \
         WHERE rn = 1 AND manually_edited",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;
    Ok(rows.into_iter().collect())
}

// ---------- 提示与 LLM ----------

fn build_persona_prompt(
    scenarios: &[(String, String)],
    current: &[(String, String, String)],
    stale_refresh: &[String],
    removed_texts: &[String],
) -> String {
    let mut user = String::new();
    writeln!(user, "== 有变动的场景 ==").ok();
    for (i, (topic, summary)) in scenarios.iter().enumerate() {
        writeln!(user, "S{} 「{topic}」：{summary}", i + 1).ok();
    }
    writeln!(user, "\n== 当前画像（各分面最新版）==").ok();
    for (aspect, content, _) in current {
        writeln!(user, "[{aspect}] {content}").ok();
    }
    if !stale_refresh.is_empty() {
        writeln!(
            user,
            "\n**以下分面必须重写（基于上方素材全量重算——旧版本中未被素材支撑的表述一律删除，宁缺毋滥）：{}**",
            stale_refresh.join(", ")
        )
        .ok();
    }
    if !removed_texts.is_empty() {
        writeln!(user, "\n== 已从记忆移除的表述（成员原子已归档/标敏感）==").ok();
        for t in removed_texts.iter().take(40) {
            writeln!(user, "- {t}").ok();
        }
        writeln!(
            user,
            "**以上表述已从记忆中移除：任何分面不得再包含其内容或同义转述，重写时直接删除。**"
        )
        .ok();
    }
    user
}

async fn persona_with_llm(
    ctx: &JobContext,
    llm: &dyn crate::llm_port::DistillLlm,
    user: &str,
) -> Result<Vec<AspectEntry>, JobError> {
    let out = crate::llm_port::chat_json_retrying(
        ctx,
        llm,
        engram_llm::types::Purpose::Persona,
        &prompts::persona_system(),
        user,
        ctx.job.id,
    )
    .await?;
    Ok(parse_aspects(&out))
}

/// 纯函数：解析 LLM 分面产出（aspect 必须在白名单内且 content 非空）。
fn parse_aspects(out: &Value) -> Vec<AspectEntry> {
    let raw = out
        .get("aspects")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut parsed = Vec::with_capacity(raw.len());
    for a in raw {
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
        parsed.push(AspectEntry {
            aspect,
            content,
            evidence_scenarios: a.get("evidence_scenarios").cloned().unwrap_or(Value::Null),
        });
    }
    parsed
}

// ---------- 证据链 ----------

/// B3：证据素材一次性查齐——场景 atom_refs + 原子 source_refs（补 L0 溯源）。
/// 返回 (scenario_atoms: sid→atom ids, atom_sessions: atom_id→session ids)。
async fn load_evidence_maps(
    pool: &sqlx::PgPool,
    scenario_ids: &[Uuid],
) -> Result<(HashMap<Uuid, Vec<Uuid>>, HashMap<Uuid, Vec<Uuid>>), JobError> {
    let scenario_atoms: HashMap<Uuid, Vec<Uuid>> = sqlx::query_as::<_, (Uuid, Value)>(
        "SELECT id, atom_refs FROM scenarios WHERE id = ANY($1)",
    )
    .bind(scenario_ids)
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?
    .into_iter()
    .map(|(sid, refs)| (sid, uuid_array(&refs)))
    .collect();

    let all_atom_ids: Vec<Uuid> = scenario_atoms.values().flatten().copied().collect();
    let atom_sessions: HashMap<Uuid, Vec<Uuid>> = if all_atom_ids.is_empty() {
        HashMap::new()
    } else {
        sqlx::query_as::<_, (Uuid, Value)>("SELECT id, source_refs FROM atoms WHERE id = ANY($1)")
            .bind(&all_atom_ids)
            .fetch_all(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?
            .into_iter()
            .map(|(aid, refs)| (aid, session_ids_of(&refs)))
            .collect()
    };
    Ok((scenario_atoms, atom_sessions))
}

fn uuid_array(v: &Value) -> Vec<Uuid> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect()
        })
        .unwrap_or_default()
}

fn session_ids_of(refs: &Value) -> Vec<Uuid> {
    refs.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    r.get("session_id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| Uuid::parse_str(s).ok())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// B3：分面级证据——LLM 标注的 S 编号映射回场景 id；
/// 非法/缺失标注回退全量场景（保守：宁多勿断链）。
fn resolve_evidence_ids(evidence: &Value, scenario_ids: &[Uuid]) -> Vec<Uuid> {
    evidence
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .filter_map(|s| s.trim_start_matches('S').parse::<usize>().ok())
                .filter(|n| (1..=scenario_ids.len()).contains(n))
                .map(|n| scenario_ids[n - 1])
                .collect::<Vec<_>>()
        })
        .filter(|v: &Vec<Uuid>| !v.is_empty())
        .unwrap_or_else(|| scenario_ids.to_vec())
}

/// 组装单个分面的证据链：依据场景 → 归属原子 → 来源会话（L3→L2→L1→L0 全链）。
fn build_evidence(
    dep_ids: &[Uuid],
    scenario_atoms: &HashMap<Uuid, Vec<Uuid>>,
    atom_sessions: &HashMap<Uuid, Vec<Uuid>>,
) -> Value {
    let mut atom_ids: Vec<Uuid> = Vec::new();
    let mut session_ids: Vec<Uuid> = Vec::new();
    for sid in dep_ids {
        for aid in scenario_atoms.get(sid).into_iter().flatten() {
            if !atom_ids.contains(aid) {
                atom_ids.push(*aid);
            }
            for sess in atom_sessions.get(aid).into_iter().flatten() {
                if !session_ids.contains(sess) {
                    session_ids.push(*sess);
                }
            }
        }
    }
    json!({
        "scenarios": dep_ids,
        "atoms": atom_ids,
        "sessions": session_ids,
    })
}

// ---------- 落库 ----------

/// 逐分面落新版本（跳过钉住分面；证据链落 `evidence_refs`）。
async fn store_aspects(
    pool: &sqlx::PgPool,
    aspects: &[AspectEntry],
    pinned: &HashSet<String>,
    scenario_ids: &[Uuid],
    scenario_atoms: &HashMap<Uuid, Vec<Uuid>>,
    atom_sessions: &HashMap<Uuid, Vec<Uuid>>,
) -> Result<Vec<Value>, JobError> {
    let mut updated = Vec::new();
    for a in aspects {
        if pinned.contains(&a.aspect) {
            continue; // 用户钉住的分面：蒸馏不覆盖（编辑能力，2026-08-31）
        }
        let dep_ids = resolve_evidence_ids(&a.evidence_scenarios, scenario_ids);
        let evidence = build_evidence(&dep_ids, scenario_atoms, atom_sessions);
        let row = sqlx::query_as::<_, (i32,)>(
            "INSERT INTO persona_aspects (id, aspect, content, evidence_refs, version, prompt_version) \
             SELECT $1, $2, $3, $4::jsonb, COALESCE((SELECT MAX(version) FROM persona_aspects WHERE aspect = $2), 0) + 1, $5 \
             RETURNING version",
        )
        .bind(Uuid::now_v7())
        .bind(&a.aspect)
        .bind(&a.content)
        .bind(sqlx::types::Json(&evidence))
        .bind(prompts::P_PERSONA.1.to_string())
        .fetch_one(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        updated.push(json!({"aspect": a.aspect, "version": row.0}));
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn uu(n: u8) -> Uuid {
        Uuid::from_u128(n as u128)
    }

    #[test]
    fn parse_aspects_filters_unknown_aspect_and_empty_content() {
        let out = json!({"aspects": [
            {"aspect": "identity", "content": " 住在上海 ", "evidence_scenarios": ["S1"]},
            {"aspect": "不是分面", "content": "x"},          // 非白名单 → 丢
            {"aspect": "goals", "content": "   "},           // 空内容 → 丢
            {"content": "缺 aspect"},                        // 缺 aspect → 丢
            {"aspect": "routines", "content": "早睡"},
        ]});
        let got = parse_aspects(&out);
        assert_eq!(got.len(), 2, "只应保留两个合法分面：{}", got.len());
        assert_eq!(got[0].aspect, "identity");
        assert_eq!(got[0].content, "住在上海", "内容应裁首尾空白");
        assert_eq!(got[1].aspect, "routines");
    }

    #[test]
    fn parse_aspects_tolerates_missing_payload() {
        assert!(parse_aspects(&json!({})).is_empty());
        assert!(parse_aspects(&json!({"aspects": "非数组"})).is_empty());
    }

    #[test]
    fn resolve_evidence_ids_maps_s_numbers_and_falls_back() {
        let ids = vec![uu(1), uu(2), uu(3)];
        // S 编号映射（1-based）
        assert_eq!(
            resolve_evidence_ids(&json!(["S2", "S3"]), &ids),
            vec![uu(2), uu(3)]
        );
        // 越界编号被丢；仍有合法项 → 不回退全量
        assert_eq!(
            resolve_evidence_ids(&json!(["S9", "S1"]), &ids),
            vec![uu(1)]
        );
        // 全非法 → 回退全量（保守：宁多勿断链）
        assert_eq!(resolve_evidence_ids(&json!(["S9", "乱写"]), &ids), ids);
        // 缺字段 → 回退全量
        assert_eq!(resolve_evidence_ids(&Value::Null, &ids), ids);
    }

    #[test]
    fn build_evidence_walks_scenario_atom_session_chain() {
        let mut scenario_atoms: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        scenario_atoms.insert(uu(1), vec![uu(10), uu(11)]);
        let mut atom_sessions: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        atom_sessions.insert(uu(10), vec![uu(100)]);
        atom_sessions.insert(uu(11), vec![uu(100), uu(101)]);
        let ev = build_evidence(&[uu(1)], &scenario_atoms, &atom_sessions);
        assert_eq!(ev["scenarios"], json!([uu(1)]));
        assert_eq!(ev["atoms"], json!([uu(10), uu(11)]));
        assert_eq!(ev["sessions"], json!([uu(100), uu(101)]), "会话应去重保序");
    }
}

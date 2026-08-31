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

    // R3 退休名单（stale 路径填充，供 prompt 明示——否则模型见素材无新意会跳过，
    // 旧内容原样滞留，2026-08-31 活体复测教训）
    let mut stale_refresh: Vec<String> = Vec::new();
    let mut scenario_ids: Vec<Uuid> = ctx
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
        // R3 画像退休：无新素材时检查分面年龄——超 7 天未更新的分面用近期场景
        // 强制重写一次（剔除过期内容：过期的相对时间/失效计划/不再成立的习惯）。
        // 说过的时话比不说话更伤信任。
        let stale: Vec<String> = sqlx::query_scalar(
            "WITH latest AS (SELECT DISTINCT ON (aspect) aspect, created_at \
             FROM persona_aspects ORDER BY aspect, version DESC) \
             SELECT aspect FROM latest WHERE created_at < now() - interval '7 days'",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        if stale.is_empty() {
            return Ok(json!({"updated": []}));
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
            let mut retired: Vec<String> = Vec::new();
            for aspect in &stale_refresh {
                let next_v: Option<Option<i32>> = sqlx::query_scalar(
                    "SELECT MAX(version) FROM persona_aspects WHERE aspect = $1",
                )
                .bind(aspect)
                .fetch_optional(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
                let v = next_v.flatten().map(|x| x + 1).unwrap_or(1);
                sqlx::query(
                    "INSERT INTO persona_aspects (id, aspect, content, evidence_refs, version, prompt_version) \
                     VALUES ($1, $2, '', '[]'::jsonb, $3, 'f3-empty')",
                )
                .bind(Uuid::now_v7())
                .bind(aspect)
                .bind(v)
                .execute(pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
                retired.push(aspect.clone());
            }
            tracing::info!(?retired, "画像退休：素材全空，分面写空版本");
            return Ok(json!({"retired_empty": retired}));
        }
    }

    // 1. 变动场景（S1..Sn 编号，LLM 证据标注用）+ 当前画像（每个 aspect 的最新版本）
    let scenarios: Vec<(String, String)> =
        sqlx::query_as("SELECT topic, summary FROM scenarios WHERE id = ANY($1)")
            .bind(&scenario_ids)
            .fetch_all(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;

    // F4 治：上游 organize 收敛传来的已移除表述——任何分面不得保留（重写时删除）。
    // 活体洞：场景解散后 constraints v1 仍抱着青霉素（2026-08-31 复测）。
    let removed_texts: Vec<String> = ctx
        .job
        .payload
        .0
        .get("removed_texts")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    // F4 治边界（测试方 seq20 头孢案）：素材全空（场景解散）+ removed_texts 非空——
    // 模型见零素材会静默跳过（R3 同族），清退必须确定性执行：分面内容与被移除
    // 表述 token 重叠 ≥2 → 直接写空 content 版本（宁缺毋滥的尽头是空）。
    let removed_all = removed_texts.join(" ");
    if scenarios.is_empty() && !removed_texts.is_empty() {
        let cur_facets: Vec<(String, String)> = sqlx::query_as(
            "SELECT DISTINCT ON (aspect) aspect, content \
             FROM persona_aspects ORDER BY aspect, version DESC",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        let removed_tokens: std::collections::HashSet<String> =
            agent_memory_search::tokenize::tokenize(&removed_all)
                .into_iter()
                .collect();
        let mut retired: Vec<String> = Vec::new();
        for (aspect, content) in &cur_facets {
            if content.trim().is_empty() {
                continue;
            }
            let overlap = agent_memory_search::tokenize::tokenize(content)
                .into_iter()
                .filter(|t| removed_tokens.contains(t))
                .count();
            if overlap < 2 {
                continue;
            }
            let next_v: Option<Option<i32>> =
                sqlx::query_scalar("SELECT MAX(version) FROM persona_aspects WHERE aspect = $1")
                    .bind(aspect)
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| JobError::Retryable(e.to_string()))?;
            let v = next_v.flatten().map(|x| x + 1).unwrap_or(1);
            sqlx::query(
                "INSERT INTO persona_aspects (id, aspect, content, evidence_refs, version, prompt_version) \
                 VALUES ($1, $2, '', '[]'::jsonb, $3, 'f4-removed-empty')",
            )
            .bind(Uuid::now_v7())
            .bind(aspect)
            .bind(v)
            .execute(pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
            retired.push(aspect.clone());
        }
        tracing::info!(?retired, "画像清退：素材全空，重叠分面写空版本");
        return Ok(serde_json::json!({"retired_by_removal": retired}));
    }

    let current: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT DISTINCT ON (aspect) aspect, content, id::text \
         FROM persona_aspects ORDER BY aspect, version DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?;

    // 编辑能力：用户钉住（manually_edited）的分面，蒸馏输出落库前丢弃——确定性保护。
    // 钉住分面仍作上下文喂给模型（保持整体一致性），但产出不落库。
    let pinned: std::collections::HashSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT aspect FROM ( \
            SELECT aspect, manually_edited, \
                   row_number() OVER (PARTITION BY aspect ORDER BY version DESC) AS rn \
            FROM persona_aspects) t \
         WHERE rn = 1 AND manually_edited",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| JobError::Retryable(e.to_string()))?
    .into_iter()
    .collect();

    let mut user = String::new();
    writeln!(user, "== 有变动的场景 ==").ok();
    for (i, (topic, summary)) in scenarios.iter().enumerate() {
        writeln!(user, "S{} 「{topic}」：{summary}", i + 1).ok();
    }
    writeln!(user, "\n== 当前画像（各分面最新版）==").ok();
    for (aspect, content, _) in &current {
        writeln!(user, "[{aspect}] {content}").ok();
    }
    if !stale_refresh.is_empty() {
        writeln!(
            user,
            "\n**注意：以下分面已超 7 天未更新，其旧版本可能含过期内容（过期的相对时间/已失效的计划/不再成立的习惯/已清除的测试数据）。必须重写这些分面：只保留能被上方场景素材直接支撑的表述，旧版本中未被素材支撑的一律删除，宁缺毋滥：{}**",
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

    // B3：证据素材一次性查齐（循环外）——场景 atom_refs + 原子 source_refs（补 L0 溯源）
    // scenario_atoms: sid -> atom ids；atom_sessions: atom_id -> session ids
    let scenario_atoms: std::collections::HashMap<Uuid, Vec<Uuid>> =
        sqlx::query_as::<_, (Uuid, serde_json::Value)>(
            "SELECT id, atom_refs FROM scenarios WHERE id = ANY($1)",
        )
        .bind(&scenario_ids)
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?
        .into_iter()
        .map(|(sid, refs)| {
            let atoms: Vec<Uuid> = refs
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                        .collect()
                })
                .unwrap_or_default();
            (sid, atoms)
        })
        .collect();

    let all_atom_ids: Vec<Uuid> = scenario_atoms.values().flatten().copied().collect();
    let atom_sessions: std::collections::HashMap<Uuid, Vec<Uuid>> = if all_atom_ids.is_empty() {
        std::collections::HashMap::new()
    } else {
        sqlx::query_as::<_, (Uuid, serde_json::Value)>(
            "SELECT id, source_refs FROM atoms WHERE id = ANY($1)",
        )
        .bind(&all_atom_ids)
        .fetch_all(pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?
        .into_iter()
        .map(|(aid, refs)| {
            let sessions: Vec<Uuid> = refs
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|r| {
                            r.get("session_id")
                                .and_then(|v| v.as_str())
                                .and_then(|s| Uuid::parse_str(s).ok())
                        })
                        .collect()
                })
                .unwrap_or_default();
            (aid, sessions)
        })
        .collect()
    };

    /// 组装单个分面的证据链：依据场景 → 归属原子 → 来源会话（L3→L2→L1→L0 全链）。
    fn build_evidence(
        dep_ids: &[Uuid],
        scenario_atoms: &std::collections::HashMap<Uuid, Vec<Uuid>>,
        atom_sessions: &std::collections::HashMap<Uuid, Vec<Uuid>>,
    ) -> serde_json::Value {
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
        serde_json::json!({
            "scenarios": dep_ids,
            "atoms": atom_ids,
            "sessions": session_ids,
        })
    }

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
        if pinned.contains(&aspect) {
            continue; // 用户钉住的分面：蒸馏不覆盖（编辑能力，2026-08-31）
        }

        // B3：分面级证据——LLM 标注的 S 编号映射回场景 id；
        // 非法/缺失标注回退全量场景（保守：宁多勿断链）
        let dep_ids: Vec<Uuid> = a
            .get("evidence_scenarios")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str())
                    .filter_map(|s| s.trim_start_matches('S').parse::<usize>().ok())
                    .filter(|n| (1..=scenario_ids.len()).contains(n))
                    .map(|n| scenario_ids[n - 1])
                    .collect::<Vec<_>>()
            })
            .filter(|v: &Vec<Uuid>| !v.is_empty())
            .unwrap_or_else(|| scenario_ids.clone());

        let evidence = build_evidence(&dep_ids, &scenario_atoms, &atom_sessions);

        let row = sqlx::query_as::<_, (i32,)>(
            "INSERT INTO persona_aspects (id, aspect, content, evidence_refs, version, prompt_version) \
             SELECT $1, $2, $3, $4::jsonb, COALESCE((SELECT MAX(version) FROM persona_aspects WHERE aspect = $2), 0) + 1, $5 \
             RETURNING version",
        )
        .bind(Uuid::now_v7())
        .bind(&aspect)
        .bind(&content)
        .bind(sqlx::types::Json(&evidence))
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

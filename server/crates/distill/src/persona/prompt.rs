//! `persona` 的实现切片（架构治理 2026-09-21：自 persona.rs 纯搬移，零行为变化）。

use super::*;

/// 给若干分面各写一个空内容新版本（版本号 = 当前最大 + 1）。
pub(crate) async fn write_empty_versions(
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
pub(crate) async fn retire_by_removal(
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

pub(crate) fn build_persona_prompt(
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

pub(crate) async fn persona_with_llm(
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
pub(crate) fn parse_aspects(out: &Value) -> Vec<AspectEntry> {
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

/// 逐分面落新版本（跳过钉住分面；证据链落 `evidence_refs`）。
pub(crate) async fn store_aspects(
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

//! `persona` 的实现切片（架构治理 2026-09-21：自 persona.rs 纯搬移，零行为变化）。

use super::*;

/// 待落库的分面（LLM 产出，已校验 aspect 合法且内容非空）。
pub(crate) struct AspectEntry {
    pub(crate) aspect: String,
    pub(crate) content: String,
    pub(crate) evidence_scenarios: Value,
}

/// B3：证据素材一次性查齐——场景 atom_refs + 原子 source_refs（补 L0 溯源）。
/// 返回 (scenario_atoms: sid→atom ids, atom_sessions: atom_id→session ids)。
pub(crate) async fn load_evidence_maps(
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

pub(crate) fn uuid_array(v: &Value) -> Vec<Uuid> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn session_ids_of(refs: &Value) -> Vec<Uuid> {
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
pub(crate) fn resolve_evidence_ids(evidence: &Value, scenario_ids: &[Uuid]) -> Vec<Uuid> {
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
pub(crate) fn build_evidence(
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

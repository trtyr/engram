//! persona：变动 L2 → L3 画像分面新版本（版本化 + 证据链）。
//!
//! 分步（架构治理 2026-09-20，纯搬移 + 抽函数）：入参解析与退休判定 → 取数（场景/当前分面/钉住）
//! → 提示构建 → LLM 更新 → 证据链组装 → 分面落库 → 事件与回报。`run()` 只做编排。
//!
//! 两条确定性清退路径必须保留：
//! - R3 退休：无新素材且分面超 7 天未更新 → 用近期场景强制重写一次；
//! - F3/F4 化石治理：素材全空（或移除表述与分面重叠 ≥2）→ 分面写空版本（宁缺毋滥的尽头是空）。

mod evidence;
mod inputs;
mod prompt;
pub(crate) use evidence::*;
pub(crate) use inputs::*;
pub(crate) use prompt::*;

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
pub(crate) struct Inputs {
    scenario_ids: Vec<Uuid>,
    stale_refresh: Vec<String>,
    removed_texts: Vec<String>,
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

// ---------- 取数 ----------

// ---------- 提示与 LLM ----------

// ---------- 证据链 ----------

// ---------- 落库 ----------

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

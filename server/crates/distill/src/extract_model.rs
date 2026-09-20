//! extract 的数据模型与纯逻辑（架构治理 2026-09-20：自 extract.rs 抽出，便于单测）。
//!
//! 这里只放**不碰库、不碰 LLM** 的东西：会话行/分段/候选原子的数据结构、
//! 分段打包（B1 覆盖率）、LLM 产出的解析与白名单过滤、时间与强度的宽容归一。
//! 副作用（认领会话、落库、挂链）留在 `extract.rs`。

use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

/// 分段字符预算（B1 覆盖率）：段内拼多轮，超预算开新段；单段一次 LLM 调用。
pub const SEGMENT_CHARS: usize = 6000;

/// 单条候选原子内容上限（字符数）。
const ATOM_MAX_CHARS: usize = 120;
/// 实体名上限（字符数）。
const ENTITY_NAME_MAX_CHARS: usize = 60;
/// 关系类型白名单（与 consolidate 同表）。
const REL_TYPES: [&str; 5] = [
    "member_of",
    "located_in",
    "works_on",
    "part_of",
    "related_to",
];
/// 实体种类白名单。
const ENTITY_KINDS: [&str; 5] = ["person", "project", "topic", "group", "place"];

/// 原始会话行（extract 内部用）。
pub struct SessionRow {
    pub id: Uuid,
    pub agent: String,
    pub content: Value,
    pub sensitive: bool,
    pub metadata: Value,
}

pub struct SegmentLine {
    pub text: String,
    pub is_header: bool,
}

/// 段内抽取产物（候选原子 + 实体挂链素材）。
pub struct PendingAtom {
    pub kind: String,
    pub content: String,
    pub confidence: f32,
    pub refs: Value,
    pub entities: Vec<(String, String)>,
    pub occurred_at: Option<chrono::DateTime<chrono::Utc>>,
    pub valid_until: Option<chrono::DateTime<chrono::Utc>>,
    pub sensitive: bool,
    pub strength: String,
}

/// 宽容 ISO8601 解析：完整 RFC3339 或 date-only（"2026-09-02" → 当日零点 UTC）。
/// LLM 输出的时间五花八门，这里只接受这两种最常见形态，其余静默丢弃（时间字段可选）。
pub fn parse_iso(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let t = s.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t) {
        return Some(dt.into());
    }
    chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|ndt| chrono::DateTime::from_naive_utc_and_offset(ndt, chrono::Utc))
}

/// 断言强度归一：fact/inference/assumption 白名单；缺失/非法一律 inference（保守——不升格）。
pub fn normalize_strength(v: Option<&str>) -> String {
    match v {
        Some(s @ ("fact" | "inference" | "assumption")) => s.to_string(),
        _ => "inference".to_string(),
    }
}

/// 构建行列表（会话头 + 全局编号轮次）→ 按预算贪心分段。
/// B1：一批会话全拼一个 prompt 时，超上下文的中间轮次会被模型静默丢弃；
/// 分段后每段独立调用，段级事件留痕，覆盖率可审计。
/// 返回（分段，轮次编号 1-based → session_id 映射）。
pub fn build_segments(sessions: &[SessionRow]) -> (Vec<Vec<SegmentLine>>, Vec<Uuid>) {
    let mut lines: Vec<SegmentLine> = Vec::new();
    let mut turn_map: Vec<Uuid> = Vec::new();
    for s in sessions {
        let is_import = s
            .metadata
            .get("source")
            .and_then(|v| v.as_str())
            .map(|src| src == "import")
            .unwrap_or(false);
        let import_hint = if is_import {
            "（批量导入的历史——对方的话是素材，不是用户本人的记忆）"
        } else {
            ""
        };
        lines.push(SegmentLine {
            text: format!("— 会话 {}（agent: {}）{}—", s.id, s.agent, import_hint),
            is_header: true,
        });
        if let Some(turns) = s.content.as_array() {
            for t in turns {
                let speaker = t.get("speaker").and_then(|v| v.as_str()).unwrap_or("?");
                let text = t.get("text").and_then(|v| v.as_str()).unwrap_or("");
                turn_map.push(s.id);
                lines.push(SegmentLine {
                    text: format!("[{}] {}: {}", turn_map.len(), speaker, text),
                    is_header: false,
                });
            }
        }
    }
    (pack_segments(lines), turn_map)
}

/// 贪心打包：会话头不落单（开新段时若末行是头，连带迁去新段）。
fn pack_segments(lines: Vec<SegmentLine>) -> Vec<Vec<SegmentLine>> {
    // 用显式「当前段」代替 `segments.last().unwrap()`：段栈永不为空是本函数的局部不变式，
    // 直接持有当前段即可表达，无需在循环体里反复 unwrap（架构治理 task-5）。
    let mut segments: Vec<Vec<SegmentLine>> = Vec::new();
    let mut current: Vec<SegmentLine> = Vec::new();
    let mut used = 0usize;
    for line in lines {
        let len = line.text.len() + 1;
        if !current.is_empty() && used + len > SEGMENT_CHARS {
            // 头不落单：上一段末行若是会话头，迁移到新段首
            let mut new_seg: Vec<SegmentLine> = Vec::new();
            if current.last().map(|l| l.is_header).unwrap_or(false)
                && let Some(header) = current.pop()
            {
                new_seg.push(header);
            }
            segments.push(std::mem::take(&mut current));
            current = new_seg;
            used = 0;
        }
        used += len;
        current.push(line);
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

/// 解析单段的 LLM 产出为候选原子（内容空/超长、confidence 越界、实体名非法一律过滤）。
pub fn parse_atoms(
    out: &Value,
    turn_map: &[Uuid],
    session_sensitive: &HashMap<Uuid, bool>,
) -> Vec<PendingAtom> {
    let atoms = out
        .get("atoms")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();
    let mut pending = Vec::with_capacity(atoms.len());
    for a in &atoms {
        let kind = a
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("fact")
            .to_string();
        let content = a
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if content.is_empty() || content.chars().count() > ATOM_MAX_CHARS {
            continue;
        }
        let confidence = a
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.7)
            .clamp(0.0, 1.0) as f32;
        let refs = a
            .get("turn_refs")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|n| n.as_u64().map(|i| (i as usize).checked_sub(1)))
                    .flatten()
                    .filter_map(|i| turn_map.get(i).copied())
                    .map(|sid| json!({"session_id": sid}))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        // 议题二：相对时间已由 prompt（今天锚）让 LLM 换算成 ISO8601；这里宽容解析
        let occurred_at = a
            .get("occurred_at")
            .and_then(|v| v.as_str())
            .and_then(parse_iso);
        let valid_until = a
            .get("valid_until")
            .and_then(|v| v.as_str())
            .and_then(parse_iso);
        // 实体：该条记忆的主角（人/项目/主题/群组）——name+kind 归一后落库挂链
        let entities: Vec<(String, String)> = a
            .get("entities")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| {
                        let name = e.get("name")?.as_str()?.trim().to_string();
                        let kind = e.get("kind").and_then(|v| v.as_str()).unwrap_or("topic");
                        if name.is_empty()
                            || name.chars().count() > ENTITY_NAME_MAX_CHARS
                            || !ENTITY_KINDS.contains(&kind)
                        {
                            return None;
                        }
                        Some((name, kind.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        // 会话敏感继承：任一轮次来源会话标 sensitive → 产物 sensitive
        let sensitive = a
            .get("turn_refs")
            .and_then(|v| v.as_array())
            .map(|_| {
                refs.iter().any(|r| {
                    r.get("session_id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<Uuid>().ok())
                        .map(|sid| *session_sensitive.get(&sid).unwrap_or(&false))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        // 断言强度：LLM 判定 + 白名单；缺失/非法一律落 inference（保守——不升格）
        let strength = normalize_strength(a.get("strength").and_then(|v| v.as_str()));
        pending.push(PendingAtom {
            kind,
            content,
            confidence,
            refs: json!(refs),
            entities,
            occurred_at,
            valid_until,
            sensitive,
            strength,
        });
    }
    pending
}

/// 关系抽取：顶层 relations（from/to 用规范称呼，rel_type 限定五类）。
pub fn parse_relations(out: &Value) -> Vec<(String, String, String)> {
    let Some(rels) = out.get("relations").and_then(|r| r.as_array()) else {
        return Vec::new();
    };
    let mut parsed = Vec::new();
    for r in rels {
        let from = r
            .get("from")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string());
        let to = r
            .get("to")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string());
        let rel_type = r
            .get("rel_type")
            .and_then(|v| v.as_str())
            .unwrap_or("related_to");
        if let (Some(f), Some(t)) = (from, to)
            && !f.is_empty()
            && !t.is_empty()
            && f != t
            && REL_TYPES.contains(&rel_type)
        {
            parsed.push((f, t, rel_type.to_string()));
        }
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session(id: u8, turns: &[(&str, &str)]) -> SessionRow {
        SessionRow {
            id: Uuid::from_u128(id as u128),
            agent: "test".into(),
            content: json!(
                turns
                    .iter()
                    .map(|(sp, t)| json!({"speaker": sp, "text": t}))
                    .collect::<Vec<_>>()
            ),
            sensitive: false,
            metadata: json!({}),
        }
    }

    #[test]
    fn strength_defaults_to_inference() {
        assert_eq!(normalize_strength(None), "inference");
        assert_eq!(normalize_strength(Some("fact")), "fact");
        assert_eq!(normalize_strength(Some("assumption")), "assumption");
        assert_eq!(
            normalize_strength(Some("FACT")),
            "inference",
            "大小写敏感——防 LLM 随手大写逃逸白名单"
        );
        assert_eq!(
            normalize_strength(Some("certain")),
            "inference",
            "词表外一律保守"
        );
    }

    #[test]
    fn parse_iso_accepts_rfc3339_and_date_only() {
        assert!(parse_iso("2026-09-02T10:00:00Z").is_some());
        let d = parse_iso("2026-09-02").expect("date-only 应可解析");
        assert_eq!(d.to_rfc3339(), "2026-09-02T00:00:00+00:00");
        assert!(parse_iso("上周三").is_none(), "花式时间应静默丢弃");
    }

    #[test]
    fn sections_split_by_budget_and_headers_never_orphan() {
        // 每段预算 6000 字符；造两段超长会话，验证头不落单
        let long = "x".repeat(SEGMENT_CHARS);
        let sessions = vec![session(1, &[("user", &long), ("assistant", &long)])];
        let (segments, turn_map) = build_segments(&sessions);
        assert!(segments.len() >= 2, "超预算应分成多段：{}", segments.len());
        assert_eq!(turn_map.len(), 2, "轮次编号应逐轮映射");
        for seg in &segments {
            assert!(
                !seg.first().map(|l| l.is_header).unwrap_or(false)
                    || seg.len() == 1
                    || !seg.last().map(|l| l.is_header).unwrap_or(false),
                "会话头不应成为段尾孤行"
            );
        }
    }

    #[test]
    fn parse_atoms_filters_and_maps_turn_refs() {
        let turn_map = vec![Uuid::from_u128(11), Uuid::from_u128(22)];
        let mut sensitive = HashMap::new();
        sensitive.insert(Uuid::from_u128(22), true);
        let out = json!({"atoms": [
            {"kind": "preference", "content": " 喜欢 Rust ", "confidence": 1.7,
             "turn_refs": [1], "strength": "fact"},
            {"kind": "fact", "content": "", "turn_refs": [1]},                    // 空内容 → 丢
            {"kind": "fact", "content": "x".repeat(121), "turn_refs": [1]},       // 超长 → 丢
            {"kind": "fact", "content": "第二轮说的", "turn_refs": [2]},
        ]});
        let got = parse_atoms(&out, &turn_map, &sensitive);
        assert_eq!(got.len(), 2, "只应保留两条合法候选");
        assert_eq!(got[0].content, "喜欢 Rust", "内容应裁首尾空白");
        assert_eq!(got[0].confidence, 1.0, "confidence 应夹到 [0,1]");
        assert_eq!(got[0].strength, "fact");
        assert_eq!(got[0].refs, json!([{"session_id": Uuid::from_u128(11)}]));
        assert!(!got[0].sensitive, "会话 1 未标敏感");
        assert!(got[1].sensitive, "轮次来源会话标敏感 → 产物继承");
        assert_eq!(
            got[1].strength, "inference",
            "缺 strength 应保守落 inference"
        );
    }

    #[test]
    fn parse_atoms_filters_bad_entities() {
        let turn_map = vec![Uuid::from_u128(11)];
        let out = json!({"atoms": [{"content": "a", "turn_refs": [1], "entities": [
            {"name": "张三", "kind": "person"},
            {"name": "  ", "kind": "person"},          // 空名 → 丢
            {"name": "X", "kind": "不是种类"},          // 非白名单 → 丢
        ]}]});
        let got = parse_atoms(&out, &turn_map, &HashMap::new());
        assert_eq!(
            got[0].entities,
            vec![("张三".to_string(), "person".to_string())]
        );
    }

    #[test]
    fn parse_relations_filters_bad_rows() {
        let out = json!({"relations": [
            {"from": " 张三 ", "to": "长亭", "rel_type": "member_of"},
            {"from": "A", "to": "A", "rel_type": "member_of"},   // 自环 → 丢
            {"from": "", "to": "B", "rel_type": "member_of"},     // 空端点 → 丢
            {"from": "A", "to": "B", "rel_type": "乱写"},          // 非白名单 → 丢
        ]});
        let got = parse_relations(&out);
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0],
            (
                "张三".to_string(),
                "长亭".to_string(),
                "member_of".to_string()
            )
        );
    }
}

//! `project` 的实现切片（架构治理 2026-09-21：自 project.rs 纯搬移，零行为变化）。

use super::*;

/// 类型模板：type → 预设分类列表（0005：开发四分类 / 调研六分类）。
pub const PROJECT_TYPES: &[(&str, &[&str])] = &[
    ("dev", &["后端", "前端", "测试", "部署", "规划"]),
    (
        "research",
        &["待查", "线索", "资料", "结论", "疑点", "证伪"],
    ),
];

/// 类型模板项（Web 建项目时选择类型用）。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProjectTypeDto {
    pub r#type: String,
    pub label: String,
    pub default_categories: Vec<String>,
}

/// 轻归一化：lowercase + 空白折叠（保留标点——整词/行首判定的边界来源）。
pub(crate) fn light_normalize(s: &str) -> String {
    s.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// 整词命中判定（仅纯 ASCII 词——中文无词边界概念跳过）：
/// 归一化行中该词左右边界均非 ASCII 字母数字。
pub(crate) fn is_whole_word_hit(norm_line: &str, term: &str) -> bool {
    if !term.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return false;
    }
    match norm_line.find(term) {
        Some(pos) => {
            let bytes = norm_line.as_bytes();
            let before_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
            let after = pos + term.len();
            let after_ok = after >= bytes.len() || !bytes[after].is_ascii_alphanumeric();
            before_ok && after_ok
        }
        None => false,
    }
}

/// 检索归一化：lowercase + 空白/中英标点忽略（与 web Galaxy 页的重复实体归一化规则一致）——
/// 「WorkBuddy」与「Work Buddy」、「部署。」与「部署」互相可召回。
pub(crate) fn normalize_for_search(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| {
            !c.is_whitespace() && !"·.-_()（）【】《》，,、。：:；;！!？?\"'`~".contains(*c)
        })
        .collect()
}

/// 文档行检索命中（grep 式定位：行号 + 原文行；配合 read_doc_lines 区间精读）。
/// 0042 检索升级后按文档相关性聚合排序：score = 文档评分（title 加权 + 命中密度），
/// doc_hit_count = 该文档内命中行数——AI 可据此先读高分文档。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DocLineHitDto {
    pub doc_id: Uuid,
    pub title: String,
    pub category: String,
    /// 1-based 行号（基于文档当前版本）
    pub line: i64,
    pub text: String,
    /// 文档相关性评分（0042）
    #[serde(default)]
    pub score: i64,
    /// 该文档内命中行数（0042）
    #[serde(default)]
    pub doc_hit_count: i64,
}

/// 项目详情（本体 + 位置 + 文档），Web 详情页左树右内容用。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProjectDetailDto {
    pub id: Uuid,
    pub name: String,
    pub r#type: String,
    pub status: String,
    pub description: Option<String>,
    pub categories: Vec<String>,
    #[schema(value_type = Object)]
    pub frontmatter: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub locations: Vec<ProjectLocationDto>,
    pub docs: Vec<ProjectDocDto>,
}

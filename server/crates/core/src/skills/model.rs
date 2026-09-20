//! `skills` 的实现切片（架构治理 2026-09-21：自 skills.rs 纯搬移，零行为变化）。

use super::*;

/// local_path 上限（本地文件系统路径的合理长度）。
pub const LOCAL_PATH_MAX: usize = 1024;

/// 解析 tags 值：`a, b` / `[a, b]` / `["a","b"]` 三种形态都收。
pub(crate) fn parse_tags_value(raw: &str) -> Vec<String> {
    let s = raw.trim();
    let s = s
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .unwrap_or(s);
    s.split(',')
        .map(|t| t.trim().trim_matches('"').trim_matches('\'').trim())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// 把解析出的键值写入 meta（未知键忽略）。
pub(crate) fn assign_meta(meta: &mut FrontmatterMeta, key: &str, value: &str) {
    match key {
        "name" if !value.is_empty() => meta.name = Some(value.to_string()),
        "description" if !value.is_empty() => meta.description = Some(value.to_string()),
        "slug" if !value.is_empty() => meta.slug = Some(value.to_string()),
        "tags" => meta.tags = parse_tags_value(value),
        _ => {}
    }
}

/// 从 markdown 全文剥离 YAML 风格 frontmatter（`---` 围栏）。
/// 容错优先：无围栏 / 围栏未闭合 / 非 name/description/slug/tags 键都按「没有该字段」处理，
/// 绝不让导入因格式瑕疵整体失败。支持 YAML 块标量（`description: >-` 折叠 / `|` 保留多行——
/// 现网 SKILL.md 常见形态），折叠 `>` 把多行并成一行，保留 `|` 以换行拼接。
pub fn parse_frontmatter(content: &str) -> (FrontmatterMeta, String) {
    let mut meta = FrontmatterMeta::default();
    let trimmed = content.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return (meta, content.to_string());
    };
    // 第一行必须是围栏开头（--- 单独成行）
    if !rest.starts_with('\n') && !rest.starts_with("\r\n") {
        return (meta, content.to_string());
    }
    let body_start = rest.trim_start_matches("\r\n").trim_start_matches('\n');
    let Some(close_rel) = body_start.find("\n---") else {
        // 围栏未闭合：整段按正文处理
        return (meta, content.to_string());
    };
    let fm_block = &body_start[..close_rel];
    // 闭合行之后的内容是正文（跳过闭合行的行尾 + 全部前导空行——不残留进正文）
    let after = &body_start[close_rel + 4..];
    let body = after.trim_start_matches(['\n', '\r']);

    let meta = parse_frontmatter_lines(fm_block);
    (meta, body.to_string())
}

/// 名字 → slug（kebab-case）：小写、空白/下划线转 -、只留 [a-z0-9-]。
/// 非 ASCII 名字（中文等）slugify 后为空 → 返回 None，由调用方兜底。
pub fn slugify(name: &str) -> Option<String> {
    let mut out = String::new();
    let mut prev_dash = true; // 首尾不留 -
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if (ch.is_whitespace() || ch == '_' || ch == '-' || ch == '.')
            && !prev_dash
            && !out.is_empty()
        {
            out.push('-');
            prev_dash = true;
        }
        // 其他字符（含中文）丢弃
    }
    while out.ends_with('-') {
        out.pop();
    }
    let out = if out.len() > 80 {
        out[..80].to_string()
    } else {
        out
    };
    (!out.is_empty()).then_some(out)
}

/// slug 合法性：`^[a-z0-9][a-z0-9-]{0,79}$`（kebab-case，不含斜杠等路径字符）。
pub fn valid_slug(slug: &str) -> bool {
    let mut chars = slug.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    slug.len() <= 80 && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// 批量导入的单条结果（逐条成败互不阻断）。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SkillImportItem {
    /// 来源文档序号（从 0 起）或 filename
    pub index: usize,
    /// 成功时返回 slug
    pub slug: Option<String>,
    /// imported=新建 / updated=覆盖已有 / failed=该条失败
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SkillImportReport {
    pub imported: usize,
    pub updated: usize,
    pub failed: usize,
    pub items: Vec<SkillImportItem>,
}

/// 单文件内容上限（256 KiB 字符——脚本/参考资料的合理量级）。
pub const SKILL_FILE_MAX_CHARS: usize = 262_144;

/// 导出条目里的附属文件（含内容）。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SkillFileEntryDto {
    pub path: String,
    pub content: String,
}

/// 导出条目：技能本体 + 附属文件（folder 形态完整带走）。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SkillExportDto {
    #[serde(flatten)]
    pub skill: SkillDto,
    pub files: Vec<SkillFileEntryDto>,
}

/// 把技能还原成 SKILL.md 文本（frontmatter + 正文）——bundle 整包导出用。
/// 与 parse_frontmatter 同一契约：import → export 往返不丢 name/description/slug/tags。
pub fn render_skill_md(s: &SkillDto) -> String {
    let mut out = String::from(
        "---
",
    );
    out.push_str(&format!(
        "name: {}
",
        s.name
    ));
    if !s.description.is_empty() {
        out.push_str(&format!(
            "description: {}
",
            s.description
        ));
    }
    out.push_str(&format!(
        "slug: {}
",
        s.slug
    ));
    if !s.tags.is_empty() {
        out.push_str(&format!(
            "tags: {}
",
            s.tags.join(", ")
        ));
    }
    out.push_str(
        "---

",
    );
    out.push_str(&s.content);
    out
}

/// 新建参数束（create_skill 入参 > 7 个会触发 clippy::too_many_arguments，收拢成结构体）。
/// kind="script" 时 local_path 必填、content 须为空、不产 create 快照；
/// origin 含 github 时可带 repo_url；kind="text" 时 local_path 必须为 None。
#[derive(Debug, Clone, Copy)]
pub struct NewSkill<'a> {
    pub slug: Option<&'a str>,
    pub name: &'a str,
    pub description: &'a str,
    pub content: &'a str,
    pub tags: &'a [String],
    pub enabled: bool,
    pub source: &'a str,
    pub kind: &'a str,
    pub origin: &'a str,
    pub local_path: Option<&'a str>,
    pub repo_url: Option<&'a str>,
}

/// 新建/更新的可选语义字段（None = 不动）。
/// origin/repo_url/local_path 为终值语义（服务层与现状合并计算，见 update_skill）。
#[derive(Debug, Default, Clone)]
pub struct SkillPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub enabled: Option<bool>,
    pub origin: Option<String>,
    pub repo_url: Option<String>,
    pub local_path: Option<String>,
}

/// 二态字段规范化结果（validate_two_kind 返回束）。
pub(crate) struct NormalizedKind {
    pub(crate) kind: String,
    pub(crate) origin: String,
    pub(crate) local_path: Option<String>,
    pub(crate) repo_url: Option<String>,
    /// script 型不产 create 快照（正文不在库中，无内容可快照）
    pub(crate) snapshot: bool,
}

/// 解析 frontmatter 块（YAML 子集：标量 / 块标量 / `tags:` 块列表），产出元数据。
fn parse_frontmatter_lines(fm_block: &str) -> FrontmatterMeta {
    let mut meta = FrontmatterMeta::default();
    let lines: Vec<&str> = fm_block.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        // YAML 块列表（D7）：`tags:` 值为空时收集后续 "- item" 行
        if let Some((consumed, items)) = parse_tags_list(&lines, i, &key, value) {
            i += consumed;
            meta.tags = items;
            continue;
        }
        if let Some((consumed, joined)) = parse_block_scalar(value, &lines, i) {
            i += consumed;
            assign_meta(&mut meta, &key, &joined);
        } else {
            assign_meta(&mut meta, &key, value);
        }
    }
    meta
}

/// YAML 块列表（D7）：`tags:` 值为空时收集后续 "- item" 行。返回 (消费行数, 列表)；
/// 不满足块列表形态返回 None。
fn parse_tags_list(
    lines: &[&str],
    i: usize,
    key: &str,
    value: &str,
) -> Option<(usize, Vec<String>)> {
    if key != "tags"
        || !value.is_empty()
        || i >= lines.len()
        || !lines[i].trim_start().starts_with("- ")
    {
        return None;
    }
    let mut j = i;
    let mut items: Vec<String> = Vec::new();
    while j < lines.len() {
        let l = lines[j].trim_start();
        match l.strip_prefix("- ") {
            Some(item) => {
                items.push(item.trim().trim_matches('"').trim_matches('\'').to_string());
                j += 1;
            }
            None => break,
        }
    }
    Some((j - i, items))
}

/// 块标量（`>` 折叠 / `|` 保留）：收集缩进行（空行不断块），去缩进 + 去尾部空行。
/// 返回 (消费行数, 值)；非块标量返回 None。
fn parse_block_scalar(value: &str, lines: &[&str], i: usize) -> Option<(usize, String)> {
    if !matches!(value, ">" | ">>" | ">-" | ">+" | "|" | "|-" | "|+") || i >= lines.len() {
        return None;
    }
    let mut j = i;
    let mut buf: Vec<&str> = Vec::new();
    let mut indent: Option<usize> = None;
    while j < lines.len() {
        let l = lines[j];
        if l.trim().is_empty() {
            buf.push(l);
            j += 1;
            continue;
        }
        let ind = l.len() - l.trim_start().len();
        if ind == 0 {
            break;
        }
        match indent {
            None => indent = Some(ind),
            Some(n) if ind < n => break,
            _ => {}
        }
        buf.push(l);
        j += 1;
    }
    let min_indent = indent.unwrap_or(2);
    let mut stripped: Vec<String> = buf
        .iter()
        .map(|l| l.get(min_indent..).unwrap_or(l.trim_start()).to_string())
        .collect();
    while stripped.last().is_some_and(|l| l.trim().is_empty()) {
        stripped.pop();
    }
    let joined = if value.starts_with('>') {
        stripped
            .iter()
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        stripped.join("\n")
    };
    Some((j - i, joined))
}

//! 技能域服务（第六域）：AI 技能（SKILL.md 形态）的资产化管理。
//!
//! 技能 = slug 唯一 + frontmatter（name/description/tags）+ markdown 正文的可复用指令包。
//! 语义字段每次变更前留版本快照（skill_revisions，保留最近 50 版），可回滚。
//! 批量导入直接吃 SKILL.md 全文（frontmatter 容错解析），迁移现有技能库零改写。
//!
//! 二态存储（0038）：text = 整体入库（无脚本，或依赖走 npm/cargo 全局二进制——全是文本）；
//! script = 真身只存本地文件夹（SKILL.md + scripts/ 等），库中只存指针（local_path）+ 来源
//! （origin: self/github/both，github 侧可记 repo_url）——content 不入库（get 现读、
//! 指针失效明确报错）、file_*/versions/restore 一律拒绝并指引本地操作、不产 revisions
//! 快照（版本归本地 git 管）。
//!
//! 持久化在 `engram_storage::repo::skills`（本文件只保留校验、冲突语义与编排；
//! 快照+变更的事务整体落在 repo 的 `*_tx` 函数内）。

use engram_storage::StoreError;
use engram_storage::repo::skills as repo;
use serde::Serialize;
use uuid::Uuid;

/// 技能域错误（api 层转 ApiError）。
#[derive(Debug, thiserror::Error)]
pub enum SkillsError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("存储暂时不可用: {0}")]
    Storage(String),
}

impl From<StoreError> for SkillsError {
    fn from(e: StoreError) -> Self {
        SkillsError::Storage(e.to_string())
    }
}

/// 版本快照保留上限（防膨胀；更老的自动淘汰）。
pub use engram_storage::repo::skills::MAX_REVISIONS;

// ---------- 二态存储（0038）：text=入库 / script=本地指针 ----------

/// 合法存储形态。
pub const KINDS: &[&str] = &["text", "script"];

/// 合法来源：self=自建未发布 / github=源自 GitHub / both=自建且已发布。
pub const ORIGINS: &[&str] = &["self", "github", "both"];

/// 「真脚本」后缀清单：text 型技能的附属文件命中即拒绝（这类技能应整体走本地 + 指针）。
/// npm/cargo 全局二进制依赖只出现在 SKILL.md 说明文字里，不在附属文件，不受影响。
pub const SCRIPT_EXTS: &[&str] = &[
    "py", "sh", "bash", "zsh", "fish", "rb", "pl", "lua", "ps1", "bat", "cmd", "js", "mjs", "cjs",
    "ts",
];

/// 附属文件路径是否是「真脚本」（按后缀判定，大小写不敏感）。
pub fn is_script_path(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .map(|ext| SCRIPT_EXTS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
        .unwrap_or(false)
}

/// local_path 上限（本地文件系统路径的合理长度）。
pub const LOCAL_PATH_MAX: usize = 1024;

// ---------- frontmatter 容错解析 ----------

/// frontmatter 解析结果（全部可选——没有 frontmatter 也能导入，名字由调用方兜底）。
#[derive(Debug, Default, Clone, PartialEq)]
pub struct FrontmatterMeta {
    pub name: Option<String>,
    pub description: Option<String>,
    pub slug: Option<String>,
    pub tags: Vec<String>,
}

/// 解析 tags 值：`a, b` / `[a, b]` / `["a","b"]` 三种形态都收。
fn parse_tags_value(raw: &str) -> Vec<String> {
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
fn assign_meta(meta: &mut FrontmatterMeta, key: &str, value: &str) {
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
        if key == "tags"
            && value.is_empty()
            && i < lines.len()
            && lines[i].trim_start().starts_with("- ")
        {
            let mut items: Vec<String> = Vec::new();
            while i < lines.len() {
                let l = lines[i].trim_start();
                match l.strip_prefix("- ") {
                    Some(item) => {
                        items.push(item.trim().trim_matches('"').trim_matches('\'').to_string());
                        i += 1;
                    }
                    None => break,
                }
            }
            meta.tags = items;
            continue;
        }
        if matches!(value, ">" | ">>" | ">-" | ">+" | "|" | "|-" | "|+") && i < lines.len() {
            // 块标量：收集缩进行（空行不断块，去缩进后折叠或保留）
            let mut buf: Vec<&str> = Vec::new();
            let mut indent: Option<usize> = None;
            while i < lines.len() {
                let l = lines[i];
                if l.trim().is_empty() {
                    buf.push(l);
                    i += 1;
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
                i += 1;
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
            assign_meta(&mut meta, &key, &joined);
        } else {
            assign_meta(&mut meta, &key, value);
        }
    }
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

// ---------- DTO（持久化模型在 storage，此处 re-export 保持路径兼容） ----------

pub use engram_storage::models::skills::{SkillDto, SkillRevisionDto, SkillSummaryDto};

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

// ---------- 附属文件（folder 形态） ----------

/// 单文件内容上限（256 KiB 字符——脚本/参考资料的合理量级）。
pub const SKILL_FILE_MAX_CHARS: usize = 262_144;

/// 单技能附属文件数上限。
pub const SKILL_FILES_MAX: usize = 64;

/// 附属文件索引条目（不含内容）。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SkillFileInfoDto {
    /// 相对路径（/ 分隔，如 scripts/run.py）
    pub path: String,
    /// 内容字节数
    pub size: i64,
}

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

// ---------- Service ----------

pub struct SkillsService {
    pool: engram_storage::PgPool,
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
struct NormalizedKind {
    kind: String,
    origin: String,
    local_path: Option<String>,
    repo_url: Option<String>,
    /// script 型不产 create 快照（正文不在库中，无内容可快照）
    snapshot: bool,
}

impl SkillsService {
    pub fn new(pool: engram_storage::PgPool) -> Self {
        Self { pool }
    }

    fn validate_name(name: &str) -> Result<(), SkillsError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(SkillsError::BadRequest(
                "技能名不能为空——给技能一个能认出来的名字".into(),
            ));
        }
        if name.len() > 200 {
            return Err(SkillsError::BadRequest("技能名过长（>200 字符）".into()));
        }
        Ok(())
    }
    /// 二态字段校验（create 用）。
    /// 规则：script 型 local_path 必填且 content 须为空；text 型不接受 local_path；
    /// origin=self 不接受 repo_url。script 型不产 create 快照。
    fn validate_two_kind(
        kind: &str,
        origin: &str,
        content: &str,
        local_path: Option<&str>,
        repo_url: Option<&str>,
    ) -> Result<NormalizedKind, SkillsError> {
        let kind = kind.trim();
        if !KINDS.contains(&kind) {
            return Err(SkillsError::BadRequest(format!(
                "kind {kind:?} 不合法——只支持 text（纯文本入库）/ script（脚本存本地、库中存指针）"
            )));
        }
        let origin = origin.trim();
        if !ORIGINS.contains(&origin) {
            return Err(SkillsError::BadRequest(format!(
                "origin {origin:?} 不合法——只支持 self（自建）/ github（源自 GitHub）/ both（自建且已发布）"
            )));
        }
        let local_path = local_path.map(str::trim).filter(|p| !p.is_empty());
        let repo_url = repo_url.map(str::trim).filter(|u| !u.is_empty());
        if kind == "script" {
            let Some(p) = local_path else {
                return Err(SkillsError::BadRequest(
                    "script 型技能必须给 local_path（本地技能文件夹路径，含 SKILL.md）——系统只存指针，正文不入库"
                        .into(),
                ));
            };
            if p.len() > LOCAL_PATH_MAX {
                return Err(SkillsError::BadRequest(format!(
                    "local_path 过长（>{LOCAL_PATH_MAX} 字符）"
                )));
            }
            if !content.trim().is_empty() {
                return Err(SkillsError::BadRequest(
                    "script 型技能正文不入库——content 须为空；SKILL.md 真身放 local_path 下，get 时由系统现读"
                        .into(),
                ));
            }
        } else if local_path.is_some() {
            return Err(SkillsError::BadRequest(
                "text 型技能整体入库，不接受 local_path——脚本存本地的请用 kind=script".into(),
            ));
        }
        if origin == "self" && repo_url.is_some() {
            return Err(SkillsError::BadRequest(
                "origin=self 不接受 repo_url——带仓库地址请用 origin=github 或 both".into(),
            ));
        }
        Ok(NormalizedKind {
            kind: kind.to_string(),
            origin: origin.to_string(),
            local_path: local_path.map(str::to_string),
            repo_url: repo_url.map(str::to_string),
            snapshot: kind != "script",
        })
    }

    fn resolve_slug(slug: Option<&str>, name: &str) -> Result<String, SkillsError> {
        match slug.map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => {
                if valid_slug(s) {
                    Ok(s.to_string())
                } else {
                    Err(SkillsError::BadRequest(format!(
                        "slug {s:?} 不合法——期望 kebab-case（小写字母/数字/-，≤80 字符，如 review-pr）"
                    )))
                }
            }
            None => {
                // D25：名字含非 ASCII（中文等）时 slugify 只保留 ASCII 部分——
                // "PR 审查" 会坍缩成 "pr"，任何同前缀中文名都挤到同一 slug。响亮拒绝强制显式。
                if !name.is_ascii() {
                    return Err(SkillsError::BadRequest(format!(
                        "技能名「{name}」含非 ASCII 字符——slug 推导会丢弃这些字符造成撞名，请显式传 slug（kebab-case，如 code-review）"
                    )));
                }
                slugify(name).ok_or_else(|| {
                    SkillsError::BadRequest(format!(
                        "无法从技能名「{name}」推导 slug（非 ASCII 名字请显式传 slug，如 code-review）"
                    ))
                })
            }
        }
    }

    pub async fn create_skill(&self, s: NewSkill<'_>) -> Result<SkillDto, SkillsError> {
        Self::validate_name(s.name)?;
        let name = s.name.trim();
        let slug = Self::resolve_slug(s.slug, s.name)?;
        let nk = Self::validate_two_kind(s.kind, s.origin, s.content, s.local_path, s.repo_url)?;
        let id = Uuid::now_v7();
        let inserted = repo::create_skill_tx(
            &self.pool,
            repo::NewSkillRow {
                id,
                slug: &slug,
                name,
                description: s.description.trim(),
                content: s.content,
                tags: s.tags,
                enabled: s.enabled,
                source: s.source,
                kind: &nk.kind,
                origin: &nk.origin,
                local_path: nk.local_path.as_deref(),
                repo_url: nk.repo_url.as_deref(),
                snapshot: nk.snapshot,
            },
        )
        .await?;
        if inserted == 0 {
            return Err(SkillsError::Conflict(format!(
                "slug「{slug}」已被占用——slug 唯一，请换 slug 或直接更新已有技能（未显式传 slug 时它由 name 推导，中文名会坍缩到 ASCII 前缀，撞车时显式传 slug 即可避开）"
            )));
        }
        self.get_skill(&slug).await
    }

    /// 列表（摘要，不含正文）：q 搜 name/description，tag 过滤，enabled 过滤。
    pub async fn list_skills(
        &self,
        q: Option<&str>,
        tag: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<Vec<SkillSummaryDto>, SkillsError> {
        // 三条件常驻 + 显式类型：避免条件拼接造成参数序号空洞（PG 推不出未引用参数的类型）
        let pattern = q.map(|q| format!("%{}%", q.trim()));
        let tag_vec = tag.map(|t| vec![t.to_string()]);
        Ok(repo::list_skills(&self.pool, pattern, tag_vec, enabled).await?)
    }

    pub async fn get_skill(&self, slug: &str) -> Result<SkillDto, SkillsError> {
        // 双寻址（R 报告 P1-11）：slug 优先，未中按 name 精确兜底
        let row = repo::get_skill(&self.pool, slug)
            .await?
            .or(repo::get_skill_by_name(&self.pool, slug).await?);
        row.ok_or_else(|| {
            SkillsError::NotFound(format!(
                "技能 {slug:?} 不存在——先 skills_list 确认 slug（可能已删除或抄错）"
            ))
        })
    }

    /// 详情读取（含正文）：script 型从 local_path/SKILL.md 现读组装（指针语义——库中不存正文）。
    /// 指针失效（目录缺失 / SKILL.md 不可读）报 NotFound 带本地路径指引。
    /// text 型与 get_skill 等价。
    pub async fn get_skill_with_content(&self, slug: &str) -> Result<SkillDto, SkillsError> {
        let mut s = self.get_skill(slug).await?;
        if s.kind == "script" {
            let path = s.local_path.clone().unwrap_or_default();
            let md = std::fs::read_to_string(std::path::Path::new(&path).join("SKILL.md"))
                .map_err(|e| {
                    SkillsError::NotFound(format!(
                        "指针失效：无法读取 {path}/SKILL.md（{e}）——确认本地技能文件夹就位，或更新 local_path"
                    ))
                })?;
            s.content = md;
        }
        Ok(s)
    }

    /// script 型技能的文件/版本操作统一拒绝（真身在本地，系统只存指针）。
    fn reject_script_ops(s: &SkillDto, op: &str) -> Result<(), SkillsError> {
        if s.kind == "script" {
            return Err(SkillsError::BadRequest(format!(
                "{op} 对 script 型技能不可用——文件真身在本地 {}，请直接操作本地文件夹（系统只存指针）",
                s.local_path.as_deref().unwrap_or("(local_path 缺失)")
            )));
        }
        Ok(())
    }

    /// slug/name → 真实 slug（更新/删除/文件操作寻址用；slug 优先，name 精确兜底）。
    async fn resolve(&self, slug_or_name: &str) -> Result<String, SkillsError> {
        Ok(self.get_skill(slug_or_name).await?.slug)
    }

    /// 语义字段更新：text 型先快照现状（origin=update）再落变更；enabled-only 不留版本。
    /// script 型：正文不入库（patch content 拒绝）、任何变更都不留快照（版本归本地 git 管）；
    /// origin/repo_url/local_path 按终值语义合并后全量写（origin=self 时 repo_url 联动清空）。
    pub async fn update_skill(
        &self,
        slug: &str,
        patch: SkillPatch,
    ) -> Result<SkillDto, SkillsError> {
        let slug = self.resolve(slug).await?;
        let current = self.get_skill(&slug).await?;
        if current.kind == "script" && patch.content.is_some() {
            return Err(SkillsError::BadRequest(format!(
                "script 型技能正文不入库——改 SKILL.md 请直接编辑本地 {}（系统只存指针）",
                current.local_path.as_deref().unwrap_or("(local_path 缺失)")
            )));
        }
        let origin = patch
            .origin
            .clone()
            .unwrap_or_else(|| current.origin.clone());
        if !ORIGINS.contains(&origin.as_str()) {
            return Err(SkillsError::BadRequest(format!(
                "origin {origin:?} 不合法——只支持 self（自建）/ github（源自 GitHub）/ both（自建且已发布）"
            )));
        }
        let local_path: Option<String> = if current.kind == "script" {
            match patch
                .local_path
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty())
            {
                Some(p) => {
                    if p.len() > LOCAL_PATH_MAX {
                        return Err(SkillsError::BadRequest(format!(
                            "local_path 过长（>{LOCAL_PATH_MAX} 字符）"
                        )));
                    }
                    Some(p.to_string())
                }
                None => current.local_path.clone(),
            }
        } else {
            if patch.local_path.is_some() {
                return Err(SkillsError::BadRequest(
                    "text 型技能整体入库，不接受 local_path——脚本存本地的请用 kind=script".into(),
                ));
            }
            None
        };
        let repo_url: Option<String> = if origin == "self" {
            None // 改回自建 → 仓库地址联动清空
        } else {
            patch
                .repo_url
                .clone()
                .map(|u| u.trim().to_string())
                .filter(|u| !u.is_empty())
                .or(current.repo_url.clone())
        };
        let semantic_change = patch.name.is_some()
            || patch.description.is_some()
            || patch.content.is_some()
            || patch.tags.is_some();
        let is_script = current.kind == "script";
        let snapshot = if semantic_change && !is_script {
            Some(repo::SkillSnapshot {
                skill_id: current.id,
                name: &current.name,
                description: &current.description,
                content: &current.content,
                tags: &current.tags,
                origin: "update",
            })
        } else {
            None
        };
        let patch_data = repo::SkillPatchData {
            name: patch.name.as_deref().map(str::trim),
            description: patch.description.as_deref().map(str::trim),
            content: patch.content.as_deref(),
            tags: &patch.tags,
            enabled: patch.enabled,
            origin: &origin,
            repo_url: repo_url.as_deref(),
            local_path: local_path.as_deref(),
        };
        let updated =
            repo::update_skill_tx(&self.pool, current.id, snapshot.as_ref(), &patch_data).await?;
        if updated == 0 {
            return Err(SkillsError::NotFound(format!(
                "技能 {slug:?} 不存在——先 skills_list 确认 slug（可能已删除或抄错）"
            )));
        }
        self.get_skill(&slug).await
    }

    pub async fn delete_skill(&self, slug: &str) -> Result<bool, SkillsError> {
        let slug = self.resolve(slug).await?;
        let deleted = repo::delete_skill(&self.pool, &slug).await?;
        if deleted == 0 {
            return Err(SkillsError::NotFound(format!(
                "技能 {slug:?} 不存在——先 skills_list 确认 slug（可能已删除或抄错）"
            )));
        }
        Ok(true)
    }

    /// 批量导入 SKILL.md 全文（frontmatter 容错解析，逐条成败互不阻断）。
    /// filename 兜底命名（去 .md 后做名字/ slug 来源）；overwrite=true 时命中已有 slug 走更新。
    pub async fn import_skills(
        &self,
        documents: &[(Option<String>, String, Vec<String>)],
        overwrite: bool,
        source: &str,
    ) -> Result<SkillImportReport, SkillsError> {
        let mut report = SkillImportReport {
            imported: 0,
            updated: 0,
            failed: 0,
            items: Vec::new(),
        };
        for (i, (filename, raw, extra_tags)) in documents.iter().enumerate() {
            let item = self
                .import_one(i, filename.as_deref(), raw, extra_tags, overwrite, source)
                .await;
            match &item.status {
                s if s == "imported" => report.imported += 1,
                s if s == "updated" => report.updated += 1,
                _ => report.failed += 1,
            }
            report.items.push(item);
        }
        Ok(report)
    }

    async fn import_one(
        &self,
        index: usize,
        filename: Option<&str>,
        raw: &str,
        extra_tags: &[String],
        overwrite: bool,
        source: &str,
    ) -> SkillImportItem {
        let fallback_name = filename.map(|f| {
            f.trim_end_matches(".md")
                .trim_end_matches(".markdown")
                .to_string()
        });
        let (meta, body) = parse_frontmatter(raw);
        let name = meta.name.or(fallback_name).unwrap_or_default();
        let make_err = |e: SkillsError| SkillImportItem {
            index,
            slug: None,
            status: "failed".into(),
            error: Some(e.to_string()),
        };
        let name = match Self::validate_name(&name) {
            Ok(()) => name.trim().to_string(),
            Err(e) => return make_err(e),
        };
        let slug = match Self::resolve_slug(meta.slug.as_deref(), &name) {
            Ok(s) => s,
            Err(e) => return make_err(e),
        };
        // frontmatter tags + 导入方附带 tags（去重保序）
        let mut tags = meta.tags.clone();
        for t in extra_tags {
            if !tags.contains(t) {
                tags.push(t.clone());
            }
        }
        let exists = repo::exists_slug(&self.pool, &slug).await;
        match exists {
            Err(e) => make_err(e.into()),
            Ok(Some(_)) => {
                if !overwrite {
                    return SkillImportItem {
                        index,
                        slug: Some(slug.clone()),
                        status: "failed".into(),
                        error: Some(format!(
                            "slug「{slug}」已存在（overwrite=false）——确认要覆盖就带 overwrite 重导"
                        )),
                    };
                }
                match self
                    .update_skill(
                        &slug,
                        SkillPatch {
                            name: Some(name),
                            description: Some(meta.description.unwrap_or_default()),
                            content: Some(body),
                            tags: Some(tags),
                            enabled: None,
                            origin: None,
                            repo_url: None,
                            local_path: None,
                        },
                    )
                    .await
                {
                    Ok(_) => SkillImportItem {
                        index,
                        slug: Some(slug),
                        status: "updated".into(),
                        error: None,
                    },
                    Err(e) => make_err(e),
                }
            }
            Ok(None) => match self
                .create_skill(NewSkill {
                    slug: Some(&slug),
                    name: &name,
                    description: &meta.description.unwrap_or_default(),
                    content: &body,
                    tags: &tags,
                    enabled: true,
                    source,
                    kind: "text",
                    origin: "self",
                    local_path: None,
                    repo_url: None,
                })
                .await
            {
                Ok(_) => SkillImportItem {
                    index,
                    slug: Some(slug),
                    status: "imported".into(),
                    error: None,
                },
                Err(e) => make_err(e),
            },
        }
    }

    /// 单技能导出（bundle 整包用）：本体 + 全部附属文件。
    /// 纯库读（不走 get_skill_with_content）：script 型导出指针元数据（content 恒空、无附属文件），
    /// 不把本地正文卷进导出——导出的是「库」，不是本地文件系统。
    pub async fn export_one(&self, slug: &str) -> Result<SkillExportDto, SkillsError> {
        let skill = repo::get_skill(&self.pool, slug)
            .await?
            .or(repo::get_skill_by_name(&self.pool, slug).await?)
            .ok_or_else(|| {
                SkillsError::NotFound(format!(
                    "技能 {slug:?} 不存在——先 skills_list 确认 slug（可能已删除或抄错）"
                ))
            })?;
        let files = if skill.kind == "script" {
            Vec::new()
        } else {
            repo::skill_file_contents(&self.pool, skill.id)
                .await?
                .into_iter()
                .map(|(path, content)| SkillFileEntryDto { path, content })
                .collect()
        };
        Ok(SkillExportDto { skill, files })
    }

    /// 全量导出（含附属文件——folder 形态整体带走，数据主权）。
    /// script 型天然只带走元数据（content 恒空、skill_files 无行）——指针与来源随行。
    pub async fn export_skills(&self) -> Result<Vec<SkillExportDto>, SkillsError> {
        let mut files_by_skill: std::collections::HashMap<Uuid, Vec<SkillFileEntryDto>> =
            std::collections::HashMap::new();
        for (sid, path, content) in repo::skill_files_all(&self.pool).await? {
            files_by_skill
                .entry(sid)
                .or_default()
                .push(SkillFileEntryDto { path, content });
        }
        Ok(repo::export_skills(&self.pool)
            .await?
            .into_iter()
            .map(|skill| SkillExportDto {
                files: files_by_skill.remove(&skill.id).unwrap_or_default(),
                skill,
            })
            .collect())
    }

    // ---------- 附属文件（folder 形态：scripts/ / references/ / assets/…） ----------
    //
    // skill = 文件夹：SKILL.md 本体在 content；附属文件按相对路径寻址。
    // 云部署语义：文件是「内容」不是「文件系统位置」——MCP 按路径下发，
    // AI 客户端取走后本地执行；服务端永不执行任何上传代码。
    //
    // 三种消费形态（按需取用，不一股脑拉全量）：
    // ① 纯文本 → skills_get 直接读（不落盘）；② 只要一个文件 →
    // GET /skills/{slug}/file?path=…&raw=1 单文件直下；③ 整个文件夹 →
    // GET /skills/{slug}/bundle（zip 整包）。

    /// 附属文件索引（path + 字节大小，按 path 排序）。
    pub async fn list_files(&self, slug: &str) -> Result<Vec<SkillFileInfoDto>, SkillsError> {
        let s = self.get_skill(slug).await?;
        Self::reject_script_ops(&s, "文件列表")?;
        Ok(repo::list_skill_files(&self.pool, s.id)
            .await?
            .into_iter()
            .map(|(path, size)| SkillFileInfoDto { path, size })
            .collect())
    }

    /// 读一个附属文件全文。
    pub async fn get_file(&self, slug: &str, path: &str) -> Result<String, SkillsError> {
        Self::validate_file_path(path)?;
        let s = self.get_skill(slug).await?;
        Self::reject_script_ops(&s, "文件读取")?;
        repo::get_skill_file(&self.pool, s.id, path.trim())
            .await?
            .ok_or_else(|| {
                SkillsError::NotFound(format!(
                    "文件 {path:?} 不存在——先看文件索引确认路径（注意大小写）"
                ))
            })
    }

    /// 写（upsert）一个附属文件。返回 (path, size 字节)。
    pub async fn put_file(
        &self,
        slug: &str,
        path: &str,
        content: &str,
    ) -> Result<(String, i64), SkillsError> {
        Self::validate_file_path(path)?;
        let path = path.trim();
        if content.chars().count() > SKILL_FILE_MAX_CHARS {
            return Err(SkillsError::BadRequest(format!(
                "文件内容超长：上限 {} 字符",
                SKILL_FILE_MAX_CHARS
            )));
        }
        let s = self.get_skill(slug).await?;
        Self::reject_script_ops(&s, "文件写入")?;
        // 判型主规则：text 型的附属文件不允许是「真脚本」——这类技能应整体走 script 型
        if is_script_path(path) {
            return Err(SkillsError::BadRequest(format!(
                "text 型技能不允许附属脚本文件 {path:?}（命中脚本后缀清单）——这类技能请整体走 script 型：真身存本地文件夹（SKILL.md + scripts/），系统只存指针 local_path"
            )));
        }
        let existing = repo::list_skill_files(&self.pool, s.id).await?;
        if existing.len() >= SKILL_FILES_MAX && !existing.iter().any(|(p, _)| p == path) {
            return Err(SkillsError::BadRequest(format!(
                "附属文件数达上限（{} 个）——清理不用的文件后再加",
                SKILL_FILES_MAX
            )));
        }
        repo::put_skill_file(&self.pool, Uuid::now_v7(), s.id, path, content).await?;
        repo::touch_skill(&self.pool, s.id).await?;
        Ok((path.to_string(), content.len() as i64))
    }

    /// 删除一个附属文件。
    pub async fn delete_file(&self, slug: &str, path: &str) -> Result<(), SkillsError> {
        Self::validate_file_path(path)?;
        let s = self.get_skill(slug).await?;
        Self::reject_script_ops(&s, "文件删除")?;
        let n = repo::delete_skill_file(&self.pool, s.id, path.trim()).await?;
        if n == 0 {
            return Err(SkillsError::NotFound(format!("文件 {path:?} 不存在")));
        }
        repo::touch_skill(&self.pool, s.id).await?;
        Ok(())
    }

    /// 相对路径校验：/ 分隔、禁止 .. 与绝对路径、禁止反斜杠、禁改 SKILL.md 本体。
    fn validate_file_path(path: &str) -> Result<(), SkillsError> {
        let p = path.trim();
        if p.is_empty() {
            return Err(SkillsError::BadRequest("path 不能为空".into()));
        }
        if p.len() > 200 {
            return Err(SkillsError::BadRequest("path 过长（>200 字符）".into()));
        }
        if p.starts_with('/') || p.contains('\\') || p.contains(':') {
            return Err(SkillsError::BadRequest(
                "path 必须是相对路径且用 / 分隔（如 scripts/run.py、references/api.md）；\\
                 不允许盘符/冒号（Windows 备用数据流风险）"
                    .into(),
            ));
        }
        if p.split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
        {
            return Err(SkillsError::BadRequest(
                "path 含空段或 . / .. 段——用规范相对路径（如 scripts/run.py）".into(),
            ));
        }
        if p.eq_ignore_ascii_case("skill.md") {
            return Err(SkillsError::BadRequest(
                "SKILL.md 是技能本体（走 update 的 content），附属文件请用其他路径".into(),
            ));
        }
        Ok(())
    }

    pub async fn list_revisions(&self, slug: &str) -> Result<Vec<SkillRevisionDto>, SkillsError> {
        let skill = self.get_skill(slug).await?;
        Self::reject_script_ops(&skill, "版本列表")?;
        Ok(repo::list_revisions(&self.pool, skill.id).await?)
    }

    /// 回滚到某个版本：先快照现状（origin=restore），再把目标版本内容落回本体。
    pub async fn restore_revision(
        &self,
        slug: &str,
        revision_id: Uuid,
    ) -> Result<SkillDto, SkillsError> {
        let skill = self.get_skill(slug).await?;
        Self::reject_script_ops(&skill, "版本回滚")?;
        let rev = repo::get_revision(&self.pool, revision_id, skill.id)
            .await?
            .ok_or_else(|| {
                SkillsError::NotFound(format!(
                    "版本 {revision_id} 不存在——先查 {slug} 的版本列表取 id"
                ))
            })?;
        repo::restore_revision_tx(
            &self.pool,
            skill.id,
            &repo::SkillSnapshot {
                skill_id: skill.id,
                name: &skill.name,
                description: &skill.description,
                content: &skill.content,
                tags: &skill.tags,
                origin: "restore",
            },
            &rev,
        )
        .await?;
        self.get_skill(slug).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_full() {
        let (meta, body) = parse_frontmatter(
            "---\nname: Review PR\ndescription: 审查拉取请求\nslug: review-pr\ntags: rust, review\n---\n正文第一行",
        );
        assert_eq!(meta.name.as_deref(), Some("Review PR"));
        assert_eq!(meta.description.as_deref(), Some("审查拉取请求"));
        assert_eq!(meta.slug.as_deref(), Some("review-pr"));
        assert_eq!(meta.tags, vec!["rust", "review"]);
        assert_eq!(body, "正文第一行");
    }

    #[test]
    fn frontmatter_crlf() {
        let (meta, body) = parse_frontmatter("---\r\nname: X\r\ntags: a, b\r\n---\r\nbody");
        assert_eq!(meta.name.as_deref(), Some("X"));
        assert_eq!(meta.tags, vec!["a", "b"]);
        assert_eq!(body, "body");
    }

    /// 现网 SKILL.md 真实形态：折叠块标量（>-）多行 description。
    #[test]
    fn frontmatter_folded_block_scalar() {
        let raw = "---\nname: chaitin-products\ndescription: >-\n  LOAD WHEN: 需要了解长亭产品线。\n  长亭产品知识库——包含产品白皮书、用户手册等。\n\n  TRIGGERS: 长亭, 万象, 雷池\n---\n\n# 长亭产品知识库\n\n## 用途\n";
        let (meta, body) = parse_frontmatter(raw);
        assert_eq!(meta.name.as_deref(), Some("chaitin-products"));
        let desc = meta.description.unwrap();
        assert!(desc.starts_with("LOAD WHEN: 需要了解长亭产品线。"));
        assert!(desc.contains("长亭产品知识库"));
        assert!(
            desc.ends_with("TRIGGERS: 长亭, 万象, 雷池"),
            "折叠标量多行并入一行：{desc}"
        );
        assert!(body.trim_start().starts_with("# 长亭产品知识库"));
    }

    /// 保留块标量（|）：换行保留。
    #[test]
    fn frontmatter_literal_block_scalar() {
        let (meta, _) =
            parse_frontmatter("---\nname: X\ndescription: |\n  第一行\n  第二行\n---\nbody");
        assert_eq!(meta.description.as_deref(), Some("第一行\n第二行"));
    }

    /// 块标量提前去缩进（下一键顶格）→ 块结束。
    #[test]
    fn frontmatter_block_scalar_ends_at_dedent() {
        let (meta, _) =
            parse_frontmatter("---\nname: X\ndescription: >-\n  折叠内容\ntags: a\n---\nb");
        assert_eq!(meta.description.as_deref(), Some("折叠内容"));
        assert_eq!(meta.tags, vec!["a"]);
    }

    #[test]
    fn frontmatter_unclosed_fence_treated_as_body() {
        let (meta, body) = parse_frontmatter("---\nname: X\nno_close");
        assert_eq!(meta.name, None);
        assert!(body.contains("name: X"));
    }

    #[test]
    fn frontmatter_closed_fence_without_trailing_newline() {
        let (meta, body) = parse_frontmatter("---\nname: X\n---");
        assert_eq!(meta.name.as_deref(), Some("X"));
        assert_eq!(body, "");
    }

    #[test]
    fn frontmatter_absent() {
        let (meta, body) = parse_frontmatter("# 直接正文");
        assert_eq!(meta, FrontmatterMeta::default());
        assert_eq!(body, "# 直接正文");
    }

    #[test]
    fn frontmatter_tags_block_list() {
        // D7：YAML 块列表 tags（tags: 后跟 "- item" 行）
        let (meta, body) = parse_frontmatter(
            "---
name: X
tags:
  - zztest
  - 标签二
---
正文",
        );
        assert_eq!(meta.tags, vec!["zztest", "标签二"]);
        assert_eq!(body, "正文");
    }

    #[test]
    fn body_strips_leading_blank_lines() {
        // 块标量/闭合行后的前导空行不应残留进正文
        let (_, body) = parse_frontmatter(
            "---
name: X
description: |
  多行
---


正文内容",
        );
        assert_eq!(body, "正文内容");
    }

    #[test]
    fn tags_json_style() {
        assert_eq!(parse_tags_value(r#"["a","b"]"#), vec!["a", "b"]);
        assert_eq!(parse_tags_value(" single "), vec!["single"]);
        assert_eq!(parse_tags_value(""), Vec::<String>::new());
    }

    #[test]
    fn slugify_basics() {
        assert_eq!(slugify("Review PR").as_deref(), Some("review-pr"));
        assert_eq!(
            slugify("  code_review.md ").as_deref(),
            Some("code-review-md")
        );
        assert_eq!(slugify("中文技能"), None);
        assert_eq!(slugify("mixed 中文 name").as_deref(), Some("mixed-name"));
    }

    #[test]
    fn slug_validation() {
        assert!(valid_slug("review-pr"));
        assert!(valid_slug("a"));
        assert!(!valid_slug(""));
        assert!(!valid_slug("-lead"));
        assert!(!valid_slug("Has Upper"));
        assert!(!valid_slug("中文"));
        assert!(!valid_slug(&"a".repeat(81)));
    }

    #[test]
    fn script_ext_detection() {
        assert!(is_script_path("scripts/run.py"));
        assert!(is_script_path("run.sh"));
        assert!(is_script_path("a/b/c.PY")); // 大小写不敏感
        assert!(is_script_path("tools/x.ts"));
        assert!(is_script_path("x.mjs"));
        assert!(!is_script_path("references/api.md"));
        assert!(!is_script_path("README"));
        assert!(!is_script_path("data.json")); // json 不在清单（数据文件不是脚本）
    }

    #[test]
    fn two_kind_validation() {
        // script 缺 local_path
        assert!(SkillsService::validate_two_kind("script", "self", "", None, None).is_err());
        // script 带 content（正文不入库）
        assert!(
            SkillsService::validate_two_kind("script", "self", "正文", Some("/tmp/sk"), None)
                .is_err()
        );
        // text 带 local_path
        assert!(
            SkillsService::validate_two_kind("text", "self", "", Some("/tmp/sk"), None).is_err()
        );
        // origin=self 带 repo_url
        assert!(
            SkillsService::validate_two_kind(
                "text",
                "self",
                "",
                None,
                Some("https://github.com/a/b")
            )
            .is_err()
        );
        // 非法 kind / origin
        assert!(SkillsService::validate_two_kind("zip", "self", "", None, None).is_err());
        assert!(SkillsService::validate_two_kind("text", "mirror", "", None, None).is_err());
        // 合法 text（github 来源 + repo_url）
        let nk = SkillsService::validate_two_kind(
            "text",
            "github",
            "正文",
            None,
            Some("https://github.com/a/b"),
        )
        .unwrap();
        assert_eq!((nk.kind.as_str(), nk.origin.as_str()), ("text", "github"));
        assert_eq!(nk.local_path, None);
        assert_eq!(nk.repo_url.as_deref(), Some("https://github.com/a/b"));
        assert!(nk.snapshot);
        // 合法 script（trim 生效、不产快照）
        let nk = SkillsService::validate_two_kind(
            "script",
            "both",
            "",
            Some(" /tmp/my-skill "),
            Some(" https://github.com/a/b "),
        )
        .unwrap();
        assert_eq!((nk.kind.as_str(), nk.origin.as_str()), ("script", "both"));
        assert_eq!(nk.local_path.as_deref(), Some("/tmp/my-skill"));
        assert_eq!(nk.repo_url.as_deref(), Some("https://github.com/a/b"));
        assert!(!nk.snapshot);
    }
}

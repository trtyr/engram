//! 技能域服务（第六域）：AI 技能（SKILL.md 形态）的资产化管理。
//!
//! 技能 = slug 唯一 + frontmatter（name/description/tags）+ markdown 正文的可复用指令包。
//! 语义字段每次变更前留版本快照（skill_revisions，保留最近 50 版），可回滚。
//! 批量导入直接吃 SKILL.md 全文（frontmatter 容错解析），迁移现有技能库零改写。
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
    // 闭合行之后的内容是正文（跳过闭合行的行尾）
    let after = &body_start[close_rel + 4..];
    let body = after
        .strip_prefix('\n')
        .or_else(|| after.strip_prefix("\r\n"))
        .unwrap_or(after);

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
#[derive(Debug, Clone, Copy)]
pub struct NewSkill<'a> {
    pub slug: Option<&'a str>,
    pub name: &'a str,
    pub description: &'a str,
    pub content: &'a str,
    pub tags: &'a [String],
    pub enabled: bool,
    pub source: &'a str,
}

/// 新建/更新的可选语义字段（None = 不动）。
#[derive(Debug, Default, Clone)]
pub struct SkillPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub enabled: Option<bool>,
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
            None => slugify(name).ok_or_else(|| {
                SkillsError::BadRequest(format!(
                    "无法从技能名「{name}」推导 slug（非 ASCII 名字请显式传 slug，如 code-review）"
                ))
            }),
        }
    }

    pub async fn create_skill(&self, s: NewSkill<'_>) -> Result<SkillDto, SkillsError> {
        Self::validate_name(s.name)?;
        let name = s.name.trim();
        let slug = Self::resolve_slug(s.slug, s.name)?;
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
            },
        )
        .await?;
        if inserted == 0 {
            return Err(SkillsError::Conflict(format!(
                "slug「{slug}」已被占用——slug 唯一，请换 slug 或直接更新已有技能"
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
        repo::get_skill(&self.pool, slug).await?.ok_or_else(|| {
            SkillsError::NotFound(format!(
                "技能 {slug:?} 不存在——先 skills_list 确认 slug（可能已删除或抄错）"
            ))
        })
    }

    /// 语义字段更新：先快照现状（origin=update），再落变更；enabled-only 不留版本。
    pub async fn update_skill(
        &self,
        slug: &str,
        patch: SkillPatch,
    ) -> Result<SkillDto, SkillsError> {
        let current = self.get_skill(slug).await?;
        let semantic_change = patch.name.is_some()
            || patch.description.is_some()
            || patch.content.is_some()
            || patch.tags.is_some();
        let snapshot = semantic_change.then(|| repo::SkillSnapshot {
            skill_id: current.id,
            name: &current.name,
            description: &current.description,
            content: &current.content,
            tags: &current.tags,
            origin: "update",
        });
        let patch_data = repo::SkillPatchData {
            name: patch.name.as_deref().map(str::trim),
            description: patch.description.as_deref().map(str::trim),
            content: patch.content.as_deref(),
            tags: &patch.tags,
            enabled: patch.enabled,
        };
        let updated =
            repo::update_skill_tx(&self.pool, current.id, snapshot.as_ref(), &patch_data).await?;
        if updated == 0 {
            return Err(SkillsError::NotFound(format!(
                "技能 {slug:?} 不存在——先 skills_list 确认 slug（可能已删除或抄错）"
            )));
        }
        self.get_skill(slug).await
    }

    pub async fn delete_skill(&self, slug: &str) -> Result<bool, SkillsError> {
        let deleted = repo::delete_skill(&self.pool, slug).await?;
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

    /// 全量导出（含正文，按 slug 排序——数据主权：技能库随时整体带走）。
    /// 单技能导出（bundle 整包用）：本体 + 全部附属文件。
    pub async fn export_one(&self, slug: &str) -> Result<SkillExportDto, SkillsError> {
        let skill = self.get_skill(slug).await?;
        let files = repo::skill_file_contents(&self.pool, skill.id)
            .await?
            .into_iter()
            .map(|(path, content)| SkillFileEntryDto { path, content })
            .collect();
        Ok(SkillExportDto { skill, files })
    }

    /// 全量导出（含附属文件——folder 形态整体带走，数据主权）。
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
        if p.starts_with('/') || p.contains('\\') {
            return Err(SkillsError::BadRequest(
                "path 必须是相对路径且用 / 分隔（如 scripts/run.py、references/api.md）".into(),
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
        Ok(repo::list_revisions(&self.pool, skill.id).await?)
    }

    /// 回滚到某个版本：先快照现状（origin=restore），再把目标版本内容落回本体。
    pub async fn restore_revision(
        &self,
        slug: &str,
        revision_id: Uuid,
    ) -> Result<SkillDto, SkillsError> {
        let skill = self.get_skill(slug).await?;
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
}

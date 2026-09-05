//! 技能域服务（第六域）：AI 技能（SKILL.md 形态）的资产化管理。
//!
//! 技能 = slug 唯一 + frontmatter（name/description/tags）+ markdown 正文的可复用指令包。
//! 语义字段每次变更前留版本快照（skill_revisions，保留最近 50 版），可回滚。
//! 批量导入直接吃 SKILL.md 全文（frontmatter 容错解析），迁移现有技能库零改写。

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
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

impl From<sqlx::Error> for SkillsError {
    fn from(e: sqlx::Error) -> Self {
        SkillsError::Storage(e.to_string())
    }
}

/// 版本快照保留上限（防膨胀；更老的自动淘汰）。
pub const MAX_REVISIONS: i32 = 50;

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

// ---------- DTO ----------

/// 列表/导入/概览用摘要（不含正文——列表与仪表盘不必拖全量指令）。
#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct SkillSummaryDto {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub enabled: bool,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 详情（含 markdown 正文）。
#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct SkillDto {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub content: String,
    pub tags: Vec<String>,
    pub enabled: bool,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 版本快照。
#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct SkillRevisionDto {
    pub id: Uuid,
    pub skill_id: Uuid,
    pub rev: i32,
    pub name: String,
    pub description: String,
    pub content: String,
    pub tags: Vec<String>,
    /// create=初始版 / update=变更前快照 / restore=回滚前的现状快照
    pub origin: String,
    pub created_at: DateTime<Utc>,
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

const SUMMARY_COLS: &str =
    "id, slug, name, description, tags, enabled, source, created_at, updated_at";
const FULL_COLS: &str =
    "id, slug, name, description, content, tags, enabled, source, created_at, updated_at";
const REV_COLS: &str = "id, skill_id, rev, name, description, content, tags, origin, created_at";

// ---------- Service ----------

pub struct SkillsService {
    pool: PgPool,
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

/// 版本快照内容束（insert_revision 入参收拢）。
struct Snapshot<'a> {
    skill_id: Uuid,
    name: &'a str,
    description: &'a str,
    content: &'a str,
    tags: &'a [String],
    origin: &'a str,
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
    pub fn new(pool: PgPool) -> Self {
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

    async fn insert_revision(
        &self,
        tx: &mut sqlx::PgConnection,
        snap: Snapshot<'_>,
    ) -> Result<(), SkillsError> {
        sqlx::query(
            "INSERT INTO skill_revisions (id, skill_id, rev, name, description, content, tags, origin) \
             VALUES ($1, $2, (SELECT COALESCE(MAX(rev), 0) + 1 FROM skill_revisions WHERE skill_id = $2), \
                     $3, $4, $5, $6, $7)",
        )
        .bind(Uuid::now_v7())
        .bind(snap.skill_id)
        .bind(snap.name)
        .bind(snap.description)
        .bind(snap.content)
        .bind(snap.tags)
        .bind(snap.origin)
        .execute(&mut *tx)
        .await?;
        // 淘汰超限旧版（保留最近 MAX_REVISIONS 版）
        sqlx::query(
            "DELETE FROM skill_revisions WHERE skill_id = $1 \
             AND rev <= (SELECT MAX(rev) FROM skill_revisions WHERE skill_id = $1) - $2",
        )
        .bind(snap.skill_id)
        .bind(MAX_REVISIONS)
        .execute(&mut *tx)
        .await?;
        Ok(())
    }

    pub async fn create_skill(&self, s: NewSkill<'_>) -> Result<SkillDto, SkillsError> {
        Self::validate_name(s.name)?;
        let name = s.name.trim();
        let slug = Self::resolve_slug(s.slug, s.name)?;
        let id = Uuid::now_v7();
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(
            "INSERT INTO skills (id, slug, name, description, content, tags, enabled, source) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (slug) DO NOTHING",
        )
        .bind(id)
        .bind(&slug)
        .bind(name)
        .bind(s.description.trim())
        .bind(s.content)
        .bind(s.tags)
        .bind(s.enabled)
        .bind(s.source)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            return Err(SkillsError::Conflict(format!(
                "slug「{slug}」已被占用——slug 唯一，请换 slug 或直接更新已有技能"
            )));
        }
        self.insert_revision(
            &mut tx,
            Snapshot {
                skill_id: id,
                name,
                description: s.description.trim(),
                content: s.content,
                tags: s.tags,
                origin: "create",
            },
        )
        .await?;
        tx.commit().await?;
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
        let rows = sqlx::query_as::<_, SkillSummaryDto>(&format!(
            "SELECT {SUMMARY_COLS} FROM skills \
             WHERE ($1::text IS NULL OR name ILIKE $1 OR description ILIKE $1) \
             AND ($2::text[] IS NULL OR tags @> $2) \
             AND ($3::bool IS NULL OR enabled = $3) \
             ORDER BY updated_at DESC"
        ))
        .bind(pattern)
        .bind(tag_vec)
        .bind(enabled)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_skill(&self, slug: &str) -> Result<SkillDto, SkillsError> {
        sqlx::query_as::<_, SkillDto>(&format!("SELECT {FULL_COLS} FROM skills WHERE slug = $1"))
            .bind(slug)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| {
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
        let mut tx = self.pool.begin().await?;
        if semantic_change {
            self.insert_revision(
                &mut tx,
                Snapshot {
                    skill_id: current.id,
                    name: &current.name,
                    description: &current.description,
                    content: &current.content,
                    tags: &current.tags,
                    origin: "update",
                },
            )
            .await?;
        }
        let res = sqlx::query(
            "UPDATE skills SET \
                name = COALESCE($2, name), \
                description = COALESCE($3, description), \
                content = COALESCE($4, content), \
                tags = COALESCE($5, tags), \
                enabled = COALESCE($6, enabled), \
                updated_at = now() \
             WHERE id = $1",
        )
        .bind(current.id)
        .bind(patch.name.as_deref().map(str::trim))
        .bind(patch.description.as_deref().map(str::trim))
        .bind(patch.content.as_deref())
        .bind(&patch.tags)
        .bind(patch.enabled)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            return Err(SkillsError::NotFound(format!(
                "技能 {slug:?} 不存在——先 skills_list 确认 slug（可能已删除或抄错）"
            )));
        }
        tx.commit().await?;
        self.get_skill(slug).await
    }

    pub async fn delete_skill(&self, slug: &str) -> Result<bool, SkillsError> {
        let res = sqlx::query("DELETE FROM skills WHERE slug = $1")
            .bind(slug)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
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
        let exists = sqlx::query_as::<_, (Uuid,)>("SELECT id FROM skills WHERE slug = $1")
            .bind(&slug)
            .fetch_optional(&self.pool)
            .await;
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
    pub async fn export_skills(&self) -> Result<Vec<SkillDto>, SkillsError> {
        Ok(
            sqlx::query_as::<_, SkillDto>(&format!("SELECT {FULL_COLS} FROM skills ORDER BY slug"))
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn list_revisions(&self, slug: &str) -> Result<Vec<SkillRevisionDto>, SkillsError> {
        let skill = self.get_skill(slug).await?;
        Ok(sqlx::query_as::<_, SkillRevisionDto>(&format!(
            "SELECT {REV_COLS} FROM skill_revisions WHERE skill_id = $1 ORDER BY rev DESC"
        ))
        .bind(skill.id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// 回滚到某个版本：先快照现状（origin=restore），再把目标版本内容落回本体。
    pub async fn restore_revision(
        &self,
        slug: &str,
        revision_id: Uuid,
    ) -> Result<SkillDto, SkillsError> {
        let skill = self.get_skill(slug).await?;
        let rev = sqlx::query_as::<_, SkillRevisionDto>(&format!(
            "SELECT {REV_COLS} FROM skill_revisions WHERE id = $1 AND skill_id = $2"
        ))
        .bind(revision_id)
        .bind(skill.id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            SkillsError::NotFound(format!(
                "版本 {revision_id} 不存在——先查 {slug} 的版本列表取 id"
            ))
        })?;
        let mut tx = self.pool.begin().await?;
        self.insert_revision(
            &mut tx,
            Snapshot {
                skill_id: skill.id,
                name: &skill.name,
                description: &skill.description,
                content: &skill.content,
                tags: &skill.tags,
                origin: "restore",
            },
        )
        .await?;
        sqlx::query(
            "UPDATE skills SET name = $2, description = $3, content = $4, tags = $5, \
             updated_at = now() WHERE id = $1",
        )
        .bind(skill.id)
        .bind(&rev.name)
        .bind(&rev.description)
        .bind(&rev.content)
        .bind(&rev.tags)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
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

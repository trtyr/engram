//! `skills` 的实现切片（架构治理 2026-09-21：自 skills.rs 纯搬移，零行为变化）。

use super::*;

impl SkillsService {
    pub fn new(pool: engram_storage::PgPool) -> Self {
        Self { pool }
    }

    pub(crate) fn validate_name(name: &str) -> Result<(), SkillsError> {
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
    pub(crate) fn validate_two_kind(
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

    pub(crate) fn resolve_slug(slug: Option<&str>, name: &str) -> Result<String, SkillsError> {
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
    pub(crate) fn reject_script_ops(s: &SkillDto, op: &str) -> Result<(), SkillsError> {
        if s.kind == "script" {
            return Err(SkillsError::BadRequest(format!(
                "{op} 对 script 型技能不可用——文件真身在本地 {}，请直接操作本地文件夹（系统只存指针）",
                s.local_path.as_deref().unwrap_or("(local_path 缺失)")
            )));
        }
        Ok(())
    }

    /// slug/name → 真实 slug（更新/删除/文件操作寻址用；slug 优先，name 精确兜底）。
    pub(crate) async fn resolve(&self, slug_or_name: &str) -> Result<String, SkillsError> {
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
        let (local_path, repo_url) = resolve_skill_paths(&current, &patch, &origin)?;
        let snapshot = skill_snapshot(&current, &patch);
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
}

/// update 解析 local_path（script 型专用，含长度校验）与 repo_url（self 型联动清空）。
fn resolve_skill_paths(
    current: &SkillDto,
    patch: &SkillPatch,
    origin: &str,
) -> Result<(Option<String>, Option<String>), SkillsError> {
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
    Ok((local_path, repo_url))
}

/// 语义变更（name/description/content/tags）且非 script 型 → 落历史快照（origin=update）。
fn skill_snapshot<'a>(
    current: &'a SkillDto,
    patch: &SkillPatch,
) -> Option<repo::SkillSnapshot<'a>> {
    let semantic_change = patch.name.is_some()
        || patch.description.is_some()
        || patch.content.is_some()
        || patch.tags.is_some();
    let is_script = current.kind == "script";

    if semantic_change && !is_script {
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
    }
}

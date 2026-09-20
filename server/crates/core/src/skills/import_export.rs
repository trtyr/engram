//! `skills` 的实现切片（架构治理 2026-09-21：自 skills.rs 纯搬移，零行为变化）。

use super::*;

impl SkillsService {
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

    pub(crate) async fn import_one(
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
        let name = meta.name.clone().or(fallback_name).unwrap_or_default();
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
        self.import_upsert(index, name, slug, meta, body, tags, source, overwrite)
            .await
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

    /// 导入落库分支：已存在 →（overwrite 时）更新；不存在 → 新建。失败统一成 failed 项。
    #[allow(clippy::too_many_arguments)]
    async fn import_upsert(
        &self,
        index: usize,
        name: String,
        slug: String,
        meta: FrontmatterMeta,
        body: String,
        tags: Vec<String>,
        source: &str,
        overwrite: bool,
    ) -> SkillImportItem {
        let make_err = |e: SkillsError| SkillImportItem {
            index,
            slug: None,
            status: "failed".into(),
            error: Some(e.to_string()),
        };
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
}

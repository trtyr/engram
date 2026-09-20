//! `skills` 的实现切片（架构治理 2026-09-21：自 skills.rs 纯搬移，零行为变化）。

use super::*;

impl SkillsService {
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
    pub(crate) fn validate_file_path(path: &str) -> Result<(), SkillsError> {
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
}

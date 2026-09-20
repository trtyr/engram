//! `skills` 的实现切片（架构治理 2026-09-21：自 skills.rs 纯搬移，零行为变化）。

use super::*;

impl SkillsService {
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

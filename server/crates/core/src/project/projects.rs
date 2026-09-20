//! `project` 的实现切片（架构治理 2026-09-21：自 project.rs 纯搬移，零行为变化）。

use super::*;

impl ProjectService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 类型模板预设分类（未知类型返回 None）。
    pub fn default_categories(type_: &str) -> Option<Vec<String>> {
        PROJECT_TYPES
            .iter()
            .find(|(t, _)| *t == type_)
            .map(|(_, cats)| cats.iter().map(|s| s.to_string()).collect())
    }

    /// 类型模板列表（Web 建项目时选择类型）。
    pub fn list_types() -> Vec<ProjectTypeDto> {
        PROJECT_TYPES
            .iter()
            .map(|(t, cats)| ProjectTypeDto {
                r#type: (*t).to_string(),
                label: type_label(t),
                default_categories: cats.iter().map(|s| s.to_string()).collect(),
            })
            .collect()
    }

    pub async fn create_project(
        &self,
        name: &str,
        type_: &str,
        description: Option<&str>,
    ) -> Result<ProjectDto, ProjectError> {
        if name.trim().is_empty() {
            return Err(ProjectError::BadRequest("项目名不能为空".to_string()));
        }
        if name.trim().chars().count() > 200 {
            return Err(ProjectError::BadRequest(
                "项目名过长（>200 字符）".to_string(),
            ));
        }
        let categories = Self::default_categories(type_).ok_or_else(|| {
            ProjectError::BadRequest(format!("未知项目类型: {type_}（支持 dev/research）"))
        })?;
        let id = Uuid::now_v7();
        let inserted =
            repo::insert_project(&self.pool, id, name, type_, description, &categories).await?;
        if inserted == 0 {
            return Err(ProjectError::Conflict(format!(
                "项目名「{name}」已存在——项目名唯一，请改名或复用已有项目（先 projects 列表确认）"
            )));
        }
        self.get_project_bare(id).await
    }

    pub async fn list_projects(
        &self,
        type_filter: Option<&str>,
    ) -> Result<Vec<ProjectDto>, ProjectError> {
        Ok(repo::list_projects(&self.pool, type_filter).await?)
    }

    pub async fn get_project(&self, id: Uuid) -> Result<ProjectDetailDto, ProjectError> {
        let project = self.get_project_bare(id).await?;
        let locations = repo::list_locations(&self.pool, id).await?;
        let docs = repo::list_docs(&self.pool, id).await?;
        Ok(ProjectDetailDto {
            id: project.id,
            name: project.name,
            r#type: project.r#type,
            status: project.status,
            description: project.description,
            categories: project.categories,
            frontmatter: project.frontmatter,
            created_at: project.created_at,
            updated_at: project.updated_at,
            locations,
            docs,
        })
    }

    /// 按名精确定位项目 id（项目名唯一；MCP 工具的 name 寻址入口）。
    pub async fn project_id_by_name(&self, name: &str) -> Result<Uuid, ProjectError> {
        repo::id_by_name(&self.pool, name).await?.ok_or_else(|| {
            ProjectError::NotFound(format!(
                "项目「{name}」不存在——先跑 projects 列表确认名字（可能已删除或写错）"
            ))
        })
    }

    pub async fn update_project(
        &self,
        id: Uuid,
        name: &str,
        status: &str,
        description: Option<&str>,
        categories: &[String],
    ) -> Result<ProjectDto, ProjectError> {
        if !PROJECT_STATUSES.contains(&status) {
            return Err(ProjectError::BadRequest(format!("未知状态: {status}")));
        }
        if name.trim().is_empty() {
            return Err(ProjectError::BadRequest("项目名不能为空".to_string()));
        }
        if name.trim().chars().count() > 200 {
            return Err(ProjectError::BadRequest(
                "项目名过长（>200 字符）".to_string(),
            ));
        }
        // 改名撞唯一约束会以 sqlx 错误冒成 500，这里先查给出 409 语义
        if let Some(holder) = repo::name_holder(&self.pool, name, id).await? {
            return Err(ProjectError::Conflict(format!(
                "项目名「{name}」已被项目 {holder} 占用——项目名唯一，请改名"
            )));
        }
        let updated =
            repo::update_project(&self.pool, id, name, status, description, categories).await?;
        if updated == 0 {
            return Err(ProjectError::NotFound(format!(
                "项目 {id} 不存在——先跑 projects 列表确认 id（可能已删除或抄错）"
            )));
        }
        self.get_project_bare(id).await
    }

    pub async fn delete_project(&self, id: Uuid) -> Result<bool, ProjectError> {
        let deleted = repo::delete_project(&self.pool, id).await?;
        if deleted == 0 {
            return Err(ProjectError::NotFound(format!(
                "项目 {id} 不存在——先跑 projects 列表确认 id（可能已删除或抄错）"
            )));
        }
        Ok(true)
    }

    /// 批量删除（列表多选），返回（删除条数, 不存在的 id）。
    pub async fn batch_delete_projects(
        &self,
        ids: &[Uuid],
    ) -> Result<(usize, Vec<Uuid>), ProjectError> {
        // 先查存在集合，用于区分「删掉」与「本就不存在」
        let existing: std::collections::HashSet<Uuid> = repo::existing_ids(&self.pool, ids)
            .await?
            .into_iter()
            .collect();
        let deleted = repo::delete_projects(&self.pool, ids).await? as usize;
        let failed: Vec<Uuid> = ids
            .iter()
            .copied()
            .filter(|id| !existing.contains(id))
            .collect();
        Ok((deleted, failed))
    }

    pub(crate) async fn get_project_bare(&self, id: Uuid) -> Result<ProjectDto, ProjectError> {
        repo::get_project(&self.pool, id).await?.ok_or_else(|| {
            ProjectError::NotFound(format!(
                "项目 {id} 不存在——先跑 projects 列表确认 id（可能已删除或抄错）"
            ))
        })
    }

    pub async fn add_location(
        &self,
        project_id: Uuid,
        ip: &str,
        host: &str,
        os: &str,
        path: &str,
        purpose: Option<&str>,
    ) -> Result<ProjectLocationDto, ProjectError> {
        self.get_project_bare(project_id).await?;
        let id = Uuid::now_v7();
        repo::insert_location(&self.pool, id, project_id, ip, host, os, path, purpose).await?;
        self.get_location(id).await
    }

    pub async fn update_location(
        &self,
        id: Uuid,
        ip: &str,
        host: &str,
        os: &str,
        path: &str,
        purpose: Option<&str>,
    ) -> Result<ProjectLocationDto, ProjectError> {
        let updated = repo::update_location(&self.pool, id, ip, host, os, path, purpose).await?;
        if updated == 0 {
            return Err(ProjectError::NotFound(format!(
                "位置 {id} 不存在——先 project-get <项目> 看 locations 列表取 id"
            )));
        }
        self.get_location(id).await
    }

    pub async fn delete_location(&self, id: Uuid) -> Result<bool, ProjectError> {
        let deleted = repo::delete_location(&self.pool, id).await?;
        if deleted == 0 {
            return Err(ProjectError::NotFound(format!(
                "位置 {id} 不存在——先 project-get <项目> 看 locations 列表取 id"
            )));
        }
        Ok(true)
    }

    pub async fn get_location(&self, id: Uuid) -> Result<ProjectLocationDto, ProjectError> {
        repo::get_location(&self.pool, id).await?.ok_or_else(|| {
            ProjectError::NotFound(format!(
                "位置 {id} 不存在——先 project-get <项目> 看 locations 列表取 id"
            ))
        })
    }

    /// 分类校验错误（列出项目现有分类，提示先扩 categories）。
    pub(crate) fn category_error(category: &str, categories: &[String]) -> ProjectError {
        let existing = if categories.is_empty() {
            "项目还没有分类".to_string()
        } else {
            categories.join("、")
        };
        ProjectError::BadRequest(format!(
            "分类「{category}」不在项目分类里（现有：{existing}）——要新分类就先 update_project 把它加进 categories"
        ))
    }
}

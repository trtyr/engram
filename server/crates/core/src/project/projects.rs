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
            ProjectError::BadRequest(format!(
                "未知项目类型（场景）: {type_}——支持 {}；先跑 projects types 看各场景的预设分类",
                super::supported_types()
            ))
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
        let assets = engram_storage::repo::asset::assets_used_by_project(&self.pool, id).await?;
        let links = repo::list_links(&self.pool, id).await?;
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
            assets,
            links,
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

    #[allow(clippy::too_many_arguments)]
    pub async fn add_location(
        &self,
        project_id: Uuid,
        ip: &str,
        host: &str,
        os: &str,
        path: &str,
        purpose: Option<&str>,
        asset_id: Option<Uuid>,
    ) -> Result<ProjectLocationDto, ProjectError> {
        self.get_project_bare(project_id).await?;
        self.ensure_asset_exists(asset_id).await?;
        let id = Uuid::now_v7();
        repo::insert_location(
            &self.pool, id, project_id, ip, host, os, path, purpose, asset_id,
        )
        .await?;
        self.get_location(id).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_location(
        &self,
        id: Uuid,
        ip: &str,
        host: &str,
        os: &str,
        path: &str,
        purpose: Option<&str>,
        asset_id: Option<Uuid>,
    ) -> Result<ProjectLocationDto, ProjectError> {
        self.ensure_asset_exists(asset_id).await?;
        let updated =
            repo::update_location(&self.pool, id, ip, host, os, path, purpose, asset_id).await?;
        if updated == 0 {
            return Err(ProjectError::NotFound(format!(
                "位置 {id} 不存在——先 project-get <项目> 看 locations 列表取 id"
            )));
        }
        self.get_location(id).await
    }

    /// 引用校验：给定的 asset_id 必须真实存在（唯一事实源铁律——引用不许悬空）。
    async fn ensure_asset_exists(&self, asset_id: Option<Uuid>) -> Result<(), ProjectError> {
        let Some(aid) = asset_id else {
            return Ok(());
        };
        if engram_storage::repo::asset::get_asset(&self.pool, aid)
            .await?
            .is_none()
        {
            return Err(ProjectError::BadRequest(format!(
                "资产 {aid} 不存在——先 assets list 定位台账条目（或跑 assets add 建档）"
            )));
        }
        Ok(())
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

    // ---------- 项目关联（project_links，0058） ----------

    /// 建关联：`part_of` = from 隶属 to（子 → 母）；`related` = 相关。
    /// 自环与同向同类重复被拒（纪律写进表约束，服务层给可行动报错）。
    pub async fn add_link(
        &self,
        from_project: Uuid,
        to_project: Uuid,
        kind: &str,
        note: &str,
    ) -> Result<ProjectLinkDto, ProjectError> {
        if !is_valid_link_kind(kind) {
            return Err(ProjectError::BadRequest(format!(
                "未知关联类型: {kind}——支持 {}",
                supported_link_kinds()
            )));
        }
        if from_project == to_project {
            return Err(ProjectError::BadRequest(
                "不能把项目关联到自己（自环无意义）——隶属/相关都要指向另一个项目".into(),
            ));
        }
        self.get_project_bare(from_project).await?;
        self.get_project_bare(to_project).await?;
        if repo::find_link(&self.pool, from_project, to_project, kind)
            .await?
            .is_some()
        {
            return Err(ProjectError::Conflict(format!(
                "这两条工作线之间已有 {kind} 关联——先 links 看现状，要改就 unlink 再建"
            )));
        }
        let id = Uuid::now_v7();
        let n =
            repo::insert_link(&self.pool, id, from_project, to_project, kind, note.trim()).await?;
        if n == 0 {
            return Err(ProjectError::Conflict("关联已存在（并发写入）".into()));
        }
        repo::get_link(&self.pool, id)
            .await?
            .ok_or_else(|| ProjectError::Storage("插入后读回失败".into()))
    }

    /// 解绑一条关联（按关联 id）。
    pub async fn remove_link(&self, id: Uuid) -> Result<bool, ProjectError> {
        let n = repo::delete_link(&self.pool, id).await?;
        if n == 0 {
            return Err(ProjectError::NotFound(format!(
                "关联 {id} 不存在——先 links 看现状（id 从 links 或 project-get 的 links 取）"
            )));
        }
        Ok(true)
    }

    /// 某项目的全部关联（两向合并；前端按 kind 分「隶属 / 下属 / 相关」）。
    pub async fn list_links(&self, project_id: Uuid) -> Result<Vec<ProjectLinkDto>, ProjectError> {
        self.get_project_bare(project_id).await?;
        Ok(repo::list_links(&self.pool, project_id).await?)
    }

    /// 工作线 ↔ 资产关系图（一次取全：项目 + 资产 + 全量关联，前端不跑 N+1）。
    pub async fn graph(&self) -> Result<ProjectGraphDto, ProjectError> {
        let projects = repo::list_projects(&self.pool, None).await?;
        let assets = engram_storage::repo::asset::list_assets(&self.pool, None, None).await?;
        let links = repo::list_all_links(&self.pool).await?;
        let usages = engram_storage::repo::asset::all_project_asset_pairs(&self.pool).await?;
        Ok(ProjectGraphDto {
            projects,
            assets,
            links,
            usages,
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

//! 项目记忆域服务：项目 / 多主机位置 / 分类文档的三表 CRUD + 类型模板。
//!
//! 设计：docs/plantree/plans/project-memory/（0005 三表模型、类型=分类模板）。
//! 类型模板是代码常量（三表决策不建第四表），新建项目时复制进 projects.categories，
//! 之后项目级自由增删。

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// 项目域错误（api 层转 ApiError）。
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("存储暂时不可用: {0}")]
    Storage(String),
}

impl From<sqlx::Error> for ProjectError {
    fn from(e: sqlx::Error) -> Self {
        ProjectError::Storage(e.to_string())
    }
}

/// 类型模板：type → 预设分类列表（0005：开发四分类 / 调研六分类）。
pub const PROJECT_TYPES: &[(&str, &[&str])] = &[
    ("dev", &["后端", "前端", "测试", "规划"]),
    (
        "research",
        &["待查", "线索", "资料", "结论", "疑点", "证伪"],
    ),
];

/// 项目状态枚举（英文存库，Web 层映射中文）：active/paused/done/abandoned。
pub const PROJECT_STATUSES: &[&str] = &["active", "paused", "done", "abandoned"];

/// 类型显示名（Web 用）。
pub fn type_label(type_: &str) -> String {
    match type_ {
        "dev" => "开发".into(),
        "research" => "调研".into(),
        other => other.into(),
    }
}

/// 类型模板项（Web 建项目时选择类型用）。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProjectTypeDto {
    pub r#type: String,
    pub label: String,
    pub default_categories: Vec<String>,
}

// ---------- DTO ----------

#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ProjectDto {
    pub id: Uuid,
    pub name: String,
    pub r#type: String,
    pub status: String,
    pub description: Option<String>,
    #[sqlx(json)]
    pub categories: Vec<String>,
    #[schema(value_type = Object)]
    pub frontmatter: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ProjectLocationDto {
    pub id: Uuid,
    pub project_id: Uuid,
    pub ip: String,
    pub host: String,
    pub os: String,
    pub path: String,
    pub purpose: Option<String>,
    pub sort_order: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ProjectDocDto {
    pub id: Uuid,
    pub project_id: Uuid,
    pub category: String,
    pub title: String,
    pub content: String,
    #[schema(value_type = Object)]
    pub frontmatter: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 文档行检索命中（grep 式定位：行号 + 原文行；配合 read_doc_lines 区间精读）。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DocLineHitDto {
    pub doc_id: Uuid,
    pub title: String,
    pub category: String,
    /// 1-based 行号（基于文档当前版本）
    pub line: i64,
    pub text: String,
}

/// 项目详情（本体 + 位置 + 文档），Web 详情页左树右内容用。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProjectDetailDto {
    pub id: Uuid,
    pub name: String,
    pub r#type: String,
    pub status: String,
    pub description: Option<String>,
    pub categories: Vec<String>,
    #[schema(value_type = Object)]
    pub frontmatter: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub locations: Vec<ProjectLocationDto>,
    pub docs: Vec<ProjectDocDto>,
}

// ---------- Service ----------

pub struct ProjectService {
    pool: PgPool,
}

const PROJECT_COLS: &str =
    "id, name, type, status, description, categories, frontmatter, created_at, updated_at";
const LOCATION_COLS: &str =
    "id, project_id, ip, host, os, path, purpose, sort_order, created_at, updated_at";
const DOC_COLS: &str =
    "id, project_id, category, title, content, frontmatter, created_at, updated_at";

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

    // ---------- 项目 CRUD ----------

    pub async fn create_project(
        &self,
        name: &str,
        type_: &str,
        description: Option<&str>,
    ) -> Result<ProjectDto, ProjectError> {
        if name.trim().is_empty() {
            return Err(ProjectError::BadRequest(
                "项目名不能为空".to_string(),
            ));
        }
        let categories = Self::default_categories(type_).ok_or_else(|| {
            ProjectError::BadRequest(format!("未知项目类型: {type_}（支持 dev/research）"))
        })?;
        let id = Uuid::now_v7();
        let res = sqlx::query(
            "INSERT INTO projects (id, name, type, status, description, categories) \
             VALUES ($1, $2, $3, 'active', $4, $5) \
             ON CONFLICT (name) DO NOTHING",
        )
        .bind(id)
        .bind(name)
        .bind(type_)
        .bind(description)
        .bind(sqlx::types::Json(&categories))
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
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
        let rows = match type_filter {
            Some(t) => sqlx::query_as::<_, ProjectDto>(&format!(
                "SELECT {PROJECT_COLS} FROM projects WHERE type = $1 ORDER BY created_at DESC, name"
            ))
            .bind(t)
            .fetch_all(&self.pool)
            .await?,
            None => {
                sqlx::query_as::<_, ProjectDto>(&format!(
                    "SELECT {PROJECT_COLS} FROM projects ORDER BY created_at DESC, name"
                ))
                .fetch_all(&self.pool)
                .await?
            }
        };
        Ok(rows)
    }

    pub async fn get_project(&self, id: Uuid) -> Result<ProjectDetailDto, ProjectError> {
        let project = self.get_project_bare(id).await?;
        let locations: Vec<ProjectLocationDto> = sqlx::query_as::<_, ProjectLocationDto>(&format!(
            "SELECT {LOCATION_COLS} FROM project_locations WHERE project_id = $1 ORDER BY sort_order, created_at"
        ))
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        let docs: Vec<ProjectDocDto> = sqlx::query_as::<_, ProjectDocDto>(&format!(
            "SELECT {DOC_COLS} FROM project_docs WHERE project_id = $1 ORDER BY category, created_at"
        ))
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
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
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM projects WHERE name = $1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| {
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
        // 改名撞唯一约束会以 sqlx 错误冒成 500，这里先查给出 409 语义
        if let Some(holder) = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM projects WHERE name = $1 AND id <> $2",
        )
        .bind(name)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        {
            return Err(ProjectError::Conflict(format!(
                "项目名「{name}」已被项目 {holder} 占用——项目名唯一，请改名"
            )));
        }
        let res = sqlx::query(
            "UPDATE projects SET name = $2, status = $3, description = $4, categories = $5, \
             updated_at = now() WHERE id = $1",
        )
        .bind(id)
        .bind(name)
        .bind(status)
        .bind(description)
        .bind(sqlx::types::Json(categories))
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Err(ProjectError::NotFound(format!(
                "项目 {id} 不存在——先跑 projects 列表确认 id（可能已删除或抄错）"
            )));
        }
        self.get_project_bare(id).await
    }

    pub async fn delete_project(&self, id: Uuid) -> Result<bool, ProjectError> {
        let res = sqlx::query("DELETE FROM projects WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
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
        let existing: std::collections::HashSet<Uuid> =
            sqlx::query_as::<_, (Uuid,)>("SELECT id FROM projects WHERE id = ANY($1)")
                .bind(ids)
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .map(|(id,)| id)
                .collect();
        let res = sqlx::query("DELETE FROM projects WHERE id = ANY($1)")
            .bind(ids)
            .execute(&self.pool)
            .await?;
        let deleted = res.rows_affected() as usize;
        let failed: Vec<Uuid> = ids
            .iter()
            .copied()
            .filter(|id| !existing.contains(id))
            .collect();
        Ok((deleted, failed))
    }

    async fn get_project_bare(&self, id: Uuid) -> Result<ProjectDto, ProjectError> {
        sqlx::query_as::<_, ProjectDto>(&format!(
            "SELECT {PROJECT_COLS} FROM projects WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            ProjectError::NotFound(format!(
                "项目 {id} 不存在——先跑 projects 列表确认 id（可能已删除或抄错）"
            ))
        })
    }

    // ---------- 位置（多主机） ----------

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
        sqlx::query(
            "INSERT INTO project_locations (id, project_id, ip, host, os, path, purpose) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id)
        .bind(project_id)
        .bind(ip)
        .bind(host)
        .bind(os)
        .bind(path)
        .bind(purpose)
        .execute(&self.pool)
        .await?;
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
        let res = sqlx::query(
            "UPDATE project_locations SET ip = $2, host = $3, os = $4, path = $5, purpose = $6, updated_at = now() \
             WHERE id = $1",
        )
        .bind(id)
        .bind(ip)
        .bind(host)
        .bind(os)
        .bind(path)
        .bind(purpose)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Err(ProjectError::NotFound(format!(
                "位置 {id} 不存在——先 project-get <项目> 看 locations 列表取 id"
            )));
        }
        self.get_location(id).await
    }

    pub async fn delete_location(&self, id: Uuid) -> Result<bool, ProjectError> {
        let res = sqlx::query("DELETE FROM project_locations WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(ProjectError::NotFound(format!(
                "位置 {id} 不存在——先 project-get <项目> 看 locations 列表取 id"
            )));
        }
        Ok(true)
    }

    pub async fn get_location(&self, id: Uuid) -> Result<ProjectLocationDto, ProjectError> {
        sqlx::query_as::<_, ProjectLocationDto>(&format!(
            "SELECT {LOCATION_COLS} FROM project_locations WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            ProjectError::NotFound(format!(
                "位置 {id} 不存在——先 project-get <项目> 看 locations 列表取 id"
            ))
        })
    }

    // ---------- 分类文档 ----------

    /// 分类校验错误（列出项目现有分类，提示先扩 categories）。
    fn category_error(category: &str, categories: &[String]) -> ProjectError {
        let existing = if categories.is_empty() {
            "项目还没有分类".to_string()
        } else {
            categories.join("、")
        };
        ProjectError::BadRequest(format!(
            "分类「{category}」不在项目分类里（现有：{existing}）——要新分类就先 update_project 把它加进 categories"
        ))
    }

    pub async fn add_doc(
        &self,
        project_id: Uuid,
        category: &str,
        title: &str,
        content: &str,
    ) -> Result<ProjectDocDto, ProjectError> {
        let project = self.get_project_bare(project_id).await?;
        if !project.categories.iter().any(|c| c == category) {
            return Err(Self::category_error(category, &project.categories));
        }
        let id = Uuid::now_v7();
        let res = sqlx::query(
            "INSERT INTO project_docs (id, project_id, category, title, content) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (project_id, category, title) DO NOTHING",
        )
        .bind(id)
        .bind(project_id)
        .bind(category)
        .bind(title)
        .bind(content)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Err(ProjectError::Conflict(format!(
                "文档「{title}」在分类「{category}」下已存在——同项目同分类 title 唯一，请 doc-update 已有文档或改 title"
            )));
        }
        self.get_doc(id).await
    }

    pub async fn update_doc(
        &self,
        id: Uuid,
        category: &str,
        title: &str,
        content: &str,
    ) -> Result<ProjectDocDto, ProjectError> {
        // 分类只在「换到别的分类」时校验——分类被项目方移除后，存量文档仍可原地编辑
        let current = self.get_doc(id).await?;
        if category != current.category {
            let project = self.get_project_bare(current.project_id).await?;
            if !project.categories.iter().any(|c| c == category) {
                return Err(Self::category_error(category, &project.categories));
            }
        }
        let res = sqlx::query(
            "UPDATE project_docs SET category = $2, title = $3, content = $4, updated_at = now() \
             WHERE id = $1",
        )
        .bind(id)
        .bind(category)
        .bind(title)
        .bind(content)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Err(ProjectError::NotFound(format!(
                "文档 {id} 不存在——先 project-get <项目> 看 docs 列表取 id"
            )));
        }
        self.get_doc(id).await
    }

    pub async fn delete_doc(&self, id: Uuid) -> Result<bool, ProjectError> {
        let res = sqlx::query("DELETE FROM project_docs WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(ProjectError::NotFound(format!(
                "文档 {id} 不存在——先 project-get <项目> 看 docs 列表取 id"
            )));
        }
        Ok(true)
    }

    pub async fn get_doc(&self, id: Uuid) -> Result<ProjectDocDto, ProjectError> {
        sqlx::query_as::<_, ProjectDocDto>(&format!(
            "SELECT {DOC_COLS} FROM project_docs WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            ProjectError::NotFound(format!(
                "文档 {id} 不存在——先 project-get <项目> 看 docs 列表取 id"
            ))
        })
    }

    // ---------- 精确寻址读（行号定位，无截断） ----------

    /// 按行区间读文档（1-based、含两端；None = 从头/到尾）。
    /// 返回 (总行数, [(行号, 行文本)])。行号基于当前版本，改文档后需重取。
    pub async fn read_doc_lines(
        &self,
        id: Uuid,
        start: Option<i64>,
        end: Option<i64>,
    ) -> Result<(i64, Vec<(i64, String)>), ProjectError> {
        let start = start.unwrap_or(1);
        let end = end.unwrap_or(i64::MAX);
        if start < 1 {
            return Err(ProjectError::BadRequest(format!(
                "start_line 从 1 开始，收到 {start}"
            )));
        }
        if start > end {
            return Err(ProjectError::BadRequest(format!(
                "start_line({start}) 不能大于 end_line({end})"
            )));
        }
        let doc = self.get_doc(id).await?;
        let total = doc.content.lines().count() as i64;
        let lines = doc
            .content
            .lines()
            .enumerate()
            .skip_while(|(i, _)| (*i as i64) < start - 1)
            .take_while(|(i, _)| (*i as i64) < end)
            .map(|(i, text)| (i as i64 + 1, text.to_string()))
            .collect();
        Ok((total, lines))
    }

    /// grep 式跨文档按行检索（大小写不敏感子串），返回命中行号 + 原文行。
    /// 定位到行号后用 read_doc_lines / project_doc_get 区间精读。
    pub async fn search_doc_lines(
        &self,
        project_id: Uuid,
        query: &str,
        limit: i64,
    ) -> Result<Vec<DocLineHitDto>, ProjectError> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Err(ProjectError::BadRequest("检索词不能为空".to_string()));
        }
        self.get_project_bare(project_id).await?;
        let docs: Vec<(Uuid, String, String, String)> =
            sqlx::query_as("SELECT id, title, category, content FROM project_docs WHERE project_id = $1 ORDER BY category, created_at")
                .bind(project_id)
                .fetch_all(&self.pool)
                .await?;
        let mut hits = Vec::new();
        let cap = limit.clamp(1, 500) as usize;
        for (id, title, category, content) in docs {
            for (i, line) in content.lines().enumerate() {
                if hits.len() >= cap {
                    return Ok(hits);
                }
                if line.to_lowercase().contains(&q) {
                    hits.push(DocLineHitDto {
                        doc_id: id,
                        title: title.clone(),
                        category: category.clone(),
                        line: i as i64 + 1,
                        text: line.to_string(),
                    });
                }
            }
        }
        Ok(hits)
    }
}

//! 项目记忆域服务：项目 / 多主机位置 / 分类文档的三表 CRUD + 类型模板。
//!
//! 设计：docs/plantree/plans/project-memory/（0005 三表模型、类型=分类模板）。
//! 类型模板是代码常量（三表决策不建第四表），新建项目时复制进 projects.categories，
//! 之后项目级自由增删。
//!
//! 持久化在 `engram_storage::repo::project`（本文件只保留校验、冲突语义与编排）。

use chrono::{DateTime, Utc};
use engram_storage::repo::project as repo;
use engram_storage::{PgPool, StoreError};
use serde::Serialize;
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

impl From<StoreError> for ProjectError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Conflict(_) => ProjectError::Conflict("唯一约束冲突".into()),
            StoreError::Sql(e) => ProjectError::Storage(e.to_string()),
        }
    }
}

/// 类型模板：type → 预设分类列表（0005：开发四分类 / 调研六分类）。
pub const PROJECT_TYPES: &[(&str, &[&str])] = &[
    ("dev", &["后端", "前端", "测试", "部署", "规划"]),
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

// ---------- DTO（持久化模型在 storage，此处 re-export 保持路径兼容） ----------

pub use engram_storage::models::project::{
    ProjectDocDto, ProjectDto, ProjectFileDto, ProjectLocationDto,
};

/// 轻归一化：lowercase + 空白折叠（保留标点——整词/行首判定的边界来源）。
fn light_normalize(s: &str) -> String {
    s.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// 整词命中判定（仅纯 ASCII 词——中文无词边界概念跳过）：
/// 归一化行中该词左右边界均非 ASCII 字母数字。
fn is_whole_word_hit(norm_line: &str, term: &str) -> bool {
    if !term.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return false;
    }
    match norm_line.find(term) {
        Some(pos) => {
            let bytes = norm_line.as_bytes();
            let before_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
            let after = pos + term.len();
            let after_ok = after >= bytes.len() || !bytes[after].is_ascii_alphanumeric();
            before_ok && after_ok
        }
        None => false,
    }
}

/// 检索归一化：lowercase + 空白/中英标点忽略（与 web Galaxy 页的重复实体归一化规则一致）——
/// 「WorkBuddy」与「Work Buddy」、「部署。」与「部署」互相可召回。
fn normalize_for_search(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| {
            !c.is_whitespace() && !"·.-_()（）【】《》，,、。：:；;！!？?\"'`~".contains(*c)
        })
        .collect()
}

/// 文档行检索命中（grep 式定位：行号 + 原文行；配合 read_doc_lines 区间精读）。
/// 0042 检索升级后按文档相关性聚合排序：score = 文档评分（title 加权 + 命中密度），
/// doc_hit_count = 该文档内命中行数——AI 可据此先读高分文档。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DocLineHitDto {
    pub doc_id: Uuid,
    pub title: String,
    pub category: String,
    /// 1-based 行号（基于文档当前版本）
    pub line: i64,
    pub text: String,
    /// 文档相关性评分（0042）
    #[serde(default)]
    pub score: i64,
    /// 该文档内命中行数（0042）
    #[serde(default)]
    pub doc_hit_count: i64,
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

    async fn get_project_bare(&self, id: Uuid) -> Result<ProjectDto, ProjectError> {
        repo::get_project(&self.pool, id).await?.ok_or_else(|| {
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

    /// folder 路径校验：/ 分隔、禁 .. 与绝对路径/反斜杠/冒号、≤200 字符；'' = 分类根下。
    /// 项目文件名校验：非空、≤200 字符、禁路径分隔与控制字符（0045 项目文件）
    fn validate_file_name(name: &str) -> Result<String, ProjectError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ProjectError::BadRequest("文件名不能为空".into()));
        }
        if name.chars().count() > 200 {
            return Err(ProjectError::BadRequest("文件名过长（>200 字符）".into()));
        }
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return Err(ProjectError::BadRequest(
                "文件名不允许路径分隔符 / \\ 或 ..".into(),
            ));
        }
        Ok(name.to_string())
    }

    /// MIME 推断：按扩展名（.html→text/html、.md→text/markdown、.svg→image/svg+xml、
    /// .json→application/json、.css/.js→text/*，其余 text/plain）
    fn infer_mime(name: &str) -> &'static str {
        let lower = name.to_lowercase();
        match lower.rsplit('.').next().unwrap_or("") {
            "html" | "htm" => "text/html",
            "md" | "markdown" => "text/markdown",
            "svg" => "image/svg+xml",
            "json" => "application/json",
            "css" => "text/css",
            "js" => "text/javascript",
            "csv" => "text/csv",
            _ => "text/plain",
        }
    }

    /// 项目文件大小门禁：8MB（架构图 HTML 实测 13K；门禁防误传超大 blob）
    const FILE_MAX_BYTES: usize = 8 * 1024 * 1024;

    pub async fn upsert_file(
        &self,
        project_id: Uuid,
        name: &str,
        mime: Option<&str>,
        content: &str,
    ) -> Result<ProjectFileDto, ProjectError> {
        let name = Self::validate_file_name(name)?;
        if content.len() > Self::FILE_MAX_BYTES {
            return Err(ProjectError::BadRequest(format!(
                "文件过大（{} bytes > 8MB 上限）",
                content.len()
            )));
        }
        let _ = self.get_project_bare(project_id).await?; // 存在性门禁
        let mime = mime
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| Self::infer_mime(&name).to_string());
        let dto = repo::upsert_file(&self.pool, project_id, &name, &mime, content).await?;
        Ok(dto)
    }

    pub async fn list_files(&self, project_id: Uuid) -> Result<Vec<ProjectFileDto>, ProjectError> {
        let _ = self.get_project_bare(project_id).await?;
        Ok(repo::list_files(&self.pool, project_id).await?)
    }

    pub async fn get_file(
        &self,
        project_id: Uuid,
        name: &str,
    ) -> Result<ProjectFileDto, ProjectError> {
        repo::get_file_by_name(&self.pool, project_id, name)
            .await?
            .ok_or_else(|| ProjectError::NotFound(format!("项目文件「{name}」不存在")))
    }

    pub async fn delete_file(&self, project_id: Uuid, name: &str) -> Result<u64, ProjectError> {
        let n = repo::delete_file(&self.pool, project_id, name).await?;
        if n == 0 {
            return Err(ProjectError::NotFound(format!("项目文件「{name}」不存在")));
        }
        Ok(n)
    }

    pub async fn list_file_versions(
        &self,
        project_id: Uuid,
        name: &str,
    ) -> Result<Vec<(i32, DateTime<Utc>)>, ProjectError> {
        let f = self.get_file(project_id, name).await?;
        Ok(repo::list_file_versions(&self.pool, f.id).await?)
    }

    pub async fn get_file_version(
        &self,
        project_id: Uuid,
        name: &str,
        version: i32,
    ) -> Result<String, ProjectError> {
        let f = self.get_file(project_id, name).await?;
        repo::get_file_version(&self.pool, f.id, version)
            .await?
            .ok_or_else(|| ProjectError::NotFound(format!("文件「{name}」不存在版本 v{version}")))
    }

    fn validate_doc_folder(folder: &str) -> Result<String, ProjectError> {
        let f = folder.trim().trim_matches('/');
        if f.is_empty() {
            return Ok(String::new());
        }
        if f.len() > 200 {
            return Err(ProjectError::BadRequest("folder 过长（>200 字符）".into()));
        }
        if f.contains('\\') || f.contains(':') {
            return Err(ProjectError::BadRequest(
                "folder 用 / 分隔；不允许反斜杠/盘符冒号".into(),
            ));
        }
        if f.split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
        {
            return Err(ProjectError::BadRequest(
                "folder 含空段或 . / .. 段——用规范相对路径（如 审计、归档/ai-permissions）".into(),
            ));
        }
        Ok(f.to_string())
    }

    pub async fn add_doc(
        &self,
        project_id: Uuid,
        category: &str,
        folder: &str,
        title: &str,
        content: &str,
    ) -> Result<ProjectDocDto, ProjectError> {
        let project = self.get_project_bare(project_id).await?;
        if !project.categories.iter().any(|c| c == category) {
            return Err(Self::category_error(category, &project.categories));
        }
        let folder = Self::validate_doc_folder(folder)?;
        let id = Uuid::now_v7();
        let inserted = repo::insert_doc(
            &self.pool, id, project_id, category, &folder, title, content,
        )
        .await?;
        if inserted == 0 {
            return Err(ProjectError::Conflict(format!(
                "文档「{title}」在分类「{category}」的 folder「{folder}」下已存在——同路径 title 唯一，请 doc-update 已有文档或改 title"
            )));
        }
        self.get_doc(id).await
    }

    /// 部分更新：None 字段保持原值（并发安全——SQL 层 COALESCE，无读-改-写窗口）。
    pub async fn update_doc(
        &self,
        id: Uuid,
        category: Option<&str>,
        folder: Option<&str>,
        title: Option<&str>,
        content: Option<&str>,
        expected_version: Option<i64>,
    ) -> Result<ProjectDocDto, ProjectError> {
        // 分类只在「换到别的分类」时校验——分类被项目方移除后，存量文档仍可原地编辑
        let current = self.get_doc(id).await?;
        // 乐观锁预检（公网多Agent P001 步骤2）：带 expected_version 且与当前不符 → 409。
        // SQL 层还有原子守卫（WHERE version = $6），预检只为给出更可读的错误。
        if let Some(ev) = expected_version
            && ev != current.version
        {
            return Err(ProjectError::Conflict(format!(
                "版本冲突：文档当前 version={}，请求基于 {}——先 doc_get 取最新版本与行号再改",
                current.version, ev
            )));
        }
        if let Some(category) = category
            && category != current.category
        {
            let project = self.get_project_bare(current.project_id).await?;
            if !project.categories.iter().any(|c| c == category) {
                return Err(Self::category_error(category, &project.categories));
            }
        }
        let folder = match folder {
            Some(f) => Some(Self::validate_doc_folder(f)?),
            None => None,
        };
        let updated = repo::update_doc(
            &self.pool,
            id,
            category,
            folder.as_deref(),
            title,
            content,
            expected_version,
        )
        .await?;
        if updated == 0 {
            // 并发竞态兜底：get_doc 与 UPDATE 之间版本被改（SQL 内 version 守卫拦截）
            if expected_version.is_some() {
                return Err(ProjectError::Conflict(
                    "版本冲突：文档版本已被并发修改——先 doc_get 取最新版本再改".into(),
                ));
            }
            return Err(ProjectError::NotFound(format!(
                "文档 {id} 不存在——先 project-get <项目> 看 docs 列表取 id"
            )));
        }
        self.get_doc(id).await
    }

    pub async fn delete_doc(&self, id: Uuid) -> Result<bool, ProjectError> {
        let deleted = repo::delete_doc(&self.pool, id).await?;
        if deleted == 0 {
            return Err(ProjectError::NotFound(format!(
                "文档 {id} 不存在——先 project-get <项目> 看 docs 列表取 id"
            )));
        }
        Ok(true)
    }

    pub async fn get_doc(&self, id: Uuid) -> Result<ProjectDocDto, ProjectError> {
        repo::get_doc(&self.pool, id).await?.ok_or_else(|| {
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
        // 0042 检索升级：评分制多词行检索（工单「文档检索可用性」）。
        // 旧实现逐行 substring 顺序截断——零相关性排序，宽泛词首屏全泡在一篇长文里。
        let terms: Vec<String> = query
            .split_whitespace()
            .map(normalize_for_search)
            .filter(|t| !t.is_empty())
            .collect();
        if terms.is_empty() {
            return Err(ProjectError::BadRequest("检索词不能为空".to_string()));
        }
        self.get_project_bare(project_id).await?;
        let docs = repo::list_doc_contents(&self.pool, project_id).await?;

        struct DocHits {
            doc_id: Uuid,
            title: String,
            category: String,
            doc_score: i64,
            lines: Vec<(i64, String, i64)>, // (行号, 原文, 行分)
        }
        let mut results: Vec<DocHits> = Vec::new();
        let cap = limit.clamp(1, 500) as usize;

        for (id, title, category, content) in docs {
            let title_norm = normalize_for_search(&title);
            // 文档级：title 命中加权（title 是文档最强信号）
            let title_score: i64 = terms
                .iter()
                .map(|t| {
                    if title_norm.contains(t.as_str()) {
                        50
                    } else {
                        0
                    }
                })
                .sum();
            let mut lines: Vec<(i64, String, i64)> = Vec::new();
            for (i, line) in content.lines().enumerate() {
                let line_norm = normalize_for_search(line);
                // 轻归一化版保留标点边界——整词/行首判定用
                let line_light = light_normalize(line);
                // 行分：命中词 +3/词；行首命中 +5；整词命中 +4；全词共现 +10
                let mut hit_terms = 0usize;
                let mut line_score = 0i64;
                for t in &terms {
                    if line_norm.contains(t.as_str()) {
                        hit_terms += 1;
                        line_score += 3;
                        if line_light.starts_with(t.as_str()) {
                            line_score += 5;
                        }
                        if is_whole_word_hit(&line_light, t) {
                            line_score += 4;
                        }
                    }
                }
                if hit_terms > 0 {
                    if hit_terms == terms.len() && terms.len() > 1 {
                        line_score += 10;
                    }
                    lines.push((i as i64 + 1, line.to_string(), line_score));
                }
            }
            if lines.is_empty() {
                continue;
            }
            // 文档分 = title 加权 + 命中密度（行数 × 5）+ 行分总和
            let doc_score =
                title_score + lines.len() as i64 * 5 + lines.iter().map(|l| l.2).sum::<i64>();
            results.push(DocHits {
                doc_id: id,
                title,
                category,
                doc_score,
                lines,
            });
        }

        // 排序：文档按 doc_score 降序，文档内行按行分降序 + 行号升序
        results.sort_by_key(|d| std::cmp::Reverse(d.doc_score));
        let mut hits = Vec::new();
        'outer: for mut d in results {
            d.lines.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
            let doc_hit_count = d.lines.len() as i64;
            for (line, text, line_score) in d.lines {
                if hits.len() >= cap {
                    break 'outer;
                }
                hits.push(DocLineHitDto {
                    doc_id: d.doc_id,
                    title: d.title.clone(),
                    category: d.category.clone(),
                    line,
                    text,
                    score: d.doc_score,
                    doc_hit_count,
                });
                let _ = line_score;
            }
        }
        Ok(hits)
    }

    /// 行级补丁（R 报告 P1-10）：改长文档不再「取全文→重发全文」。
    /// mode=replace（默认）：[start_line, end_line]（1-based 含两端）替换为 content（必填）；
    /// mode=insert：在 start_line 行**之前**插入 content（必填），start_line 允许 total+1（追加到末尾）；
    /// mode=delete：删除 [start_line, end_line]，content 忽略。
    /// 返回更新后的文档（正文完整——调用方按需取字段）。
    pub async fn patch_doc(
        &self,
        id: Uuid,
        start_line: i64,
        end_line: i64,
        mode: &str,
        content: Option<&str>,
        expected_version: Option<i64>,
    ) -> Result<ProjectDocDto, ProjectError> {
        let doc = self.get_doc(id).await?;
        if start_line < 1 {
            return Err(ProjectError::BadRequest(format!(
                "start_line 从 1 开始（收到 {start_line}）"
            )));
        }
        // split 保留结尾空元素：原文以 \n 结尾时 split 出末尾空串，重组后 newline 语义不丢
        let mut lines: Vec<String> = doc.content.split('\n').map(str::to_string).collect();
        let real_total = lines.len() as i64; // 以 \n 结尾的文档 real_total = total + 1（末尾空串）
        let bounded_total = if doc.content.is_empty() {
            0
        } else {
            real_total
        };
        match mode {
            "replace" => {
                let Some(text) = content else {
                    return Err(ProjectError::BadRequest(
                        "mode=replace 需要传 content（替换后的文本，可多行）".into(),
                    ));
                };
                if end_line < start_line {
                    return Err(ProjectError::BadRequest(format!(
                        "end_line({end_line}) 不能小于 start_line({start_line})"
                    )));
                }
                if end_line > bounded_total {
                    return Err(ProjectError::BadRequest(format!(
                        "end_line({end_line}) 超出文档总行数（{bounded_total}）——先 doc_get 确认行号"
                    )));
                }
                let replacement: Vec<String> = text.split('\n').map(str::to_string).collect();
                let pos = (start_line - 1) as usize;
                lines.splice(pos..(end_line as usize), replacement);
            }
            "insert" => {
                let Some(text) = content else {
                    return Err(ProjectError::BadRequest(
                        "mode=insert 需要传 content（插入的文本，可多行）".into(),
                    ));
                };
                if start_line > bounded_total + 1 {
                    return Err(ProjectError::BadRequest(format!(
                        "start_line({start_line}) 超界——插入允许 1..={}（total+1 = 追加到末尾）",
                        bounded_total + 1
                    )));
                }
                let insertion: Vec<String> = text.split('\n').map(str::to_string).collect();
                let pos = (start_line - 1) as usize;
                lines.splice(pos..pos, insertion);
            }
            "delete" => {
                if end_line < start_line {
                    return Err(ProjectError::BadRequest(format!(
                        "end_line({end_line}) 不能小于 start_line({start_line})"
                    )));
                }
                if end_line > bounded_total {
                    return Err(ProjectError::BadRequest(format!(
                        "end_line({end_line}) 超出文档总行数（{bounded_total}）"
                    )));
                }
                lines.drain((start_line - 1) as usize..(end_line as usize));
            }
            other => {
                return Err(ProjectError::BadRequest(format!(
                    "mode 只支持 replace/insert/delete（收到 {other:?}）"
                )));
            }
        }
        let patched = lines.join("\n");
        self.update_doc(id, None, None, None, Some(&patched), expected_version)
            .await
    }
}

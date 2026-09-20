//! `project` 的实现切片（架构治理 2026-09-21：自 project.rs 纯搬移，零行为变化）。

use super::*;

impl ProjectService {
    /// folder 路径校验：/ 分隔、禁 .. 与绝对路径/反斜杠/冒号、≤200 字符；'' = 分类根下。
    /// 项目文件名校验：非空、≤200 字符、禁路径分隔与控制字符（0045 项目文件）
    pub(crate) fn validate_file_name(name: &str) -> Result<String, ProjectError> {
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
    pub(crate) fn infer_mime(name: &str) -> &'static str {
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
        let _ = self.get_project_bare(project_id).await?; // 有意忽略：只借其错误通道做存在性门禁，Project 值本处不用
        let mime = mime
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| Self::infer_mime(&name).to_string());
        let dto = repo::upsert_file(&self.pool, project_id, &name, &mime, content).await?;
        Ok(dto)
    }

    pub async fn list_files(&self, project_id: Uuid) -> Result<Vec<ProjectFileDto>, ProjectError> {
        let _ = self.get_project_bare(project_id).await?; // 有意忽略：存在性门禁（值本处不用）
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
}

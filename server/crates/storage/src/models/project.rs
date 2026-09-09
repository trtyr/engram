//! 项目记忆域行类型（三表：projects / project_locations / project_docs）。

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

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
    /// 子文件夹相对路径（/ 分隔，'' = 分类根下；树形呈现 = category → folder → 文档）
    pub folder: String,
    pub title: String,
    pub content: String,
    #[schema(value_type = Object)]
    pub frontmatter: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

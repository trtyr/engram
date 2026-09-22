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

/// 项目文件（非 markdown 制品：架构图 HTML / 配置样例 / 导出报告；0045）
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ProjectFileDto {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    /// MIME 类型；渲染契约：text/html → iframe sandbox 查看器，text/markdown → WikiMarkdown，其余 <pre>
    pub mime: String,
    pub content: String,
    pub version: i32,
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
    /// 指向资产台账条目的**真引用**（0058；NULL = 尚未归一到资产）。
    /// 唯一事实源铁律：`host` 是显示用文本，身份以 `assets` 为准（见《项目与资产模型 · README》§2.4）。
    pub asset_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ProjectLinkDto {
    pub id: Uuid,
    /// 关联起点（`part_of` 语义下 = 子方）
    pub from_project: Uuid,
    pub from_name: String,
    /// 关联终点（`part_of` 语义下 = 母方）
    pub to_project: Uuid,
    pub to_name: String,
    /// `part_of` 隶属 / `related` 相关（值域事实源 = core 的 `PROJECT_LINK_KINDS`）
    pub kind: String,
    pub note: String,
    pub created_at: DateTime<Utc>,
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
    /// 乐观锁版本：doc_update/patch 成功 +1；写入方可带 expected_version 检测陈旧
    pub version: i64,
}

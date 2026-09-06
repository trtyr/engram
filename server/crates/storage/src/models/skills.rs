//! 技能域行类型（skills / skill_revisions）。

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// 列表/导入/概览用摘要（不含正文——列表与仪表盘不必拖全量指令）。
#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct SkillSummaryDto {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub enabled: bool,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 详情（含 markdown 正文）。
#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct SkillDto {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub content: String,
    pub tags: Vec<String>,
    pub enabled: bool,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 版本快照。
#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct SkillRevisionDto {
    pub id: Uuid,
    pub skill_id: Uuid,
    pub rev: i32,
    pub name: String,
    pub description: String,
    pub content: String,
    pub tags: Vec<String>,
    /// create=初始版 / update=变更前快照 / restore=回滚前的现状快照
    pub origin: String,
    pub created_at: DateTime<Utc>,
}

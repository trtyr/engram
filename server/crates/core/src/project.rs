//! 项目记忆域服务：项目 / 多主机位置 / 分类文档的三表 CRUD + 类型模板。
//!
//! 设计：docs/plantree/plans/project-memory/（0005 三表模型、类型=分类模板）。
//! 类型模板是代码常量（三表决策不建第四表），新建项目时复制进 projects.categories，
//! 之后项目级自由增删。
//!
//! 持久化在 `engram_storage::repo::project`（本文件只保留校验、冲突语义与编排）。

mod docs;
mod files;
mod model;
mod projects;
pub use model::*;

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

/// 项目状态枚举（英文存库，Web 层映射中文）：active/paused/done/abandoned。
pub const PROJECT_STATUSES: &[&str] = &["active", "paused", "done", "abandoned"];

/// 类型（场景）显示名（Web 用）。
pub fn type_label(type_: &str) -> String {
    match type_ {
        "dev" => "开发".into(),
        "ops" => "运维".into(),
        "research" => "调研".into(),
        "study" => "学习".into(),
        "life" => "生活".into(),
        "create" => "创作".into(),
        other => other.into(),
    }
}

// ---------- DTO（持久化模型在 storage，此处 re-export 保持路径兼容） ----------

pub use engram_storage::models::project::{
    ProjectDocDto, ProjectDto, ProjectFileDto, ProjectLocationDto,
};

// ---------- Service ----------

pub struct ProjectService {
    pool: PgPool,
}

impl ProjectService {
    // ---------- 项目 CRUD ----------

    // ---------- 位置（多主机） ----------

    // ---------- 分类文档 ----------

    /// 项目文件大小门禁：8MB（架构图 HTML 实测 13K；门禁防误传超大 blob）
    const FILE_MAX_BYTES: usize = 8 * 1024 * 1024;

    // ---------- 精确寻址读（行号定位，无截断） ----------
}

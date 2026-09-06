//! 待办域服务（第七域）：不绑定项目的临时任务/灵感速记。
//!
//! 定位：轻量「速记 → 做完勾掉」，非工单（无指派/SLA/流程）。
//! 场景靠 tags + priority + due_at + project_hint（纯文本提示，不做 FK 绑定）表达。

use chrono::{DateTime, Utc};
use engram_storage::repo::todos as repo;
use engram_storage::{PgPool, StoreError};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum TodoError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("存储暂时不可用: {0}")]
    Storage(String),
}

impl From<StoreError> for TodoError {
    fn from(e: StoreError) -> Self {
        TodoError::Storage(e.to_string())
    }
}

pub const STATUSES: &[&str] = &["open", "done", "archived"];
pub const PRIORITIES: &[&str] = &["low", "normal", "high"];

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct TodoDto {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    pub status: String,
    pub priority: String,
    pub tags: Vec<String>,
    pub due_at: Option<DateTime<Utc>>,
    pub project_hint: Option<String>,
    pub done_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn to_dto(t: engram_storage::repo::todos::TodoTuple) -> TodoDto {
    TodoDto {
        id: t.0,
        title: t.1,
        body: t.2,
        status: t.3,
        priority: t.4,
        tags: t.5,
        due_at: t.6,
        project_hint: t.7,
        done_at: t.8,
        created_at: t.9,
        updated_at: t.10,
    }
}

pub struct TodoService {
    pool: PgPool,
}

impl TodoService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    fn validate_priority(priority: &str) -> Result<(), TodoError> {
        if !PRIORITIES.contains(&priority) {
            return Err(TodoError::BadRequest(format!(
                "priority 仅接受 {}（收到 {priority}）",
                PRIORITIES.join("/")
            )));
        }
        Ok(())
    }

    /// 新建待办。
    pub async fn create(
        &self,
        title: &str,
        body: &str,
        priority: &str,
        tags: &[String],
        due_at: Option<DateTime<Utc>>,
        project_hint: Option<&str>,
    ) -> Result<TodoDto, TodoError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(TodoError::BadRequest("title 不能为空".into()));
        }
        if title.chars().count() > 200 {
            return Err(TodoError::BadRequest("title 过长（>200 字符）".into()));
        }
        Self::validate_priority(priority)?;
        let id = Uuid::now_v7();
        repo::insert(
            &self.pool,
            &engram_storage::repo::todos::NewTodo {
                id,
                title,
                body: body.trim(),
                priority,
                tags,
                due_at,
                project_hint: project_hint.map(str::trim).filter(|s| !s.is_empty()),
            },
        )
        .await?;
        self.get(id).await
    }

    /// 列表：open 优先；status/priority/tag/q 过滤。
    pub async fn list(
        &self,
        status: Option<&str>,
        priority: Option<&str>,
        tag: Option<&str>,
        q: Option<&str>,
        limit: i64,
    ) -> Result<Vec<TodoDto>, TodoError> {
        if let Some(s) = status
            && !STATUSES.contains(&s)
        {
            return Err(TodoError::BadRequest(format!(
                "status 仅接受 {}（收到 {s}）",
                STATUSES.join("/")
            )));
        }
        if let Some(p) = priority
            && !PRIORITIES.contains(&p)
        {
            return Err(TodoError::BadRequest(format!(
                "priority 仅接受 {}（收到 {p}）",
                PRIORITIES.join("/")
            )));
        }
        Ok(
            repo::list(&self.pool, status, priority, tag, q, limit.min(500))
                .await?
                .into_iter()
                .map(to_dto)
                .collect(),
        )
    }

    pub async fn get(&self, id: Uuid) -> Result<TodoDto, TodoError> {
        repo::get(&self.pool, id)
            .await?
            .map(to_dto)
            .ok_or_else(|| TodoError::NotFound(format!("待办 {id} 不存在")))
    }

    /// 更新（部分字段，None 不动）。
    #[allow(clippy::too_many_arguments)]
    pub async fn update(
        &self,
        id: Uuid,
        title: Option<&str>,
        body: Option<&str>,
        priority: Option<&str>,
        status: Option<&str>,
        due_at: Option<Option<DateTime<Utc>>>,
        project_hint: Option<Option<&str>>,
        tags: Option<&[String]>,
    ) -> Result<TodoDto, TodoError> {
        if let Some(t) = title {
            let t = t.trim();
            if t.is_empty() {
                return Err(TodoError::BadRequest("title 不能为空".into()));
            }
        }
        if let Some(p) = priority {
            Self::validate_priority(p)?;
        }
        if let Some(s) = status
            && !STATUSES.contains(&s)
        {
            return Err(TodoError::BadRequest(format!(
                "status 仅接受 {}（收到 {s}）",
                STATUSES.join("/")
            )));
        }
        let n = repo::update(
            &self.pool,
            id,
            &engram_storage::repo::todos::TodoPatch {
                title,
                body,
                priority,
                status,
                due_at,
                project_hint,
                tags,
            },
        )
        .await?;
        if n == 0 {
            return Err(TodoError::NotFound(format!("待办 {id} 不存在")));
        }
        self.get(id).await
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), TodoError> {
        let n = repo::delete(&self.pool, id).await?;
        if n == 0 {
            return Err(TodoError::NotFound(format!("待办 {id} 不存在")));
        }
        Ok(())
    }

    /// 全量导出（P4 数据主权）。
    pub async fn export_all(&self) -> Result<Vec<TodoDto>, TodoError> {
        Ok(repo::export_all(&self.pool)
            .await?
            .into_iter()
            .map(to_dto)
            .collect())
    }
}

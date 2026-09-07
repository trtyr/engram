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

    /// NUL 字节拒绝（D20）：PG UTF8 层对 \0 直接报编码错误——裸漏成「存储暂时不可用」，
    /// 在入参层响亮拒绝。
    fn reject_nul(field: &str, value: &str) -> Result<(), TodoError> {
        if value.contains('\0') {
            return Err(TodoError::BadRequest(format!(
                "{field} 含非法控制字符（NUL）"
            )));
        }
        Ok(())
    }

    /// tag 规整：trim + 丢弃空串（观察项：空字符串 tag 无意义还污染过滤面）。
    fn normalize_tags(tags: &[String]) -> Vec<String> {
        tags.iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
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
        Self::reject_nul("title", title)?;
        Self::reject_nul("body", body)?;
        let tags = Self::normalize_tags(tags);
        for t in &tags {
            Self::reject_nul("tags", t)?;
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
                tags: &tags,
                due_at,
                project_hint: project_hint.map(str::trim).filter(|s| !s.is_empty()),
            },
        )
        .await?;
        self.get(id).await
    }

    /// 列表：open 优先；status/priority/tag/q 过滤。
    /// cursor（D29 keyset 分页，单页上限 500）：上一页最后一条的
    /// `{1|0}|{updated_at ISO8601}|{id}`——1 表示该条 status=open。首查不传。
    pub async fn list(
        &self,
        status: Option<&str>,
        priority: Option<&str>,
        tag: Option<&str>,
        q: Option<&str>,
        cursor: Option<&str>,
        limit: i64,
    ) -> Result<Vec<TodoDto>, TodoError> {
        if limit < 0 {
            return Err(TodoError::BadRequest(format!(
                "limit 不能为负（收到 {limit}）"
            )));
        }
        let cursor = match cursor {
            None | Some("") => None,
            Some(raw) => {
                let parts: Vec<&str> = raw.split('|').collect();
                if parts.len() != 3 {
                    return Err(TodoError::BadRequest(format!(
                        "cursor 非法（收到 {raw:?}）——期望 {{1|0}}|{{updated_at ISO8601}}|{{id}}，取上一页最后一条构造"
                    )));
                }
                let flag = parts[0].parse::<i32>().ok().filter(|f| *f == 0 || *f == 1);
                let ts = chrono::DateTime::parse_from_rfc3339(parts[1].trim())
                    .map(|d| d.with_timezone(&Utc))
                    .ok();
                let id = Uuid::parse_str(parts[2].trim()).ok();
                match (flag, ts, id) {
                    (Some(flag), Some(ts), Some(id)) => Some((flag, ts, id)),
                    _ => {
                        return Err(TodoError::BadRequest(format!(
                            "cursor 非法（收到 {raw:?}）——期望 {{1|0}}|{{updated_at ISO8601}}|{{id}}，取上一页最后一条构造"
                        )));
                    }
                }
            }
        };
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
            repo::list(&self.pool, status, priority, tag, q, cursor, limit.min(500))
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
            Self::reject_nul("title", t)?;
        }
        if let Some(b) = body {
            Self::reject_nul("body", b)?;
        }
        let tags = tags.map(Self::normalize_tags);
        if let Some(ts) = &tags {
            for t in ts {
                Self::reject_nul("tags", t)?;
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
                tags: tags.as_deref(),
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

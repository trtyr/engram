//! 待办域服务（0074 工单拆表后回归轻量定位）：
//! todo = 微软式行动项——速记→做完勾掉，tags + priority + due_at 表达场景。
//! 工单在 core::tickets（项目绑定制，独立生命周期）。

use chrono::{DateTime, Utc};
use engram_storage::StoreError;
use engram_storage::repo::todos as repo;
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
    pub done_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// 全局单调短号（显示为 EN-<n>；人类可读引用）
    pub short_no: i32,
}

fn to_dto(t: repo::TodoRow) -> TodoDto {
    TodoDto {
        id: t.id,
        title: t.title,
        body: t.body,
        status: t.status,
        priority: t.priority,
        tags: t.tags,
        due_at: t.due_at,
        done_at: t.done_at,
        created_at: t.created_at,
        updated_at: t.updated_at,
        short_no: t.short_no,
    }
}

/// 解析 keyset 游标：`{1|0}|{updated_at RFC3339}|{id}`（取上一页最后一条构造）。
fn parse_todo_cursor(raw: Option<&str>) -> Result<Option<(i32, DateTime<Utc>, Uuid)>, TodoError> {
    let cursor = match raw {
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
    Ok(cursor)
}

pub struct TodoService {
    pool: engram_storage::PgPool,
}

impl TodoService {
    pub fn new(pool: engram_storage::PgPool) -> Self {
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

    /// 按引用取 todo：支持完整 UUID 或短号形式「EN-<n>」。
    pub async fn find_by_ref(&self, r: &str) -> Result<Option<TodoDto>, TodoError> {
        let r = r.trim();
        if let Ok(id) = Uuid::parse_str(r) {
            return Ok(repo::get(&self.pool, id).await?.map(to_dto));
        }
        let n = r
            .strip_prefix("EN-")
            .or_else(|| r.strip_prefix("en-"))
            .and_then(|n| n.parse::<i32>().ok())
            .ok_or_else(|| {
                TodoError::BadRequest(format!("引用格式非法：{r}（应为 UUID 或 EN-<短号>）"))
            })?;
        Ok(repo::find_by_short_no(&self.pool, n).await.map(to_dto))
    }

    // ---------- 关联关系（blocked_by / relates_to / parent） ----------

    /// 建关联（幂等）。from≠to、双方存在性由调用方/MCP 层校验，库层 FK 兜底。
    pub async fn link(&self, from: Uuid, to: Uuid, kind: &str) -> Result<bool, TodoError> {
        if from == to {
            return Err(TodoError::BadRequest("不能关联自身".into()));
        }
        if !["blocked_by", "relates_to", "parent"].contains(&kind) {
            return Err(TodoError::BadRequest(format!(
                "kind 仅接受 blocked_by/relates_to/parent（收到 {kind}）"
            )));
        }
        repo::link_add(&self.pool, from, to, kind)
            .await
            .map_err(|e| TodoError::Storage(format!("存储暂时不可用: {e}")))
    }

    pub async fn unlink(&self, from: Uuid, to: Uuid, kind: &str) -> Result<bool, TodoError> {
        repo::link_remove(&self.pool, from, to, kind)
            .await
            .map_err(|e| TodoError::Storage(format!("存储暂时不可用: {e}")))
    }

    /// 双向关联列表：(from, to, kind, direction=out|in)。
    pub async fn links(&self, id: Uuid) -> Result<Vec<(Uuid, Uuid, String, String)>, TodoError> {
        repo::links_for(&self.pool, id)
            .await
            .map_err(|e| TodoError::Storage(format!("存储暂时不可用: {e}")))
    }

    /// 关联计数（id → 条数）。
    pub async fn link_count_map(&self) -> Result<std::collections::HashMap<Uuid, i64>, TodoError> {
        repo::link_count_map(&self.pool)
            .await
            .map_err(|e| TodoError::Storage(format!("存储暂时不可用: {e}")))
    }

    /// 新建行动项。
    pub async fn create(
        &self,
        title: &str,
        body: &str,
        priority: &str,
        tags: &[String],
        due_at: Option<DateTime<Utc>>,
    ) -> Result<TodoDto, TodoError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(TodoError::BadRequest("title 不能为空".into()));
        }
        if title.chars().count() > 500 {
            return Err(TodoError::BadRequest("title 过长（>500 字符）".into()));
        }
        Self::reject_nul("title", title)?;
        Self::reject_nul("body", body)?;
        Self::validate_priority(priority)?;
        let tags = Self::normalize_tags(tags);
        for t in &tags {
            Self::reject_nul("tags", t)?;
        }
        let id = Uuid::now_v7();
        repo::insert(
            &self.pool,
            &repo::NewTodo {
                id,
                title,
                body: body.trim(),
                priority,
                tags: &tags,
                due_at,
            },
        )
        .await?;
        self.get(id).await
    }

    /// 列表：open 优先；status/priority/tag/q/due 过滤。
    /// cursor（D29 keyset 分页，单页上限 500）：上一页最后一条的
    /// `{1|0}|{updated_at ISO8601}|{id}`——1 表示该条 status=open。首查不传。
    #[allow(clippy::too_many_arguments)]
    pub async fn list(
        &self,
        status: Option<&str>,
        priority: Option<&str>,
        tag: Option<&str>,
        q: Option<&str>,
        due: Option<&str>,
        cursor: Option<&str>,
        limit: i64,
    ) -> Result<Vec<TodoDto>, TodoError> {
        if limit < 0 {
            return Err(TodoError::BadRequest(format!(
                "limit 不能为负（收到 {limit}）"
            )));
        }
        if let Some(s) = status
            && !STATUSES.contains(&s)
        {
            return Err(TodoError::BadRequest(format!(
                "status 仅接受 {}（收到 {s}）",
                STATUSES.join("/")
            )));
        }
        if let Some(d) = due.filter(|d| *d != "overdue" && *d != "today") {
            return Err(TodoError::BadRequest(format!(
                "due 仅接受 overdue/today（收到 {d}）"
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
        let cursor = parse_todo_cursor(cursor)?;
        Ok(repo::list(
            &self.pool,
            status,
            priority,
            tag,
            q,
            due,
            cursor,
            limit.min(500),
        )
        .await?
        .into_iter()
        .map(to_dto)
        .collect())
    }

    pub async fn get(&self, id: Uuid) -> Result<TodoDto, TodoError> {
        repo::get(&self.pool, id)
            .await?
            .map(to_dto)
            .ok_or_else(|| TodoError::NotFound(format!("待办 {id} 不存在")))
    }

    /// 更新（部分字段，None 不动）。done 语义：open→done 盖 done_at、回 open 清空。
    #[allow(clippy::too_many_arguments)]
    pub async fn update(
        &self,
        id: Uuid,
        title: Option<&str>,
        body: Option<&str>,
        priority: Option<&str>,
        status: Option<&str>,
        due_at: Option<Option<DateTime<Utc>>>,
        tags: Option<&[String]>,
    ) -> Result<TodoDto, TodoError> {
        repo::get(&self.pool, id)
            .await?
            .ok_or_else(|| TodoError::NotFound(format!("待办 {id} 不存在")))?;
        if let Some(t) = title {
            let t = t.trim();
            if t.is_empty() {
                return Err(TodoError::BadRequest("title 不能为空".into()));
            }
            // 与 create 同款上限（P018-T004：旧实现漏查，PATCH 可绕过创建侧约束；500 口径经 trtyr 2026-10-08 拍板）
            if t.chars().count() > 500 {
                return Err(TodoError::BadRequest("title 过长（>500 字符）".into()));
            }
            Self::reject_nul("title", t)?;
        }
        if let Some(b) = body {
            Self::reject_nul("body", b)?;
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
        let tags = tags.map(Self::normalize_tags);
        if let Some(ts) = &tags {
            for t in ts {
                Self::reject_nul("tags", t)?;
            }
        }
        let n = repo::update(
            &self.pool,
            id,
            &repo::TodoPatch {
                title,
                body,
                priority,
                status,
                due_at,
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

    /// 总数（同 list 过滤，不含分页）。
    #[allow(clippy::too_many_arguments)]
    pub async fn count(
        &self,
        status: Option<&str>,
        priority: Option<&str>,
        tag: Option<&str>,
        q: Option<&str>,
    ) -> Result<i64, TodoError> {
        let n = repo::count(&self.pool, status, priority, tag, q).await?;
        Ok(n)
    }
}

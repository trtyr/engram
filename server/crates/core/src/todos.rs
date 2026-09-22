//! 待办域服务：0041 起双形态——
//! **todo**（微软式行动项：速记→做完勾掉，轻量两态）与
//! **ticket**（工单：结构化问题跟踪，severity/症状/复现/验收 + 五态状态机）。
//! 场景靠 tags + priority + due_at + project_hint（纯文本提示，不做 FK 绑定）表达。

use chrono::{DateTime, Utc};

mod validate;
use engram_storage::repo::todos as repo;
use engram_storage::{PgPool, StoreError};
use serde::Serialize;
use uuid::Uuid;
use validate::*;

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

/// 列表过滤参数校验（status/severity/priority 白名单；ticket 状态集另计）。
fn validate_todo_list_filters(
    status: Option<&str>,
    kind: Option<&str>,
    severity: Option<&str>,
    priority: Option<&str>,
) -> Result<(), TodoError> {
    if let Some(s) = status
        && !valid_status(kind.unwrap_or("todo"), s)
        && !(kind.is_none() && TICKET_STATUSES.contains(&s))
    {
        let allowed = match kind {
            Some("ticket") => TICKET_STATUSES.join("/"),
            _ => format!(
                "{}/{}",
                STATUSES.join("/"),
                "confirmed/in_progress/resolved/verified"
            ),
        };
        return Err(TodoError::BadRequest(format!(
            "status 仅接受 {}（收到 {s}）",
            allowed
        )));
    }
    if let Some(sv) = severity
        && !SEVERITIES.contains(&sv)
    {
        return Err(TodoError::BadRequest(format!(
            "severity 仅接受 {}（收到 {sv}）",
            SEVERITIES.join("/")
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
    Ok(())
}

pub const STATUSES: &[&str] = &["open", "done", "archived"];
pub const TICKET_STATUSES: &[&str] = &[
    "open",
    "confirmed",
    "in_progress",
    "resolved",
    "verified",
    "archived",
];
pub const SEVERITIES: &[&str] = &["P0", "P1", "P2", "P3"];
pub const PRIORITIES: &[&str] = &["low", "normal", "high"];

/// kind + status 组合合法性（与 0041 联合 CHECK 同构——应用层先友好报错）。
pub fn valid_status(kind: &str, status: &str) -> bool {
    match kind {
        "todo" => STATUSES.contains(&status),
        "ticket" => TICKET_STATUSES.contains(&status),
        _ => false,
    }
}

fn status_error(kind: &str, status: &str) -> TodoError {
    let allowed = match kind {
        "ticket" => TICKET_STATUSES.join("/"),
        _ => STATUSES.join("/"),
    };
    TodoError::BadRequest(format!(
        "kind={kind} 的 status 仅接受 {allowed}（收到 {status}）"
    ))
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct TodoDto {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    pub kind: String,
    pub status: String,
    pub priority: String,
    pub severity: Option<String>,
    pub symptom: String,
    pub reproduce: String,
    pub acceptance: String,
    pub resolution: String,
    pub tags: Vec<String>,
    pub due_at: Option<DateTime<Utc>>,
    pub project_hint: Option<String>,
    pub done_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
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
        kind: t.kind,
        status: t.status,
        priority: t.priority,
        severity: t.severity,
        symptom: t.symptom,
        reproduce: t.reproduce,
        acceptance: t.acceptance,
        resolution: t.resolution,
        tags: t.tags,
        due_at: t.due_at,
        project_hint: t.project_hint,
        done_at: t.done_at,
        resolved_at: t.resolved_at,
        created_at: t.created_at,
        updated_at: t.updated_at,
        short_no: t.short_no,
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

    /// 新建待办/工单（kind 决定形态：todo=行动项 / ticket=工单）。
    #[allow(clippy::too_many_arguments)]
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

    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        title: &str,
        body: &str,
        kind: &str,
        priority: &str,
        severity: Option<&str>,
        symptom: &str,
        reproduce: &str,
        acceptance: &str,
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
        if kind != "todo" && kind != "ticket" {
            return Err(TodoError::BadRequest(
                "kind 仅接受 todo（行动项）/ ticket（工单）".into(),
            ));
        }
        let tags = prepare_todo_create(kind, severity, tags)?;
        // 分级合并（工单模型细化）：ticket 分级用 severity，priority 退役——
        // 显式传非空 priority 才 400；缺省（空串）静默落 normal（列 NOT NULL 兼容）
        // 分级合并：ticket 分级用 severity——显式传非空 priority 400；
        // 缺省（空串）静默落 normal（列 NOT NULL+CHECK 兼容），语义退役
        let priority: &str = if kind == "ticket" {
            if !priority.trim().is_empty() {
                return Err(TodoError::BadRequest(
                    "工单分级用 severity（P0-P3）——priority 已对 ticket 退役".into(),
                ));
            }
            "normal"
        } else {
            priority
        };
        Self::validate_priority(priority)?;
        let id = Uuid::now_v7();
        repo::insert(
            &self.pool,
            &engram_storage::repo::todos::NewTodo {
                id,
                title,
                body: body.trim(),
                kind,
                priority,
                severity,
                symptom: symptom.trim(),
                reproduce: reproduce.trim(),
                acceptance: acceptance.trim(),
                tags: &tags,
                due_at,
                project_hint: project_hint.map(str::trim).filter(|s| !s.is_empty()),
            },
        )
        .await?;
        self.get(id).await
    }

    /// 列表：open 优先；status/priority/tag/q/severity 过滤。
    /// status 合法集按 kind 取：给了 kind=ticket → 工单六态；给了 kind=todo 或未给 kind →
    /// todo 三态 ∪ 工单态（未给 kind 时两形态都可能命中，取并集不误拒）。
    /// cursor（D29 keyset 分页，单页上限 500）：上一页最后一条的
    /// `{1|0}|{updated_at ISO8601}|{id}`——1 表示该条 status=open。首查不传。
    #[allow(clippy::too_many_arguments)]
    pub async fn list(
        &self,
        status: Option<&str>,
        kind: Option<&str>,
        priority: Option<&str>,
        tag: Option<&str>,
        q: Option<&str>,
        severity: Option<&str>,
        cursor: Option<&str>,
        limit: i64,
    ) -> Result<Vec<TodoDto>, TodoError> {
        if limit < 0 {
            return Err(TodoError::BadRequest(format!(
                "limit 不能为负（收到 {limit}）"
            )));
        }
        let cursor = parse_todo_cursor(cursor)?;
        validate_todo_list_filters(status, kind, severity, priority)?;
        Ok(repo::list(
            &self.pool,
            status,
            kind,
            priority,
            tag,
            q,
            severity,
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

    /// 更新（部分字段，None 不动）。status 合法性按该条的 kind 校验
    /// （todo 拒工单态 / ticket 拒 done——0041 联合 CHECK 的应用层友好版）。
    #[allow(clippy::too_many_arguments)]
    pub async fn update(
        &self,
        id: Uuid,
        kind: Option<&str>,
        title: Option<&str>,
        body: Option<&str>,
        priority: Option<&str>,
        status: Option<&str>,
        severity: Option<Option<&str>>,
        symptom: Option<&str>,
        reproduce: Option<&str>,
        acceptance: Option<&str>,
        resolution: Option<&str>,
        due_at: Option<Option<DateTime<Utc>>>,
        project_hint: Option<Option<&str>>,
        tags: Option<&[String]>,
    ) -> Result<TodoDto, TodoError> {
        let existing = repo::get(&self.pool, id)
            .await?
            .ok_or_else(|| TodoError::NotFound(format!("待办 {id} 不存在")))?;
        let kind = kind.unwrap_or(existing.kind.as_str()).to_string();
        if kind != "todo" && kind != "ticket" {
            return Err(TodoError::BadRequest(
                "kind 仅接受 todo（行动项）/ ticket（工单）".into(),
            ));
        }
        validate_todo_update_basic(&kind, title, body, tags)?;
        validate_todo_update_priority(&kind, priority)?;
        validate_todo_update_status(
            &kind,
            status,
            severity,
            resolution,
            existing.resolution.as_str(),
        )?;
        let n = repo::update(
            &self.pool,
            id,
            &engram_storage::repo::todos::TodoPatch {
                kind: Some(&kind),
                title,
                body,
                priority,
                status,
                severity,
                symptom,
                reproduce,
                acceptance,
                resolution,
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

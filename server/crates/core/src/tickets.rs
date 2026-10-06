//! 工单域服务（0074 拆表）：项目绑定制——工单必须挂在已有项目下，
//! 绑定不了的工单在服务层直接拒绝（不自动建项目、不模糊匹配）。
//! 结构化问题跟踪：severity(P0-P3) / 症状 / 复现 / 验收 / 解决记录 + 六态状态机。

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use engram_storage::StoreError;
use engram_storage::repo::tickets as repo;

/// 六态状态机（与 tickets 表 CHECK 同构——应用层先友好报错）。
pub const STATUSES: &[&str] = &[
    "open",
    "confirmed",
    "in_progress",
    "resolved",
    "verified",
    "archived",
];
pub const SEVERITIES: &[&str] = &["P0", "P1", "P2", "P3"];

#[derive(Debug, thiserror::Error)]
pub enum TicketError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("存储暂时不可用: {0}")]
    Storage(String),
}

impl From<StoreError> for TicketError {
    fn from(e: StoreError) -> Self {
        TicketError::Storage(e.to_string())
    }
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct TicketDto {
    pub id: Uuid,
    pub project_id: Uuid,
    pub title: String,
    pub body: String,
    pub status: String,
    pub severity: Option<String>,
    pub symptom: String,
    pub reproduce: String,
    pub acceptance: String,
    pub resolution: String,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// 全局单调短号（显示为 EN-<n>；与 todos 序列衔接，编号跨表连续）
    pub short_no: i32,
}

fn to_dto(t: repo::TicketRow) -> TicketDto {
    TicketDto {
        id: t.id,
        project_id: t.project_id,
        title: t.title,
        body: t.body,
        status: t.status,
        severity: t.severity,
        symptom: t.symptom,
        reproduce: t.reproduce,
        acceptance: t.acceptance,
        resolution: t.resolution,
        resolved_at: t.resolved_at,
        created_at: t.created_at,
        updated_at: t.updated_at,
        short_no: t.short_no,
    }
}

/// 解析 keyset 游标：`{1|0}|{updated_at RFC3339}|{id}`（取上一页最后一条构造）。
fn parse_ticket_cursor(
    raw: Option<&str>,
) -> Result<Option<(i32, DateTime<Utc>, Uuid)>, TicketError> {
    let cursor = match raw {
        None | Some("") => None,
        Some(raw) => {
            let parts: Vec<&str> = raw.split('|').collect();
            if parts.len() != 3 {
                return Err(TicketError::BadRequest(format!(
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
                    return Err(TicketError::BadRequest(format!(
                        "cursor 非法（收到 {raw:?}）——期望 {{1|0}}|{{updated_at ISO8601}}|{{id}}，取上一页最后一条构造"
                    )));
                }
            }
        }
    };
    Ok(cursor)
}

fn status_error(status: &str) -> TicketError {
    TicketError::BadRequest(format!(
        "status 仅接受 {}（收到 {status}）",
        STATUSES.join("/")
    ))
}

fn severity_error(sv: &str) -> TicketError {
    TicketError::BadRequest(format!(
        "severity 仅接受 {}（收到 {sv}）",
        SEVERITIES.join("/")
    ))
}

/// NUL 字节拒绝（D20）：PG UTF8 层对 \0 直接报编码错误——裸漏成「存储暂时不可用」，
/// 在入参层响亮拒绝。
fn reject_nul(field: &str, value: &str) -> Result<(), TicketError> {
    if value.contains('\0') {
        return Err(TicketError::BadRequest(format!(
            "{field} 含非法控制字符（NUL）"
        )));
    }
    Ok(())
}

pub struct TicketService {
    pool: engram_storage::PgPool,
}

impl TicketService {
    pub fn new(pool: engram_storage::PgPool) -> Self {
        Self { pool }
    }

    /// 建单（项目绑定的唯一入口）：project_id 必须指向已有项目——
    /// 不存在直接拒绝，绝不自动建项目、绝不模糊匹配项目名。
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        project_id: Uuid,
        title: &str,
        body: &str,
        severity: Option<&str>,
        symptom: &str,
        reproduce: &str,
        acceptance: &str,
    ) -> Result<TicketDto, TicketError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(TicketError::BadRequest("title 不能为空".into()));
        }
        if title.chars().count() > 200 {
            return Err(TicketError::BadRequest("title 超长：最多 200 字".into()));
        }
        reject_nul("title", title)?;
        reject_nul("body", body)?;
        reject_nul("symptom", symptom)?;
        reject_nul("reproduce", reproduce)?;
        reject_nul("acceptance", acceptance)?;
        if let Some(sv) = severity
            && !SEVERITIES.contains(&sv)
        {
            return Err(severity_error(sv));
        }
        // 绑定校验：项目必须真实存在——「绑定不了就不进工单」的落点
        let exists = engram_storage::repo::project::existing_ids(&self.pool, &[project_id]).await?;
        if exists.is_empty() {
            return Err(TicketError::BadRequest(format!(
                "项目 {project_id} 不存在——工单必须绑定已有项目；先用 projects create 建项目，或检查 project_id"
            )));
        }
        let id = Uuid::now_v7();
        repo::insert(
            &self.pool,
            &repo::NewTicket {
                id,
                project_id,
                title,
                body,
                severity,
                symptom,
                reproduce,
                acceptance,
            },
        )
        .await?;
        let row = repo::get(&self.pool, id)
            .await?
            .ok_or_else(|| TicketError::NotFound(format!("工单 {id}")))?;
        Ok(to_dto(row))
    }

    /// 列表（open 优先；status/severity/project/q 过滤 + keyset 分页）。
    #[allow(clippy::too_many_arguments)]
    pub async fn list(
        &self,
        status: Option<&str>,
        severity: Option<&str>,
        project_id: Option<Uuid>,
        q: Option<&str>,
        cursor: Option<&str>,
        limit: i64,
    ) -> Result<(Vec<TicketDto>, i64), TicketError> {
        if let Some(s) = status
            && !STATUSES.contains(&s)
        {
            return Err(status_error(s));
        }
        if let Some(sv) = severity
            && !SEVERITIES.contains(&sv)
        {
            return Err(severity_error(sv));
        }
        let cur = parse_ticket_cursor(cursor)?;
        let rows = repo::list(
            &self.pool,
            status,
            severity,
            project_id,
            q,
            cur,
            limit.min(500),
        )
        .await?
        .into_iter()
        .map(to_dto)
        .collect();
        let total = repo::count(&self.pool, status, severity, project_id, q).await?;
        Ok((rows, total))
    }

    /// 按引用取工单：完整 UUID 或短号「EN-<n>」。
    pub async fn find_by_ref(&self, r: &str) -> Result<Option<TicketDto>, TicketError> {
        let r = r.trim();
        if let Ok(id) = Uuid::parse_str(r) {
            return Ok(repo::get(&self.pool, id).await?.map(to_dto));
        }
        let n = r
            .strip_prefix("EN-")
            .or_else(|| r.strip_prefix("en-"))
            .and_then(|n| n.parse::<i32>().ok())
            .ok_or_else(|| {
                TicketError::BadRequest(format!("引用格式非法：{r}（应为 UUID 或 EN-<短号>）"))
            })?;
        Ok(repo::find_by_short_no(&self.pool, n).await.map(to_dto))
    }

    /// 更新：部分更新；status 迁移校验 + resolved/verified 必须带解决记录
    /// （0041 语义随迁——verified 从 resolved 来时 resolution 已有，不再强制重填）。
    #[allow(clippy::too_many_arguments)]
    pub async fn update(
        &self,
        id: Uuid,
        title: Option<&str>,
        body: Option<&str>,
        status: Option<&str>,
        severity: Option<Option<&str>>,
        symptom: Option<&str>,
        reproduce: Option<&str>,
        acceptance: Option<&str>,
        resolution: Option<&str>,
        actor: &str,
    ) -> Result<TicketDto, TicketError> {
        let cur = repo::get(&self.pool, id)
            .await?
            .ok_or_else(|| TicketError::NotFound(format!("工单 {id} 不存在")))?;
        if let Some(t) = title {
            let t = t.trim();
            if t.is_empty() {
                return Err(TicketError::BadRequest("title 不能为空".into()));
            }
            reject_nul("title", t)?;
        }
        for (f, v) in [
            ("body", body),
            ("symptom", symptom),
            ("reproduce", reproduce),
            ("acceptance", acceptance),
            ("resolution", resolution),
        ] {
            if let Some(v) = v {
                reject_nul(f, v)?;
            }
        }
        if let Some(s) = status
            && !STATUSES.contains(&s)
        {
            return Err(status_error(s));
        }
        if let Some(Some(sv)) = severity
            && !SEVERITIES.contains(&sv)
        {
            return Err(severity_error(sv));
        }
        // 解决必填（友好版；CHECK 兜底）
        if let Some(st) = status
            && ["resolved", "verified"].contains(&st)
            && resolution
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .or(Some(cur.resolution.as_str()))
                .filter(|r| !r.is_empty())
                .is_none()
        {
            return Err(TicketError::BadRequest(
                "工单转 resolved/verified 必须填写解决记录（resolution）——做了什么/怎么修的".into(),
            ));
        }
        let n = repo::update(
            &self.pool,
            id,
            &repo::TicketPatch {
                title,
                body,
                status,
                severity,
                symptom,
                reproduce,
                acceptance,
                resolution,
            },
        )
        .await?;
        if n == 0 {
            return Err(TicketError::NotFound(format!("工单 {id} 不存在")));
        }
        // 状态流转自动留痕（ticket_events kind=event）：状态真的变了才记
        let new_status = status.unwrap_or(cur.status.as_str());
        if new_status != cur.status {
            repo::event_insert(
                &self.pool,
                id,
                "event",
                &serde_json::json!({ "from": cur.status, "to": new_status }),
                actor,
            )
            .await?;
        }
        let _ = cur;
        let row = repo::get(&self.pool, id)
            .await?
            .ok_or_else(|| TicketError::NotFound(format!("工单 {id}")))?;
        Ok(to_dto(row))
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), TicketError> {
        let n = repo::delete(&self.pool, id).await?;
        if n == 0 {
            return Err(TicketError::NotFound(format!("工单 {id} 不存在")));
        }
        Ok(())
    }

    /// 项目下的工单数（项目详情汇总用）。
    pub async fn count_by_project(&self, project_id: Uuid) -> Result<i64, TicketError> {
        Ok(repo::count(&self.pool, None, None, Some(project_id), None).await?)
    }

    /// 时间线：追加（kind=event|comment）。
    pub async fn event_add(
        &self,
        ticket_id: Uuid,
        kind: &str,
        payload: &serde_json::Value,
        actor: &str,
    ) -> Result<Uuid, TicketError> {
        if !matches!(kind, "event" | "comment") {
            return Err(TicketError::BadRequest(format!(
                "kind 仅接受 event/comment（收到 {kind}）"
            )));
        }
        // 工单必须存在（时间线不能挂在悬空引用上）
        repo::get(&self.pool, ticket_id)
            .await?
            .ok_or_else(|| TicketError::NotFound(format!("工单 {ticket_id} 不存在")))?;
        Ok(repo::event_insert(&self.pool, ticket_id, kind, payload, actor).await?)
    }

    /// 时间线（升序）。
    pub async fn events(&self, ticket_id: Uuid) -> Result<Vec<repo::TicketEventRow>, TicketError> {
        Ok(repo::events_for(&self.pool, ticket_id).await?)
    }
}

//! jobs 域参数（EN-61）：AI 轮询异步任务的状态/错误/救活——闭环 codegraph index/sync 链路。
//!
//! 权限对齐 HTTP 侧：list/get/events 任何合法凭证可读（AI 轮询自己触发的任务）；
//! revive 仅管理员（与 POST /jobs/{id}/revive 同语义）。

use rmcp::schemars::JsonSchema;

#[derive(Debug, serde::Deserialize, JsonSchema)]
pub struct JobsListParams {
    /// 可选：任务种类过滤（逗号分隔，如 cg_index,cg_sync）
    #[schemars(description = "可选：任务种类过滤（逗号分隔，如 cg_index,cg_sync）。")]
    pub kind: Option<String>,
    /// 可选：状态过滤（逗号分隔：pending,running,succeeded,failed,dead）
    #[schemars(
        description = "可选：状态过滤（逗号分隔：pending,running,succeeded,failed,dead）。"
    )]
    pub status: Option<String>,
    /// 可选：游标（上一页最后一条的 created_at，RFC3339）
    #[schemars(description = "可选：游标（上一页最后一条的 created_at，RFC3339）。")]
    pub cursor: Option<String>,
    /// 可选：条数（默认 50，上限 200）
    #[schemars(description = "可选：条数（默认 50，上限 200）。")]
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, JsonSchema)]
pub struct JobsGetParams {
    /// 任务 id（codegraph index/sync 与 gc 自愈返回的 job_id）
    #[schemars(description = "任务 id（codegraph index/sync 与 gc 自愈返回的 job_id）。")]
    pub id: String,
}

#[derive(Debug, serde::Deserialize, JsonSchema)]
pub struct JobsEventsParams {
    /// 任务 id
    #[schemars(description = "任务 id。")]
    pub id: String,
    /// 可选：增量游标（上一批最后事件 id）
    #[schemars(description = "可选：增量游标（上一批最后事件 id）。")]
    pub after: Option<i64>,
    /// 可选：条数（默认 100，上限 1000）
    #[schemars(description = "可选：条数（默认 100，上限 1000）。")]
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, JsonSchema)]
pub struct JobsReviveParams {
    /// 任务 id（dead/failed 的任务）
    #[schemars(description = "任务 id（dead/failed 的任务）。")]
    pub id: String,
}

// --- jobs 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）---

use super::*;

#[tool_router(router = jobs_router)]
impl EngramMcpServer {
    // ---------- 异步任务域（EN-61）：job 状态/错误/事件/救活——闭环 AI 侧异步链路 ----------

    /// 解析 jobs.list 的状态过滤字面量（唯一收口 `JobStatus::parse_filter`，与 HTTP 同口径；非法值报错，RJ-02）。
    pub(crate) fn parse_job_statuses(
        s: &Option<String>,
    ) -> Result<Vec<engram_jobs::types::JobStatus>, rmcp::ErrorData> {
        engram_jobs::types::JobStatus::parse_filter(s.as_deref())
            .map_err(|e| mcp_err(ErrorCode::INVALID_PARAMS, e))
    }

    /// JobError → MCP 错误（Permanent=参数/状态问题可自愈，Retryable=临时故障）。
    pub(crate) fn from_job(e: engram_jobs::types::JobError) -> rmcp::ErrorData {
        use engram_jobs::types::JobError;
        match e {
            JobError::Permanent(m) => mcp_err(ErrorCode::INVALID_PARAMS, m),
            JobError::Retryable(m) => mcp_err(
                ErrorCode::INTERNAL_ERROR,
                format!("临时故障（可重试）：{m}"),
            ),
        }
    }

    /// 任务列表（EN-61）：codegraph index/sync 与 gc 自愈的 job 都在这。
    pub(crate) async fn jobs_list(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<jobs::JobsListParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        principal_of(&ctx)?; // 鉴权门禁：Principal 值本处不使用
        let cursor = match params.0.cursor.as_deref() {
            Some(v) => Some(parse_flex_datetime(v).map_err(|e| {
                mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    format!("cursor 格式不合法：{e}——用 RFC3339（如 2026-09-17T00:00:00Z）"),
                )
            })?),
            None => None,
        };
        let rows = engram_jobs::JobQueue::new(self.state.pool.clone())
            .list(
                &params
                    .0
                    .kind
                    .as_deref()
                    .map(|v| {
                        v.split(',')
                            .map(|s| s.trim().to_string())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
                &Self::parse_job_statuses(&params.0.status)?,
                cursor,
                params.0.limit.unwrap_or(50).min(200),
            )
            .await
            .map_err(Self::from_job)?;
        Ok(CallToolResult::structured(serde_json::json!({
            "count": rows.len(),
            "jobs": rows,
            "hint": "状态字面量 pending/running/succeeded/failed/dead/cancelled——轮询终止判断用 succeeded 或 failed/dead/cancelled",
        })))
    }

    /// 任务详情（EN-61）：状态/错误/attempts——job_id 从 codegraph index/sync 返回拿。
    pub(crate) async fn jobs_get(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<jobs::JobsGetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        principal_of(&ctx)?; // 鉴权门禁：Principal 值本处不使用
        let id = Uuid::parse_str(&params.0.id).map_err(|_| {
            mcp_err(
                ErrorCode::INVALID_PARAMS,
                format!("任务 id 不是合法 UUID：{}", params.0.id),
            )
        })?;
        let job = engram_jobs::JobQueue::new(self.state.pool.clone())
            .get(id)
            .await
            .map_err(Self::from_job)?
            .ok_or_else(|| {
                mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    format!("任务 {id} 不存在——codegraph index/sync 的返回里有 job_id"),
                )
            })?;
        Ok(CallToolResult::structured(serde_json::json!({
            "job": job,
            "hint": "failed/dead 时看 error 字段定因；dead 可让管理员 revive（Web 控制台或 admin token）",
        })))
    }

    /// 任务事件时间线（EN-61）：增量轮询（after=上一批最后事件 id）。
    pub(crate) async fn jobs_events(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<jobs::JobsEventsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        principal_of(&ctx)?; // 鉴权门禁：Principal 值本处不使用
        let id = Uuid::parse_str(&params.0.id).map_err(|_| {
            mcp_err(
                ErrorCode::INVALID_PARAMS,
                format!("任务 id 不是合法 UUID：{}", params.0.id),
            )
        })?;
        let rows = engram_jobs::JobQueue::new(self.state.pool.clone())
            .events(id, params.0.after, params.0.limit.unwrap_or(100).min(1000))
            .await
            .map_err(Self::from_job)?;
        Ok(CallToolResult::structured(serde_json::json!({
            "count": rows.len(),
            "events": rows,
            "after": rows.last().map(|e| e.id),
        })))
    }

    /// 复活 dead/failed 任务（EN-61）：仅管理员（与 HTTP POST /jobs/{id}/revive 同语义）。
    pub(crate) async fn jobs_revive(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<jobs::JobsReviveParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        match principal_of(&ctx)? {
            Principal::Admin => {}
            Principal::ApiKey { .. } => {
                return Err(mcp_err(
                    ErrorCode::INVALID_REQUEST,
                    "任务复活仅管理员——用 Web 控制台操作，或让管理员处理（HTTP POST /jobs/{id}/revive 同语义）",
                ));
            }
        }
        let id = Uuid::parse_str(&params.0.id).map_err(|_| {
            mcp_err(
                ErrorCode::INVALID_PARAMS,
                format!("任务 id 不是合法 UUID：{}", params.0.id),
            )
        })?;
        let queue = engram_jobs::JobQueue::new(self.state.pool.clone());
        queue.revive(id).await.map_err(Self::from_job)?;
        // 复活后回读确认——非 dead/failed 的任务 UPDATE 影响 0 行，如实告知而非假成功
        let job = queue
            .get(id)
            .await
            .map_err(Self::from_job)?
            .ok_or_else(|| mcp_err(ErrorCode::INVALID_PARAMS, format!("任务 {id} 不存在")))?;
        if matches!(
            job.status,
            engram_jobs::types::JobStatus::Failed | engram_jobs::types::JobStatus::Dead
        ) {
            return Ok(CallToolResult::structured(serde_json::json!({
                "revived": false,
                "id": id,
                "status": job.status.to_string(),
                "hint": "只有 dead/failed 任务能复活——当前状态不符合",
            })));
        }
        Ok(CallToolResult::structured(serde_json::json!({
            "revived": true,
            "id": id,
            "status": job.status.to_string(),
            "hint": "任务已回 pending 重新调度——用 jobs get 跟进进展",
        })))
    }

    /// 异步任务域（EN-61）：轮询 job 状态/错误/事件，闭环 codegraph index/sync
    /// 的异步链路（拿到 job_id 不再干看着）。list/get/events 任何合法凭证可读；
    /// revive 复活 dead/failed 任务仅管理员（与 HTTP 同语义）。操作全景：action="help"。
    #[tool(
        name = "jobs",
        annotations(
            title = "异步任务域",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(crate) async fn jobs_tool(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(call): Parameters<dispatch::DomainCall>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // jobs 无域 scope（对齐 HTTP：list/get 任何合法凭证可读）；revive 的 Admin 检查在 handler 内
        principal_of(&ctx)?; // 鉴权门禁：Principal 值本处不使用
        if call.action == "help" {
            let cfg = load_config(&self.state.pool).await;
            return ok_json(dispatch::render_manual("jobs", &cfg.disabled_tools));
        }
        match call.action.as_str() {
            "list" => {
                self.jobs_list(
                    ctx,
                    Parameters(dispatch::from_args("jobs", "list", call.args)?),
                )
                .await
            }
            "get" => {
                self.jobs_get(
                    ctx,
                    Parameters(dispatch::from_args("jobs", "get", call.args)?),
                )
                .await
            }
            "events" => {
                self.jobs_events(
                    ctx,
                    Parameters(dispatch::from_args("jobs", "events", call.args)?),
                )
                .await
            }
            "revive" => {
                self.jobs_revive(
                    ctx,
                    Parameters(dispatch::from_args("jobs", "revive", call.args)?),
                )
                .await
            }
            other => Err(dispatch::unknown_action("jobs", other)),
        }
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_jobs() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::jobs_router()
}

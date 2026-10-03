//! logs 域参数（P010）：Agent 自查系统日志面——「系统里发生的一切」的查询入口。
//!
//! 与其他域并列的第十一个 MCP 工具。查询能力对齐 HTTP GET /logs：
//! level / q / request_id / since / until / audit / job_id 过滤 + limit/offset 分页；
//! 另有 stats 聚合（按 level/target 计数），便于 Agent 一眼看全局态势。
//! 全部只读——任何合法凭证可读（与 HTTP /logs 的 admin 门禁差异见 handler 内注释）。

use rmcp::schemars::JsonSchema;

/// 日志查询参数（logs.query / logs.stats 共用过滤项）。
#[derive(Debug, serde::Deserialize, JsonSchema)]
pub struct LogsQueryParams {
    /// 可选：级别过滤（TRACE/DEBUG/INFO/WARN/ERROR，大小写不敏感）
    #[schemars(description = "可选：级别过滤（TRACE/DEBUG/INFO/WARN/ERROR，大小写不敏感）。")]
    pub level: Option<String>,
    /// 可选：模糊匹配（message/target ILIKE）
    #[schemars(description = "可选：模糊匹配（message/target ILIKE 子串）。")]
    pub q: Option<String>,
    /// 可选：按后台执行筛（logs 中 fields.job_id）——查某次执行的完整过程
    #[schemars(description = "可选：按执行 id 筛（查该次执行的完整过程：入队/开始/进度/终态）。")]
    pub job_id: Option<String>,
    /// 可选：按 request_id 贯穿筛
    #[schemars(description = "可选：按 request_id 贯穿筛（一次 HTTP 请求引发的全部日志）。")]
    pub request_id: Option<String>,
    /// 可选：功能域筛（T019 域化）
    #[schemars(
        description = "可选：功能域筛（memory=用户记忆蒸馏/审计 / wiki / codegraph / system=请求与错误）。"
    )]
    pub domain: Option<String>,
    /// 可选：起始时间（RFC3339）
    #[schemars(description = "可选：起始时间（RFC3339，如 2026-10-03T00:00:00Z）。")]
    pub since: Option<String>,
    /// 可选：结束时间（RFC3339）
    #[schemars(description = "可选：结束时间（RFC3339）。")]
    pub until: Option<String>,
    /// 可选：条数（默认 100，上限 500）
    #[schemars(description = "可选：条数（默认 100，上限 500）。")]
    pub limit: Option<i64>,
    /// 可选：偏移（分页）
    #[schemars(description = "可选：偏移（分页用）。")]
    pub offset: Option<i64>,
}

/// 日志聚合参数（logs.stats）。
#[derive(Debug, serde::Deserialize, JsonSchema)]
pub struct LogsStatsParams {
    /// 可选：起始时间（RFC3339）；缺省为近 24 小时
    #[schemars(description = "可选：起始时间（RFC3339）；缺省为近 24 小时。")]
    pub since: Option<String>,
    /// 可选：结束时间（RFC3339）
    #[schemars(description = "可选：结束时间（RFC3339）。")]
    pub until: Option<String>,
    /// 分组维度：level（默认）/ target
    #[schemars(description = "分组维度：level（默认）/ target。")]
    pub group_by: Option<String>,
}

// --- logs 域 MCP 工具面 ---

use super::*;

#[tool_router(router = logs_router)]
impl EngramMcpServer {
    /// 查询系统日志（P010）：系统里发生的一切——请求、错误、后台执行都在这条时间线上。
    pub(crate) async fn logs_query(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<logs::LogsQueryParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        principal_of(&ctx)?;
        let p = params.0;
        let since = parse_logs_time(p.since.as_deref(), "since")?;
        let until = parse_logs_time(p.until.as_deref(), "until")?;
        let filter = engram_storage::repo::logs::LogFilter {
            level: p.level.as_deref(),
            q: p.q.as_deref(),
            request_id: p.request_id.as_deref(),
            job_id: p.job_id.as_deref(),
            job_scope: None,
            domain: p.domain.as_deref(),
            since,
            until,
            audit_only: false,
            limit: p.limit.unwrap_or(100),
            offset: p.offset.unwrap_or(0),
        };
        let rows = engram_storage::repo::logs::query_logs(&self.state.pool, &filter)
            .await
            .map_err(|e| mcp_err(ErrorCode::INTERNAL_ERROR, format!("日志查询失败：{e}")))?;
        Ok(CallToolResult::structured(serde_json::json!({
            "count": rows.len(),
            "logs": rows,
            "hint": "后台执行过程用 job_id 筛（入队→开始→进度→终态全在同一条时间线）；系统错误用 level=ERROR。",
        })))
    }

    /// 日志聚合（P010）：按 level 或 target 计数，一眼看系统态势。
    pub(crate) async fn logs_stats(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<logs::LogsStatsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        principal_of(&ctx)?;
        let p = params.0;
        let until = parse_logs_time(p.until.as_deref(), "until")?.unwrap_or_else(chrono::Utc::now);
        let since = parse_logs_time(p.since.as_deref(), "since")?
            .unwrap_or_else(|| until - chrono::Duration::hours(24));
        let group_by = p.group_by.as_deref().unwrap_or("level");
        if group_by != "level" && group_by != "target" {
            return Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                "group_by 只支持 level / target",
            ));
        }
        let buckets =
            engram_storage::repo::logs::count_logs(&self.state.pool, since, until, group_by)
                .await
                .map_err(|e| mcp_err(ErrorCode::INTERNAL_ERROR, format!("日志聚合失败：{e}")))?;
        Ok(CallToolResult::structured(serde_json::json!({
            "since": since,
            "until": until,
            "group_by": group_by,
            "buckets": buckets,
        })))
    }

    /// 日志域（P010）：系统里发生的一切都在这里——请求/错误/后台执行。
    /// 支持 level/q/domain/job_id/request_id/时间窗过滤，另有 stats 聚合。
    /// 操作全景：action="help"。
    #[tool(
        name = "logs",
        annotations(
            title = "日志域",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(crate) async fn logs_tool(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(call): Parameters<dispatch::DomainCall>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        principal_of(&ctx)?;
        if call.action == "help" {
            let cfg = load_config(&self.state.pool).await;
            return ok_json(dispatch::render_manual("logs", &cfg.disabled_tools));
        }
        match call.action.as_str() {
            "query" => {
                self.logs_query(
                    ctx,
                    Parameters(dispatch::from_args("logs", "query", call.args)?),
                )
                .await
            }
            "stats" => {
                self.logs_stats(
                    ctx,
                    Parameters(dispatch::from_args("logs", "stats", call.args)?),
                )
                .await
            }
            other => Err(dispatch::unknown_action("logs", other)),
        }
    }
}

/// 解析日志时间参数（RFC3339；空串=None；错误带字段名）。
fn parse_logs_time(
    raw: Option<&str>,
    field: &str,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, rmcp::ErrorData> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => chrono::DateTime::parse_from_rfc3339(s)
            .map(|d| Some(d.with_timezone(&chrono::Utc)))
            .map_err(|_| {
                mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    format!("{field} 需 RFC3339（如 2026-10-01T00:00:00Z）"),
                )
            }),
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_logs() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::logs_router()
}

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

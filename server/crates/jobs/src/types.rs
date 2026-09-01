//! 任务领域类型。

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

/// 任务状态机：
/// pending → running → succeeded
///              ↘ failed（永久错误终态）
///              ↘ dead（可重试错误重试耗尽；人工可复活）
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, utoipa::ToSchema)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Dead,
    /// P-C 两阶段清空的取消态（0019 迁移同日加入 DB CHECK；缺这个变体会让
    /// list_jobs 反序列化整表炸 503——2026-08-31 登录态误判事故的病根）
    Cancelled,
}

/// 任务行。
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, utoipa::ToSchema)]
pub struct Job {
    pub id: Uuid,
    pub kind: String,
    #[schema(value_type = Object)]
    pub payload: sqlx::types::Json<Value>,
    pub status: JobStatus,
    pub attempts: i32,
    pub max_attempts: i32,
    pub idempotency_key: Option<String>,
    pub error: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub progress: Option<sqlx::types::Json<Value>>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub due_at: DateTime<Utc>,
    pub locked_by: Option<String>,
    pub locked_at: Option<DateTime<Utc>>,
    pub visibility_timeout_s: i32,
}

/// 任务事件。
#[derive(Debug, Clone, sqlx::FromRow, Serialize, utoipa::ToSchema)]
pub struct JobEvent {
    pub id: i64,
    pub job_id: Uuid,
    pub ts: DateTime<Utc>,
    pub level: String,
    pub message: String,
    #[schema(value_type = Option<Object>)]
    pub data: Option<sqlx::types::Json<Value>>,
}

/// 入队模板。
#[derive(Debug, Clone)]
pub struct JobTemplate {
    pub kind: String,
    pub payload: Value,
    /// 幂等键：同键任务已存在则直接返回既有任务（不重复入队）。
    pub idempotency_key: Option<String>,
    pub max_attempts: i32,
    pub visibility_timeout_s: i32,
    /// 延迟执行（定时任务）；默认立即。
    pub due_at: Option<DateTime<Utc>>,
}

impl JobTemplate {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            payload: Value::Null,
            idempotency_key: None,
            max_attempts: 3,
            visibility_timeout_s: 300,
            due_at: None,
        }
    }

    pub fn with_payload(mut self, payload: Value) -> Self {
        self.payload = payload;
        self
    }

    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// 延迟执行时间（定时/防抖）。
    pub fn with_due(mut self, due_at: chrono::DateTime<chrono::Utc>) -> Self {
        self.due_at = Some(due_at);
        self
    }
}

/// 执行错误分类（决定重试与否）。
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    /// 瞬态故障（LLM 429/5xx、网络抖动、超时）：按退避重试。
    #[error("可重试: {0}")]
    Retryable(String),
    /// 永久错误（参数/校验/逻辑）：不重试，直接 failed。
    #[error("永久失败: {0}")]
    Permanent(String),
}

/// fail() 的结果去向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailOutcome {
    /// 重排等待下次执行
    Rescheduled,
    /// 永久失败终态
    Failed,
    /// 重试耗尽终态
    Dead,
}

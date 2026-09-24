//! 统一错误体。所有 handler 的错误出口。
//!
//! 契约（engram projects 域（主题：api-design））：
//! `{"error": {"code", "message", "retryable", "details?"}}`
//! - code 稳定可编程判断；message 是人话且不泄漏内部细节。
//! - 内部细节只进日志，不进响应。

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ErrorBody {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// API 层错误。变体即错误分类；新增类别时同步更新 code。
#[allow(
    dead_code,
    reason = "Database/Internal 变体经 From 转换构造，无直接构造点"
)]
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// 请求参数不合法（不重试）
    #[error("{0}")]
    BadRequest(String),
    /// 资源不存在
    #[error("{0}")]
    NotFound(String),
    /// 资源已存在冲突（409，唯一约束命中）
    #[error("{0}")]
    Conflict(String),
    /// 未认证（401）
    #[error("{0}")]
    Unauthorized(String),
    /// 已认证但权限不足（403）
    #[error("{0}")]
    Forbidden(String),
    /// 触发速率限制（429；可重试——等冷却窗口）
    #[error("尝试过于频繁，请稍后再试")]
    TooManyRequests,
    /// 数据库故障（可重试）
    #[error("存储层暂时不可用")]
    Database(#[source] engram_storage::StoreError),
    /// 依赖服务不可用（可重试）
    #[error("服务暂时不可用，请稍后重试")]
    Unavailable(String),
    /// 未捕获的内部错误（不重试；详情只进日志）
    #[error("内部错误")]
    Internal(anyhow::Error),
}

impl ApiError {
    fn parts(&self) -> (StatusCode, &'static str, bool) {
        match self {
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request", false),
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found", false),
            ApiError::Conflict(_) => (StatusCode::CONFLICT, "conflict", false),
            ApiError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "unauthorized", false),
            ApiError::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden", false),
            ApiError::TooManyRequests => (StatusCode::TOO_MANY_REQUESTS, "too_many_requests", true),
            ApiError::Database(_) => (StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable", true),
            ApiError::Unavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable", true),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal", false),
        }
    }

    /// 用户可见 message（安全子集）。
    fn safe_message(&self) -> String {
        match self {
            ApiError::BadRequest(m) | ApiError::NotFound(m) | ApiError::Conflict(m) => m.clone(),
            _ => self.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, retryable) = self.parts();
        // 内部细节（Debug 格式含 source 链）进日志，一次性记录
        tracing::error!(
            code,
            status = status.as_u16(),
            error = ?self,
            "API 错误"
        );
        let body = ErrorEnvelope {
            error: ErrorBody {
                code,
                message: self.safe_message(),
                retryable,
                details: None,
            },
        };
        (status, Json(body)).into_response()
    }
}

impl From<engram_jobs::types::JobError> for ApiError {
    fn from(e: engram_jobs::types::JobError) -> Self {
        ApiError::Unavailable(e.to_string())
    }
}

impl From<engram_llm::types::LlmError> for ApiError {
    fn from(e: engram_llm::types::LlmError) -> Self {
        match e {
            engram_llm::types::LlmError::NotConfigured(m) => ApiError::Unavailable(m),
            other => ApiError::Unavailable(other.to_string()),
        }
    }
}

impl From<engram_storage::StoreError> for ApiError {
    fn from(e: engram_storage::StoreError) -> Self {
        ApiError::Database(e)
    }
}

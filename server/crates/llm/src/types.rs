//! 领域类型。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 调用用途（路由键）。档位见 topics/llm-providers.md。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Purpose {
    /// L0→L1 抽取（低档）
    Extract,
    /// 矛盾仲裁（低档）
    Arbitrate,
    /// 嵌入（低档，embedding 模型）
    Embed,
    /// L1→L2 组织（中档）
    Organize,
    /// 定期整理（中档）
    Consolidate,
    /// Wiki 分析（中档）
    WikiAnalysis,
    /// L2→L3 画像（高档）
    Persona,
    /// Wiki 生成（高档）
    WikiGeneration,
    /// Wiki 语义 lint（中档——页面间矛盾/过时/缺页检查）
    WikiLint,
    /// 检索重排序（中档——R6：top 候选精排）
    SearchRerank,
}

impl Purpose {
    pub fn as_str(&self) -> &'static str {
        match self {
            Purpose::Extract => "extract",
            Purpose::Arbitrate => "arbitrate",
            Purpose::Embed => "embed",
            Purpose::Organize => "organize",
            Purpose::Consolidate => "consolidate",
            Purpose::WikiAnalysis => "wiki_analysis",
            Purpose::Persona => "persona",
            Purpose::WikiGeneration => "wiki_generation",
            Purpose::WikiLint => "wiki_lint",
            Purpose::SearchRerank => "search_rerank",
        }
    }
}

/// 聊天消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String, // system | user | assistant
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }
}

/// 聊天请求。
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    /// 采样温度（None = 服务端默认）
    pub temperature: Option<f32>,
    /// 强制 JSON 输出（OpenAI response_format 兼容；不支持时由提示词兜底）
    pub json_mode: bool,
    pub max_tokens: Option<u32>,
}

/// 聊天响应。
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub model: String,
    pub latency_ms: i64,
}

/// 嵌入请求（批量）。
#[derive(Debug, Clone)]
pub struct EmbedRequest {
    pub model: String,
    pub inputs: Vec<String>,
    /// 请求降维（matryoshka，如 Qwen3-Embedding；None = 服务端默认维度）。
    /// 存储层统一 1024 维（D0010）。
    pub dimensions: Option<u32>,
}

/// 嵌入响应（与 inputs 等长同序）。
#[derive(Debug, Clone)]
pub struct EmbedResponse {
    pub embeddings: Vec<Vec<f32>>,
    pub input_tokens: i64,
    pub model: String,
    pub latency_ms: i64,
}

/// 用量记账行。
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct UsageRecord {
    pub id: i64,
    pub provider: String,
    pub model: String,
    pub purpose: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub latency_ms: i32,
    pub job_id: Option<uuid::Uuid>,
    pub ts: DateTime<Utc>,
}

/// 记账参数（record_usage 入参）。
#[derive(Debug, Clone)]
pub struct UsageMeta {
    pub provider: String,
    pub model: String,
    pub purpose: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub latency_ms: i64,
    pub job_id: Option<Uuid>,
}

/// LLM 错误分类（jobs 重试策略消费）。
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// 瞬态：429/5xx/超时/网络——可重试
    #[error("LLM 瞬态故障: {0}")]
    Transient(String),
    /// 永久：401/404/模型不存在/响应格式错——不重试
    #[error("LLM 永久错误: {0}")]
    Permanent(String),
    /// 本地没有可用 provider/路由——配置问题
    #[error("LLM 配置缺失: {0}")]
    NotConfigured(String),
}

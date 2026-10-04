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
    /// Wiki 维护 Agent Harness（P004-T010；档位独立——可为它配专属模型分账）
    WikiAgent,
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
            Purpose::WikiAgent => "wiki_agent",
        }
    }
}

/// 聊天消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String, // system | user | assistant | tool
    pub content: String,
    /// assistant 消息的工具调用请求（tool-calling 循环用；None = 普通消息）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// tool 角色消息对应的调用 id（OpenAI tool_call_id）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }
    /// assistant：携带工具调用请求（harness 循环回填用）。
    pub fn assistant_with_tool_calls(content: impl Into<String>, calls: Vec<ToolCall>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            tool_calls: Some(calls),
            tool_call_id: None,
        }
    }
    /// tool：工具执行结果回填。
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// 工具定义（OpenAI function-calling 格式的中性表达）。
#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// 参数 JSON Schema（object）
    pub parameters: serde_json::Value,
}

/// 一次工具调用（响应侧解析 / 回填消息共用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// OpenAI 格式：arguments 是 JSON 字符串
    pub arguments: String,
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
    /// 可用工具（Some = 开启 tool-calling）
    pub tools: Option<Vec<ToolDef>>,
    /// 透传到请求体的额外参数（provider 差异大：关闭思考链各家用不同字段——
    /// OpenRouter `reasoning.enabled=false`、DashScope `enable_thinking=false`——
    /// 统一透传通道，不在类型层枚举各家中文名。None = 不带。）
    pub extras: Option<serde_json::Map<String, serde_json::Value>>,
}

/// 聊天响应。
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content: String,
    /// 工具调用请求（Some = 模型要求调工具；content 可能为空）
    pub tool_calls: Option<Vec<ToolCall>>,
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

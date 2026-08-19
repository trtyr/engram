//! LLM 客户端：provider 抽象（OpenAI 兼容）、purpose 路由、用量记账、密钥加密。
//! 平台所有 LLM 调用的唯一出口。
//!
//! 设计文档：docs/plantree/plans/agent-memory-platform/topics/llm-providers.md

pub mod crypto;
pub mod provider;
pub mod router;
pub mod types;

pub use crypto::KeyCipher;
pub use provider::{OpenAiCompatProvider, ProviderRegistry};
pub use router::PurposeRouter;
pub use types::{
    ChatMessage, ChatRequest, ChatResponse, EmbedRequest, EmbedResponse, LlmError, ModelInfo,
    Purpose, UsageRecord,
};

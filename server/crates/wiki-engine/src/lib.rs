//! Wiki 引擎：两步 ingest（analysis→generation）、wikilink 解析、
//! index/log/overview 维护、lint、链接图。
//!
//! 设计文档：docs/plantree/plans/agent-memory-platform/topics/wiki-engine.md
//! 模式：Karpathy LLM-wiki（原料不可变，LLM 增量维护，人负责纠偏）。

pub mod ingest;
pub mod lint;
pub mod markup;
pub mod prompts;
pub mod service;

pub use lint::LintReport;
pub use service::{LlmRef, WikiError, WikiPageDto, WikiService};

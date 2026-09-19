//! Wiki 引擎：两步 ingest（analysis→generation）、wikilink 解析、
//! index/log/overview 维护、lint、链接图、多库（library）管理。
//!
//! 设计文档：docs/plantree/plans/engram-platform/topics/wiki-engine.md
//! 模式：Karpathy LLM-wiki（原料不可变，LLM 增量维护，人负责纠偏）。
//! 多库（0037）：库是一级命名空间，页面/双链/原料/审查/洞察/purpose 全部挂库。

pub mod cascade;
pub mod community;
pub mod community_summaries;
pub mod cross_links;
pub mod ingest;
pub mod insights;
pub mod libraries;
pub mod lint;
pub mod lint_deep;
pub mod markup;
pub mod promote;
pub mod prompts;
pub mod purpose;
pub mod relevance;
pub mod repair;
pub mod review;

/// 数据根解析唯一收口（EN-47）——顶层 re-export，供 core/api/mcp 调用。
pub use ingest::data_root;
pub mod service;

pub use libraries::WikiLibraryDto;
pub use lint::LintReport;
pub use service::{LlmRef, WikiError, WikiPageDto, WikiService};

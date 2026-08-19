//! 混合检索：FTS + pgvector ANN + RRF 融合、上下文预算控制。
//!
//! 设计文档：docs/plantree/plans/agent-memory-platform/topics/search.md
//! 中文方案（D0009）：应用层 jieba 预分词，写入与查询同源。

pub mod hybrid;
pub mod rrf;
pub mod tokenize;

pub use hybrid::{SearchHit, search_atoms, search_scenarios};
pub use rrf::rrf_merge;
pub use tokenize::{tsv_query, tsv_text};

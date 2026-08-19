//! 领域层：memory / knowledge / wiki / codegraph 各域服务与跨域编排。
//!
//! 依赖方向（见 docs/plantree/baseline/module-map.md）：
//! `api → core → (storage, llm, jobs, search, distill, wiki-engine, cg-bridge)`

pub mod memory;

pub use memory::{MemoryService, SearchResponse};

//! 领域层：memory / knowledge / wiki / codegraph 各域服务与跨域编排。
//!
//! 依赖方向（见 docs/plantree/baseline/module-map.md）：
//! `api → core → (storage, llm, jobs, search, distill, wiki-engine, cg-bridge, parsing)`

pub mod codegraph;
pub mod knowledge;
pub mod memory;
pub mod unified;
pub mod wiki;

pub use codegraph::{CgBridge, CgError, CgProjectDto, QueryKind};
pub use knowledge::{ChunkHit, DocumentDto, KnowledgeService};
pub use memory::{AtomRevision, MemoryService, SearchResponse};
pub use unified::{UnifiedError, UnifiedHit, UnifiedSearch};
pub use wiki::{LintReport, WikiError, WikiPageDto, WikiService};

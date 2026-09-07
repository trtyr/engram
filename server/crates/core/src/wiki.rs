//! Wiki 域门面——api 经此访问 wiki-engine（实现留在 wiki-engine crate）。
//!
//! 依赖方向：api → core → wiki-engine（Q9 收敛：api 不再直连 wiki-engine）。

pub use engram_wiki_engine::cascade::CascadeReport;
pub use engram_wiki_engine::ingest::IngestOutcome;
pub use engram_wiki_engine::insights::InsightsReport;
pub use engram_wiki_engine::purpose::Purpose;
pub use engram_wiki_engine::review::ReviewItem;
pub use engram_wiki_engine::service::GraphDto;
pub use engram_wiki_engine::service::WikiPageVersionDto;
pub use engram_wiki_engine::{LintReport, LlmRef, WikiError, WikiPageDto, WikiService};

/// 摄取 job handler 注册（main.rs 装配用）。
pub mod ingest {
    pub use engram_wiki_engine::ingest::register_handlers;
}

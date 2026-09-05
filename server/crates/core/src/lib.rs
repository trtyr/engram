//! 领域层：memory / wiki / codegraph 各域服务与跨域编排。
//!
//! 依赖方向（见 docs/plantree/baseline/module-map.md）：
//! `api → core → (storage, llm, jobs, search, distill, wiki-engine, cg-bridge, parsing)`

pub mod codegraph;
pub mod memory;
pub mod project;
pub mod skills;
pub mod unified;
pub mod wiki;
pub mod wiki_docs;

pub use codegraph::{CgBridge, CgError, CgProjectDto, QueryKind};
pub use memory::{
    AtomRevision, EntityRelationDto, EntityRevision, MemoryService, SearchResponse, TimelineEvent,
    purge_deep_pool,
};
pub use project::{
    PROJECT_STATUSES, PROJECT_TYPES, ProjectDetailDto, ProjectDocDto, ProjectError,
    ProjectLocationDto, ProjectService, ProjectTypeDto, type_label,
};
pub use skills::{
    MAX_REVISIONS, SkillDto, SkillImportItem, SkillImportReport, SkillPatch, SkillRevisionDto,
    SkillSummaryDto, SkillsError, SkillsService, parse_frontmatter, slugify, valid_slug,
};
pub use unified::{UnifiedError, UnifiedHit, UnifiedSearch};
pub use wiki::{LintReport, WikiError, WikiPageDto, WikiService};
pub use wiki_docs::{ChunkHit, DocumentDto, WikiDocumentService};

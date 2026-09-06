//! 领域层：memory / wiki / codegraph 各域服务与跨域编排。
//!
//! 依赖方向：`api/mcp → core → storage → DB`，core 不直接写 SQL——
//! 领域表的业务面读写唯一收口在 `engram_storage::repo`（含持久化模型 models）。
//! 本 crate 同时承载跨适配器（HTTP / MCP）共用的身份与装配类型（auth / state）。

pub mod auth;
pub mod codegraph;
pub mod memory;
pub mod project;
pub mod skills;
pub mod state;
pub mod transfer;
pub mod unified;
pub mod wiki;
pub mod wiki_docs;

pub use auth::{Principal, SCOPES};
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
    MAX_REVISIONS, SKILL_FILE_MAX_CHARS, SKILL_FILES_MAX, SkillDto, SkillExportDto,
    SkillFileEntryDto, SkillFileInfoDto, SkillImportItem, SkillImportReport, SkillPatch,
    SkillRevisionDto, SkillSummaryDto, SkillsError, SkillsService, parse_frontmatter, slugify,
    valid_slug,
};
pub use state::AppState;
pub use unified::{UnifiedError, UnifiedHit, UnifiedSearch};
pub use wiki::{LintReport, WikiError, WikiPageDto, WikiService};
pub use wiki_docs::{ChunkHit, DocumentDto, WikiDocumentService};

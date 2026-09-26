//! MCP（Model Context Protocol）适配器：渐进式发现工具面（九域 + 跨域 search_all）。
//!
//! 官方 Rust SDK（rmcp）Streamable HTTP 传输，由 api 装配到 engram-server 的 `/mcp` 端点。
//! 与 HTTP 路由平级的第二适配器：同一套 core 服务与 scope 分权，独立成 crate。
//! 渐进式发现（progressive disclosure）：九个领域各一个入口工具
//! （memory/projects/assets/skills/wiki/todos/tickets/codegraph/jobs），域内操作经 action 分发
//! （历史 53 个扁平工具全部收编，R 测试报告后又扩至 60+ 个：remember/doc_patch/
//! versions/restore/sources 等；action 表在 dispatch 模块，三级发现同源）。
//! 各域工具在调用时检查各自 scope。鉴权复用 Bearer 中间件（amk_ key / ams_ 会话）：
//! 每个工具调用请求都过 `bearer_auth`，Principal 已注入 request extensions；
//! rmcp 把 HTTP request Parts 注入工具上下文，工具实现从这里取 Principal
//! 做与 HTTP API 同一套的 scope / 编辑分权检查。
//! key 吊销即刻生效（每个请求独立认证，会话保活也不能豁免）。
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))] // 架构治理 task-5：生产代码禁裸崩溃（测试豁免）

mod assets;
mod codegraph;
pub mod dispatch;
mod guard;
pub mod jobs;
mod memory;
mod memory_kv;
mod memory_manage;
mod memory_sessions;
mod memory_write;
mod project_docs;
mod project_files;
mod project_links;
mod project_locations;
mod projects;
mod registry;
mod search_all;
mod server;
#[allow(dead_code)]
mod skills;
#[allow(dead_code)]
mod skills_versions;
mod tickets;
mod todos;
mod todos_links;
pub mod wiki;
mod wiki_curation;
mod wiki_docs;
mod wiki_ops;

// 架构治理 2026-09-20：lib.rs 只留 crate 装配（类型/构造/router 合并/再导出），
// 各域工具面在各自模块内（`#[tool_router(router = <mod>_router)]` + `routes_<mod>()`）。
pub(crate) use assets::*;
pub(crate) use codegraph::*;
pub(crate) use guard::*;
pub(crate) use memory::*;
pub(crate) use memory_kv::*;
pub(crate) use memory_sessions::*;
pub(crate) use memory_write::*;
pub(crate) use project_docs::*;
pub(crate) use project_files::*;
pub(crate) use project_links::*;
pub(crate) use project_locations::*;
pub(crate) use projects::*;
pub use registry::*;
pub use server::*;
#[allow(unused_imports)]
pub(crate) use skills::*;
#[allow(unused_imports)]
pub(crate) use skills_versions::*;
pub(crate) use tickets::*;
pub(crate) use todos::*;
pub(crate) use todos_links::*;

use rmcp::ServerHandler;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, ErrorCode, Implementation, ServerCapabilities, ServerInfo,
};
use rmcp::schemars::JsonSchema;
use rmcp::service::RequestContext;
use rmcp::service::RoleServer;
use rmcp::tool;
use rmcp::tool_handler;
use rmcp::tool_router;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::tower::{
    StreamableHttpServerConfig, StreamableHttpService,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use axum::response::IntoResponse;
use engram_core::auth::{DomainAccess, Principal};
use engram_core::state::AppState;
use engram_core::unified::UnifiedHit;

impl EngramMcpServer {
    /// 合并各域工具 router（每个域模块自带 `#[tool_router]` 块）。
    pub(crate) fn build_tool_router() -> ToolRouter<Self> {
        let mut tool_router = ToolRouter::<Self>::new();
        // EN-252：skills 域裁撤——工具路由注销（模块与存储层保留供回滚）
        tool_router.merge(crate::wiki_ops::routes_wiki_ops());
        tool_router.merge(crate::projects::routes_projects());
        tool_router.merge(crate::assets::routes_assets());
        tool_router.merge(crate::codegraph::routes_codegraph());
        tool_router.merge(crate::todos::routes_todos());
        tool_router.merge(crate::memory::routes_memory());
        tool_router.merge(crate::memory_write::routes_memory_write());
        tool_router.merge(crate::search_all::routes_search_all());
        tool_router.merge(crate::project_docs::routes_project_docs());
        tool_router.merge(crate::wiki_curation::routes_wiki_curation());
        tool_router.merge(crate::jobs::routes_jobs());
        tool_router.merge(crate::tickets::routes_tickets());
        tool_router.merge(crate::memory_sessions::routes_memory_sessions());
        tool_router.merge(crate::project_files::routes_project_files());
        tool_router.merge(crate::wiki_docs::routes_wiki_docs());
        tool_router.merge(crate::todos_links::routes_todos_links());
        tool_router.merge(crate::memory_kv::routes_memory_kv());
        tool_router.merge(crate::project_locations::routes_project_locations());
        // tool_router.merge(crate::skills_versions::routes_skills_versions()); // EN-252 裁撤
        tool_router
    }
}

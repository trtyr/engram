//! MCP Wiki 域工具面：参数结构 + 错误桥 + 实现辅助（engram-mcp 内部模块）。
//!
//! `#[tool]` 方法必须落在 `mcp.rs` 的 `#[tool_router]` impl 块内（rmcp 宏只收集
//! 该块内标注的方法），这里只放参数结构与可复用的实现细节，保持 mcp.rs 可读。

use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::mcp_err;
use engram_core::auth::Principal;

/// WikiError → MCP 错误码（与 wiki_api.rs 的 we() 同语义）。
pub fn from_wiki(e: engram_core::wiki::WikiError) -> rmcp::ErrorData {
    use engram_core::wiki::WikiError;
    match e {
        WikiError::NotFound(m) => rmcp::ErrorData::resource_not_found(m, None),
        WikiError::BadRequest(m) => rmcp::ErrorData::invalid_params(m, None),
        WikiError::Storage(m) => rmcp::ErrorData::internal_error(m, None),
    }
}

pub fn require_wiki(principal: &Principal) -> Result<(), rmcp::ErrorData> {
    if principal.has_scope("wiki") {
        Ok(())
    } else {
        Err(mcp_err(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            "缺少 wiki scope——请用带 wiki scope 的 amk_ key 连接 MCP",
        ))
    }
}

pub fn svc(state: &engram_core::state::AppState) -> engram_core::wiki::WikiService {
    engram_core::wiki::WikiService::new(state.pool.clone(), state.registry())
}

/// wiki_list_pages 的瘦身输出：列表不带正文（正文可能很大），全文走 wiki_get_page。
pub fn trim_page(page: &serde_json::Value) -> serde_json::Value {
    let mut v = page.clone();
    v["content"] = serde_json::json!("");
    v["content_omitted"] = serde_json::json!(true);
    v
}

// ---------- 工具参数 ----------

/// 无参工具（wiki_graph / wiki_lint）的占位参数：MCP 规范要求 inputSchema 根类型为 object，
/// 不能用 `Parameters<()>`（其 schema 为 null）。
#[derive(Serialize, Deserialize, JsonSchema, Default)]
pub struct WikiNoParams {}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiSearchParams {
    /// 检索词
    #[schemars(description = "检索词。中英文均可，混合检索（全文 FTS + 向量）。")]
    pub query: String,
    /// 返回条数（默认 20，上限 50）
    #[schemars(description = "返回条数，默认 20。")]
    pub max_items: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiListPagesParams {
    /// 页型过滤：entity/concept/source/synthesis/comparison/queries/overview/index 等
    #[schemars(
        description = "可选：按页型过滤。entity=实体, concept=概念, source=来源, synthesis=综合, comparison=对比, queries=查询存档, overview=总览, index=索引。"
    )]
    pub page_type: Option<String>,
    /// 返回条数（默认 100，上限 300）
    #[schemars(description = "返回条数，默认 100。")]
    pub limit: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiGetPageParams {
    /// 页面 slug（来自 wiki_list_pages / wiki_search 的返回）
    #[schemars(description = "页面 slug。对大小写与空格/连字符差异宽容。")]
    pub slug: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiWritePageParams {
    /// 页面 slug（中英数字与 - _ ·，≤80 字符，禁空白与路径分隔符；已存在则覆盖更新）
    #[schemars(
        description = "页面 slug：中英数字与连字符，≤80 字符，禁空白。已存在同 slug 页面则整体覆盖更新（版本 +1）。"
    )]
    pub slug: String,
    /// 页面标题
    #[schemars(description = "页面标题。")]
    pub title: String,
    /// Markdown 正文（可用 [[wikilink]] 双链其他页面）
    #[schemars(description = "Markdown 正文。可用 [[slug]] 双链其他页面，互链会进链接图。")]
    pub content: String,
    /// 目录树文件夹（Obsidian 式 / 分隔多级路径；缺省用页型默认目录）
    #[schemars(description = "可选：目录树文件夹（/ 分隔多级路径）。缺省按页型默认目录。")]
    pub folder: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiIngestParams {
    /// 来源标题
    #[schemars(description = "来源标题（如文档名、主题名）。")]
    pub title: String,
    /// 源文本（Markdown/纯文本；相同内容重复织入会被 sha 去重跳过）
    #[schemars(description = "源文本全文。内容相同（sha 命中）会跳过。")]
    pub text: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiArchiveQueryParams {
    /// 存档标题（同标题已存档会幂等跳过）
    #[schemars(description = "存档标题。同标题已存档 → 幂等跳过（skipped=true）。")]
    pub title: String,
    /// 当时的提问
    #[schemars(description = "当时的提问原文。")]
    pub question: String,
    /// 最终回答
    #[schemars(description = "最终回答（值得沉淀的版本，不要贴过程流水账）。")]
    pub answer: String,
}

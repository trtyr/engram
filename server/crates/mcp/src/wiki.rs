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
    match principal.domain_access("wiki") {
        engram_core::auth::DomainAccess::None => Err(mcp_err(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            "缺少 wiki scope——请用带 wiki scope 的 amk_ key 连接 MCP",
        )),
        // Full 直过；ReadOnly 的动作级判定在 call_tool 入口（check_action_access）已做
        _ => Ok(()),
    }
}

pub fn svc(state: &engram_core::state::AppState) -> engram_core::wiki::WikiService {
    engram_core::wiki::WikiService::new(state.pool.clone(), state.registry()).with_llm(state.llm())
}

/// wiki_list_pages 的瘦身输出：列表不带正文（正文可能很大），全文走 wiki_get_page。
/// content_chars（P1-7）给「值不值得拉全文」的决策依据。
pub fn trim_page(page: serde_json::Value) -> serde_json::Value {
    let mut v = page;
    // 规模化 2026-09-20：list 现在直接返回元数据（带 content_chars、无 content），
    // 字优先取 content_chars；其余调用方（如带正文的行）仍从 content 计算。
    let chars = v["content_chars"]
        .as_i64()
        .or_else(|| v["content"].as_str().map(|c| c.chars().count() as i64))
        .unwrap_or(0);
    v["content"] = serde_json::json!("");
    v["content_omitted"] = serde_json::json!(true);
    v["content_chars"] = serde_json::json!(chars);
    v
}

/// wiki_search 的命中瘦身（P0-2）：正文换片段——命中词附近窗口，找不到词
/// （纯向量召回）取开头 160 字符。全文按需 wiki_get_page。
pub fn snippet_page(page: serde_json::Value, query: &str) -> serde_json::Value {
    let content = page["content"].as_str().unwrap_or("").to_string();
    let mut v = page;
    let chars = content.chars().count();
    v["content"] = serde_json::json!(build_snippet(&content, query));
    v["content_omitted"] = serde_json::json!(true);
    v["content_chars"] = serde_json::json!(chars);
    v
}

/// 命中片段：优先第一个查询词出现的位置，取前后 ~160 字符窗口；
/// 全部词都不在（向量召回路径）→ 取正文开头。
fn build_snippet(content: &str, query: &str) -> String {
    let lower = content.to_lowercase();
    let hit = query
        .split_whitespace()
        .filter(|w| w.chars().count() >= 2)
        .find_map(|w| lower.find(&w.to_lowercase()));
    let start = match hit {
        Some(pos) => {
            let char_pos = lower[..pos].chars().count();
            char_pos.saturating_sub(40)
        }
        None => 0,
    };
    let window: String = content.chars().skip(start).take(160).collect();
    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if content.chars().count() > start + 160 {
        "…"
    } else {
        ""
    };
    format!("{prefix}{window}{suffix}")
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
    /// 批次④：LLM rerank 精排（默认关；开启后 top-20 交模型重排，延迟 +2~8s，质量优先场景开）
    #[schemars(
        description = "可选：LLM rerank 精排（默认关）。开启后 top-20 交 LLM 重排，延迟 +2~8s；追求排序质量的场景开。"
    )]
    pub rerank: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiListPagesParams {
    /// 页型过滤：entity/concept/source/synthesis/comparison/queries/overview/index/analysis 等
    #[schemars(
        description = "可选：按页型过滤。entity=实体, concept=概念, source=来源, synthesis=综合, comparison=对比, queries=查询存档, overview=总览, index=索引, analysis=分析归档。"
    )]
    pub page_type: Option<String>,
    /// 目录子树过滤：folder 精确等于该路径或其下级（folder LIKE '路径/%'）
    #[schemars(
        description = "可选：按目录子树过滤（folder 路径或其下级）。与 GET /wiki/folders 配合可按目录逐个拉取。"
    )]
    pub folder: Option<String>,
    /// keyset 分页游标（D28）：上一页最后一条的 {updated_at ISO8601}|{id}
    #[schemars(
        description = "可选：keyset 分页游标。取上一页最后一条构造 {updated_at ISO8601}|{id}。首查不传；返回条数恰等于 limit 时说明可能还有下一页。"
    )]
    pub cursor: Option<String>,
    /// 可选：返回条数上限；不传 = 全量返回
    #[schemars(description = "可选：返回条数上限。不传 = 全量返回（2026-09-20 起无上限）。")]
    pub limit: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiGetPageParams {
    /// 页面 slug（来自 wiki_list_pages / wiki_search 的返回）
    #[schemars(
        description = "页面 slug。对大小写与空格/连字符差异宽容；传页面标题（title 精确匹配）也可寻址。"
    )]
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
    /// Markdown 正文（[[wikilink]] 库内双链；[[lib/slug]] 跨库引用）
    #[schemars(
        description = "Markdown 正文。[[slug]] 双链本库页面；[[lib/slug]] 跨库引用其他库的页面（目标存在自动建跨库链，缺失 lint 会报）。互链都进链接图。"
    )]
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
pub struct WikiDocumentAddParams {
    /// 二选一：要入库的文本（name 作标题）
    #[schemars(
        description = "二选一：要入库的文本全文（分块+嵌入进原文 RAG，并触发 LLM 织入）。与 url 二选一。"
    )]
    pub text: Option<String>,
    /// 二选一：要抓取的 URL（SSRF 校验）
    #[schemars(description = "二选一：要抓取的 URL（自动抓取→分块→嵌入→织入）。与 text 二选一。")]
    pub url: Option<String>,
    /// 可选：文档名/标题（text 模式作标题；url 模式忽略）。兼容 `title` 字段名。
    #[schemars(description = "可选：文档标题（text 模式）。字段名 `name` 或 `title` 均可。")]
    #[serde(alias = "title")]
    pub name: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiDocumentGetParams {
    /// 文档 id（document_add 返回的 id）
    #[schemars(description = "文档 id（document_add 返回的 id）。status 字段即处理进度。")]
    pub id: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiDocumentsSearchParams {
    /// 原文检索词（chunk 级 FTS+向量混合——搜的是原文分块不是 LLM 生成的页面）
    #[schemars(description = "检索词（chunk 级原文 RAG——与 wiki search 的页面级检索互补）。")]
    pub query: String,
    /// 返回上限（默认 8）
    #[schemars(description = "可选：返回上限。默认 8。")]
    pub limit: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiReviewsParams {
    /// 可选：按状态过滤（open/resolved/dismissed；缺省 open）
    #[schemars(description = "可选：按状态过滤（open/resolved/dismissed；缺省 open）。")]
    pub status: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiReviewResolveParams {
    /// 评审项 id（reviews 返回的 id）
    #[schemars(description = "评审项 id（reviews 返回的 id）。")]
    pub id: String,
    /// 可选：处置动作标签（如 create_page / deep_research / skip——记录到提案）
    #[schemars(description = "可选：处置动作标签（如 create_page / deep_research / skip）。")]
    pub action: Option<String>,
    /// 是否驳回作废（缺省 false = 标记已处理 resolved）
    #[schemars(description = "可选：是否驳回作废（dismiss）；缺省 false = 已处理（resolved）。")]
    pub dismiss: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiMergeParams {
    /// 保留的主页 slug（并入目标）。
    #[schemars(description = "保留的主页 slug（并入目标）。")]
    pub primary: String,
    /// 被合并页 slug（内容并入 primary 后删除，留版本快照——下架不烧书）。
    #[schemars(description = "被合并页 slug（内容并入 primary 后删除，留版本快照——下架不烧书）。")]
    pub duplicate: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiLintDeepParams {
    /// 可选：限定检查的页面 slug 集合（缺省全库非系统页）——控制 LLM 成本
    #[schemars(
        description = "可选：限定检查的页面 slug 集合（缺省全库非系统页）——控制 LLM 成本。"
    )]
    pub slugs: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiArchiveParams {
    /// 归档页 slug
    #[schemars(description = "归档页 slug（仅字母/数字/-/_/·，≤80 字符）。")]
    pub slug: String,
    /// 归档页标题
    #[schemars(description = "归档页标题。")]
    pub title: String,
    /// 归档正文（Markdown；支持 [[wikilink]] 互链）
    #[schemars(description = "归档正文（Markdown，支持 [[wikilink]]）。")]
    pub content: String,
    /// 可选：相关页面 slug 列表——自动建双向 wikilinks（karpathy LLM Wiki：好答案该归档，不该消失在聊天记录里）
    #[schemars(description = "可选：相关页面 slug 列表——自动建双向 wikilinks。")]
    pub related: Option<Vec<String>>,
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

/// 删除 Wiki 页面参数。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiDeletePageParams {
    /// 页面 slug（wiki_list_pages 返回的 slug）
    #[schemars(
        description = "要删除的页面 slug（wiki_list_pages 返回；也接受页面标题）。不可逆——最后状态会留版本快照，可用 restore_version 重建。"
    )]
    pub slug: String,
}

/// 页面版本列表参数（R 报告建议 #5）。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiVersionsParams {
    /// 页面 slug（也接受页面标题）
    #[schemars(
        description = "页面 slug（或标题）。返回该页的历史版本（新→旧，含已删除页的最后状态）。"
    )]
    pub slug: String,
}

/// 读取某版本正文参数（回滚前预览）。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiVersionContentParams {
    /// 页面 slug（或标题）
    #[schemars(description = "页面 slug（或标题）。")]
    pub slug: String,
    /// 版本号（versions 列表里的 version）
    #[schemars(description = "版本号（来自 versions 列表）。")]
    pub version: i32,
}

/// 回滚到历史版本参数。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiRestoreVersionParams {
    /// 页面 slug（或标题）
    #[schemars(description = "页面 slug（或标题）。页面已删除时会从快照重建。")]
    pub slug: String,
    /// 要恢复到的版本号
    #[schemars(
        description = "要恢复到的版本号（来自 versions 列表）。回滚本身也产生新版本，历史不丢。"
    )]
    pub version: i32,
}

/// 无参操作（wiki sources 列表）占位。
#[derive(Serialize, Deserialize, JsonSchema, Default)]
pub struct WikiSourcesParams {}

/// 删除织入原料参数（E7：删除页面后原料成 stale_source 残留——级联清理通道）。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiDeleteSourceParams {
    /// 原料 id（sources 列表返回；级联删除该源产出的页面与任务）
    #[schemars(
        description = "原料 id（sources 列表返回）。级联删除：该源、其任务与由它产出的页面一并删除，不可逆。"
    )]
    pub source_id: String,
}

/// 删除一条文档 RAG 原料（document_add 返回的 id——documents 体系，非 delete_source 的 sources 体系）。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiDocumentDeleteParams {
    /// 文档 id（document_add 返回的 id）
    #[schemars(description = "文档 id（document_add 返回的 id）。删除该文档及其分块/嵌入。")]
    pub doc_id: String,
}

/// graph/lint 等无参操作的占位（单库终局：无库入参）。
#[derive(Serialize, Deserialize, JsonSchema, Default)]
pub struct WikiLibParams {}

/// 知识晋升参数（EN-59）：把项目文档里的一条跨项目知识提炼成 wiki synthesis 页，
/// 服务端自动双向回链（页 frontmatter 带源回链 + 源文档追加 ⛳ 晋升标记 + 登记表）。
/// EN-250：必填 7 项一次给全——project、doc_id、anchor（源文定位短语）、slug、title、content（提炼正文）；library 缺省 main。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiPromoteParams {
    /// 来源项目（名或 id）
    #[schemars(description = "来源项目（名或 id）。")]
    pub project: String,
    /// 来源文档 id
    #[schemars(description = "来源文档 id（project-get 看文档列表取）。")]
    pub doc_id: String,
    /// 源定位（小节标题/行区间说明——回链精度用）
    #[schemars(description = "源定位（小节标题/行区间说明，如「§机器产出原样透传」）。")]
    pub anchor: String,
    /// 目标页 slug
    #[schemars(description = "目标页 slug（仅字母/数字/-/_/·，≤80 字符）。")]
    pub slug: String,
    /// 页标题（提炼后的通用标题，非原文标题）
    #[schemars(description = "页标题（提炼后的通用标题，非原文标题）。")]
    pub title: String,
    /// 提炼后的通用知识正文（markdown，可带 [[wikilink]]）——提炼由调用方完成，服务端不做 LLM 提炼
    #[schemars(
        description = "提炼后的通用知识正文（markdown，可带 [[wikilink]]）——提炼由调用方完成，服务端不做 LLM 提炼。"
    )]
    pub content: String,
}

/// 晋升登记列表参数。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WikiPromotionsParams {
    /// 可选：按来源项目（名或 id）过滤
    #[schemars(description = "可选：按来源项目（名或 id）过滤。")]
    pub project: Option<String>,
}

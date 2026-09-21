//! 渐进式发现（progressive disclosure）分发层：每域一个 MCP 工具，域内操作按需发现。
//!
//! 三层发现：
//! L0 常驻目录——域工具描述尾部自动拼「操作一览」（本模块 ActionDoc 表，单一事实源）；
//! L1 按需手册——`{"action":"help"}` 返回全部操作的参数 JSON Schema；
//! L2 错误自愈——未知 action / 坏参数的报错附带合法清单与 help 提示。
//!
//! 历史上的 53 个扁平工具（memory_search 等）全部收编为「域 + action」：
//! `{"tool":"memory","arguments":{"action":"search","query":"…"}}`。
//! action 名 = 原工具名去域前缀（project_doc_add → doc_add）。

use rmcp::schemars::JsonSchema;
use rmcp::schemars::schema_for;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

use crate::mcp_err;
use rmcp::model::ErrorCode;

/// 域工具统一调用信封：`action` 必填，其余参数按域内操作各自的结构体解析（平铺）。
#[derive(Deserialize, JsonSchema)]
pub struct DomainCall {
    /// 操作名。action="help" 返回本域全部操作与参数手册（渐进式发现）。
    #[schemars(description = "操作名。action=\"help\" 返回本域全部操作与参数手册（渐进式发现）。")]
    pub action: String,
    /// 该操作的参数（平铺在顶层，与 help 返回的参数 schema 一致）。
    #[schemars(
        description = "该操作的参数（平铺在顶层，键名见 action=\"help\" 返回的参数 schema）。"
    )]
    #[serde(flatten)]
    pub args: serde_json::Map<String, Value>,
}

/// 单个 action 的静态文档：L0 目录、help 手册、管理台三级共用同一份表。
pub struct ActionDoc {
    pub action: &'static str,
    /// 一行摘要（L0 目录与 help 共用）
    pub summary: &'static str,
    /// 破坏性操作（不可逆/删除类）——L0 目录标注 + 管理台角标
    pub destructive: bool,
    /// 参数 JSON Schema（help 手册用；从参数结构体生成，与校验同源）
    pub schema: fn() -> Value,
}

macro_rules! action_docs {
    ( $( $action:literal , $destr:literal , $summary:literal => $ty:ty );* $(;)? ) => {
        &[ $( ActionDoc {
            action: $action,
            summary: $summary,
            destructive: $destr,
            schema: || serde_json::to_value(schema_for!($ty)).unwrap_or_default(),
        } ),* ]
    };
}

/// 全部域与各域 action 表（唯一事实源；tools/list 目录、help、管理台都从这里拼）。
pub fn action_docs(domain: &str) -> Option<&'static [ActionDoc]> {
    Some(match domain {
        "memory" => action_docs![
            "context", false, "装载用户记忆上下文包（L3 画像 + L2 场景 + L1 原子 + 实体；会话开场调用一次）" => crate::ContextParams;
            "search", false, "定向检索用户记忆（全文+向量，跨 L1/L2/L3/实体）" => crate::SearchParams;
            "distill_result", false, "蒸馏回执：查一次会话蒸馏产出了哪些原子（id/内容/强度/状态）——写入方可验收" => crate::MemoryDistillResultParams;
            "kv_put", false, "写入/更新结构化精确值（序列号/UUID/IP:PORT 等）——同 key 就地覆盖，蒸馏零介入逐字保存" => crate::MemoryKvPutParams;
            "kv_get", false, "读取结构化精确值（按 key）" => crate::MemoryKvGetParams;
            "kv_list", false, "列出全部 KV 值（按 updated_at 倒序）" => crate::MemoryKvListParams;
            "kv_search", false, "字面量直查 KV（key/value/context ILIKE——精确值不依赖分词）" => crate::MemoryKvSearchParams;
            "remember", false, "一句话记忆（正文字段 text；strength=fact 直写限 120 字；缺省走 auto 蒸馏，产物默认 inference；strength=fact 显式声明用户明示事实则直写落库原话保真）" => crate::RememberParams;
            "correct", false, "更正记忆（快路径取代链）：用户说「你记错了」时用——先 search 定位旧原子，再 correct(target_id, text)；旧原子 superseded 指向新条目，仅 active 非敏感可更正" => crate::CorrectParams;
            "confirm", false, "待审复核通过：摘掉 needs_review 标记（仅 needs_review=true 可处置；AI 代管复核）" => crate::ReviewActionParams;
            "discard", false, "待审复核丢弃：归档该条（仅 needs_review=true 可处置；AI 代管复核）" => crate::ReviewActionParams;
            "persona_edit", false, "编辑画像分面（AI 记忆管家）：version+1 落钉（manually_edited=true），蒸馏对该分面不再覆盖；aspect 七值之一，内容 1~4000 字" => crate::PersonaEditParams;
            "distill", false, "手动触发蒸馏链（撞车守卫：正在蒸馏时只提示不投递）；full=true 附带 consolidate 全量整理；mode=sleep 为记忆巩固预留位" => crate::DistillParams;
            "write_session", false, "写入一段对话到 L0 会话（收尾用；蒸馏自动抽取记忆）" => crate::WriteSessionParams;
            "append_session", false, "向未蒸馏的会话追加轮次（长对话分段落库）" => crate::AppendSessionParams;
            "list_sessions", false, "列出 L0 会话（keyset 分页，可按 agent 过滤）" => crate::ListSessionsParams;
            "get_session", false, "读取一个会话的逐轮原文全文" => crate::GetSessionParams;
            "list_atoms", false, "浏览 L1 原子事实（默认只看 active；可按类型/状态/待审过滤，分页）" => crate::ListAtomsParams;
            "entities", false, "检索实体（人物/项目/主题/群组/地点的横向档案）" => crate::EntitiesParams;
            "forget", true, "遗忘：void 作废（级联归档产物）/ erase 物理删除（需 erase scope）/ restore 撤销 void" => crate::ForgetParams
        ],
        "projects" => action_docs![
            "types", false, "列出项目类型模板（建项目选 type 用）" => crate::ProjectTypesParams;
            "list", false, "列出项目（按创建时间倒序）" => crate::ProjectListParams;
            "get", false, "项目详情（目标/位置/文档索引；默认索引模式不带正文）" => crate::ProjectGetParams;
            "create", false, "新建项目（type 决定初始分类）" => crate::ProjectCreateParams;
            "update", false, "编辑项目（改名/状态/描述/分类；补丁式）" => crate::ProjectUpdateParams;
            "delete", true, "删除项目（级联删除位置与文档，不可逆）" => crate::ProjectDeleteParams;
            "batch_delete", true, "批量删除项目（不可逆）" => crate::ProjectBatchDeleteParams;
            "location_add", false, "登记项目在主机上的位置（多主机登记制）" => crate::ProjectLocationAddParams;
            "location_update", false, "编辑已登记的位置" => crate::ProjectLocationUpdateParams;
            "location_delete", true, "删除一条位置登记" => crate::ProjectLocationDeleteParams;
            "doc_add", false, "项目下新增分类文档（Markdown）" => crate::ProjectDocAddParams;
            "doc_get", false, "读项目文档（全文或按行区间精读）" => crate::ProjectDocGetParams;
            "doc_search", false, "grep 式跨文档按行检索（需 project_id/project_name 定位项目，先 list；定位到哪篇哪行）" => crate::ProjectDocSearchParams;
            "doc_patch", false, "行级补丁（replace/insert/delete 一个行区间——改长文档不必取全文重发）" => crate::ProjectDocPatchParams;
            "doc_update", false, "编辑项目文档（补丁式）" => crate::ProjectDocUpdateParams;
            "doc_delete", true, "删除项目文档（不可逆）" => crate::ProjectDocDeleteParams;
            "file_put", false, "写项目文件（架构图 HTML/配置/报告等制品；同 name 覆盖 version+1，旧版进快照）" => crate::ProjectFileUpsertParams;
            "file_get", false, "读项目文件（当前或指定版本；支持 project_id/project_name 定位）" => crate::ProjectFileRefParams;
            "file_list", false, "列出项目文件（name/mime/version/大小；不含内容）" => crate::ProjectFileListParams;
            "file_delete", true, "删除项目文件（历史快照级联删，不可逆）" => crate::ProjectFileDeleteParams
        ],
        "skills" => action_docs![
            "list", false, "列出技能（q/tag/enabled 过滤，不含正文；kind=script 的条目带 local_path 指针）" => crate::SkillsListParams;
            "get", false, "读技能全文（script 型从 local_path 现读，指针失效报错）" => crate::SkillsGetParams;
            "file_get", false, "读技能附属文件（scripts/references 等；script 型不可用——文件在本地，系统只存指针）" => crate::SkillsFileGetParams;
            "file_put", false, "写技能附属文件（同路径覆盖；SKILL.md 本体走 update；script 型不可用；text 型禁 .py/.sh 等脚本后缀）" => crate::SkillsFilePutParams;
            "create", false, "沉淀新技能（默认 text 整体入库；带 .py/.sh 等脚本的用 kind=script + local_path 存本地指针）" => crate::SkillsCreateParams;
            "update", false, "更新技能（语义变更自动留版本快照；script 型改正文拒绝、可改 origin/repo_url/local_path）" => crate::SkillsUpdateParams;
            "versions", false, "版本快照列表（改坏前看历史 / 找回滚 revision_id；script 型不可用——版本归本地 git）" => crate::SkillsVersionsParams;
            "restore", false, "回滚到历史版本（回滚本身也留快照；script 型不可用）" => crate::SkillsRestoreParams;
            "delete", true, "删除技能（级联删版本快照，不可逆；仅限用户明确要求）" => crate::SkillsDeleteParams;
            "import", false, "导入现成 SKILL.md（frontmatter 容错解析；导入为 text 型）" => crate::SkillsImportParams
        ],
        "wiki" => action_docs![
            "search", false, "Wiki 检索（FTS + 向量融合；命中带片段，全文按需 get_page；按库）" => crate::wiki::WikiSearchParams;
            "list_pages", false, "浏览页面列表（可按页型过滤；不含正文）" => crate::wiki::WikiListPagesParams;
            "get_page", false, "读页面全文（含 frontmatter 与版本）" => crate::wiki::WikiGetPageParams;
            "write_page", false, "写/覆盖一个页面（Markdown + [[wikilink]]；覆盖前先 get_page，旧文自动留版本快照）" => crate::wiki::WikiWritePageParams;
            "ingest", false, "整篇源文本织入 Wiki（异步 LLM 流水线，sha 去重）" => crate::wiki::WikiIngestParams;
            "archive_query", false, "把一条问答存档为 queries 页（幂等跳过重复）" => crate::wiki::WikiArchiveQueryParams;
            "versions", false, "页面版本列表（含已删除页的最后状态快照）" => crate::wiki::WikiVersionsParams;
            "version_content", false, "读某版本快照的正文（回滚前预览）" => crate::wiki::WikiVersionContentParams;
            "restore_version", false, "回滚到历史版本（已删除页面从快照重建）" => crate::wiki::WikiRestoreVersionParams;
            "sources", false, "列出织入原料（wiki_sources 及其状态；stale_source 清理的入口）" => crate::wiki::WikiSourcesParams;
            "delete_source", true, "删除一条织入原料及其全部产出（级联，不可逆）" => crate::wiki::WikiDeleteSourceParams;
            "graph", false, "Wiki 链接图全貌（节点/边/社区划分；按库）" => crate::wiki::WikiLibParams;
            "lint", false, "Wiki 体检（死链/孤页/缺源；只报告不修改；按库）" => crate::wiki::WikiLibParams;
            "lint_deep", false, "语义 lint（LLM 深度检查页面间矛盾/过时声明/缺页概念；异步任务，产出入人审队列；slugs 可限定范围控成本）" => crate::wiki::WikiLintDeepParams;
            "merge", false, "合并页面：duplicate 并入 primary（冗余丢弃或内容并入 + 全库链接改指 + 快照兜底删除）——处置重复页 flag 用" => crate::wiki::WikiMergeParams;
            "document_add", false, "入库文档（text 或 url）——分块+嵌入进原文 RAG 并触发织入；幂等去重" => crate::wiki::WikiDocumentAddParams;
            "document_get", false, "文档状态（status/error 即处理进度）" => crate::wiki::WikiDocumentGetParams;
            "documents_search", false, "原文检索（chunk 级 FTS+向量混合——与页面级 search 互补）" => crate::wiki::WikiDocumentsSearchParams;
            "reviews", false, "人审队列：列出审查项（lint 深检/织入期 LLM 旗标的发现——kind/payload/来源；status 可过滤 open/resolved/dismissed，缺省 open）" => crate::wiki::WikiReviewsParams;
            "review_resolve", false, "处置评审项：标记已处理（resolved）或驳回作废（dismiss），可附动作标签" => crate::wiki::WikiReviewResolveParams;
            "index", false, "内容目录（按页型分组的全库目录：slug/标题/入链数/首段摘要；只读动态聚合）" => crate::wiki::WikiLibParams;
            "archive", false, "问答/分析产物归档为 analysis 页（related 自动建双向 wikilinks——好答案不该消失在聊天记录里）" => crate::wiki::WikiArchiveParams;
            "purpose", false, "读取库的方向意图（每库一份——写页前先读，避免写跑题）" => crate::wiki::WikiLibParams;
            "insights", false, "列出库的洞察（AI 评审产出的观察项，可与 reviews 对照看）" => crate::wiki::WikiLibParams;
            "promote", false, "知识晋升（EN-59）：把项目文档里的一条跨项目知识提炼成 synthesis 页（frontmatter 带源回链）+ 源文档自动追加 ⛳ 标记——提炼由调用方完成" => crate::wiki::WikiPromoteParams;
            "promotions", false, "晋升登记列表（谁家的哪些知识晋升成了 wiki 页；按项目过滤）" => crate::wiki::WikiPromotionsParams;
            "delete_page", true, "删除页面（连带清理双向 wikilink；最后状态留快照可重建）" => crate::wiki::WikiDeletePageParams
        ],
        "todos" => action_docs![
            "add", false, "记一条待办（kind 固定 todo——行动项/灵感速记；开工单用 tickets 域）" => crate::TodoAddParams;
            "list", false, "待办列表（仅 kind=todo；status/priority/tag/q 过滤；默认摘要模式 brief 只回短号/标题/状态/优先级/关联计数）" => crate::TodoListParams;
            "link", false, "建立关联：blocked_by（被阻塞）/ relates_to（相关）/ parent（父子），幂等；from/to 支持 EN-短号" => crate::TodoLinkParams;
            "unlink", false, "解除关联" => crate::TodoUnlinkParams;
            "links", false, "双向关联列表（含 EN-短号与方向）——「谁阻塞我」反查入口" => crate::TodoLinksParams;
            "get", false, "详情" => crate::TodoIdParams;
            "done", false, "标记完成（记 done_at）" => crate::TodoIdParams;
            "update", false, "编辑（标题/详情/优先级/状态/截止）" => crate::TodoUpdateParams;
            "delete", true, "删除待办（不可逆）" => crate::TodoIdParams
        ],
        "tickets" => action_docs![
            "add", false, "开工单（kind 固定 ticket——结构化问题跟踪；建议填 severity/symptom/acceptance）" => crate::TicketAddParams;
            "list", false, "工单列表（仅 kind=ticket；status 含 confirmed/in_progress/resolved/verified 六态；默认摘要模式 brief 只回短号/标题/状态/分级/关联计数）" => crate::TicketListParams;
            "link", false, "建立关联：blocked_by（被阻塞）/ relates_to（相关）/ parent（父子），幂等；from/to 支持 EN-短号" => crate::TodoLinkParams;
            "unlink", false, "解除关联" => crate::TodoUnlinkParams;
            "links", false, "双向关联列表（含 EN-短号与方向）——「谁阻塞我」反查入口" => crate::TodoLinksParams;
            "get", false, "详情（含 severity/symptom/acceptance/resolution）" => crate::TodoIdParams;
            "update", false, "编辑与状态流转（confirmed/in_progress/resolved/verified/archived；工单字段 severity/symptom/acceptance/resolution）" => crate::TodoUpdateParams;
            "delete", true, "删除工单（不可逆）" => crate::TodoIdParams
        ],
        "codegraph" => action_docs![
            "list", false, "列出已注册代码库（注册状态/索引规模/当下可用性 usable）" => crate::CgNoParams;
            "gc", false, "失效条目对账（路径已不存在/索引产物已丢失的条目标为 error；可重新 index 恢复）" => crate::CgNoParams;
            "register", false, "注册代码库（**只接受 git 仓库地址**，如 https://github.com/you/repo）——注册即 git clone 到默认 <数据根>/codegraph/<项目名>/（可用 dest_parent 指定父目录，须在白名单根内）并**自动入队建索引**；本地路径已不支持（本机源码改用 upload）" => crate::CgRegisterParams;
            "query", false, "代码图谱查询（search/explore大纲/node/callers/callees/impact/full_graph全图）" => crate::CgQueryParams;
            "index", false, "建索引/重建索引（异步 job）" => crate::CgNameParams;
            "sync", false, "增量同步索引（小改动后刷新）" => crate::CgNameParams;
            "upload", true, "产物上传（公网模型）——客户端本机 codegraph index 后上传 db(base64)+HEAD；服务端只存+声明式新鲜度，无代码无 git 凭证" => crate::CgUploadParams;
            "delete", true, "注销代码图谱项目（删注册与产物；默认落盘的服务端自建目录连目录清，自定义落盘目录保留、需手动清理）" => crate::CgNameParams
        ],
        "jobs" => action_docs![
            "list", false, "列出异步任务（可按 kind/status 过滤——codegraph index/sync 与 gc 自愈的 job 都在这）" => crate::jobs::JobsListParams;
            "get", false, "查任务详情（状态/错误/进度/attempts——job_id 从 codegraph index/sync 返回拿）" => crate::jobs::JobsGetParams;
            "events", false, "任务事件时间线（增量轮询）" => crate::jobs::JobsEventsParams;
            "revive", true, "复活 dead/failed 任务重跑（仅管理员——amk_ key 会收到明确拒绝）" => crate::jobs::JobsReviveParams
        ],
        _ => return None,
    })
}

/// 域工具名（= scope 名，projects 例外——scope 叫 project）。
pub const DOMAIN_TOOLS: &[&str] = &[
    "memory",
    "projects",
    "skills",
    "wiki",
    "todos",
    "tickets",
    "codegraph",
    "jobs",
];

pub fn is_domain_tool(name: &str) -> bool {
    DOMAIN_TOOLS.contains(&name)
}

/// action 级开关键（disabled_tools 里用 `域.action` 表示停用某个操作）。
pub fn action_key(domain: &str, action: &str) -> String {
    format!("{domain}.{action}")
}

/// 域参数解析：把 DomainCall 平铺参数还原成该操作的强类型结构体（历史参数结构体全复用）。
pub fn from_args<T: serde::de::DeserializeOwned>(
    domain: &str,
    action: &str,
    args: serde_json::Map<String, Value>,
) -> Result<T, rmcp::ErrorData> {
    serde_json::from_value(Value::Object(args)).map_err(|e| {
        mcp_err(
            ErrorCode::INVALID_PARAMS,
            format!("{domain}.{action} 参数错误：{e}。用 action=\"help\" 查看该操作的参数说明。"),
        )
    })
}

/// 未知 action：报错即发现（L2）——列出全部合法操作。
pub fn unknown_action(domain: &str, action: &str) -> rmcp::ErrorData {
    let known: Vec<&str> = action_docs(domain)
        .map(|docs| docs.iter().map(|d| d.action).collect())
        .unwrap_or_default();
    mcp_err(
        ErrorCode::INVALID_PARAMS,
        format!(
            "{domain} 域没有操作 {action:?}。可用：{}。用 action=\"help\" 看参数手册。",
            known.join("、")
        ),
    )
}

// ---------- 动作级权限（公网多Agent P001 步骤3） ----------

/// 写类 action 清单（`:ro` 只读变体的拒绝面）。
///
/// 与 [`is_read_action`] 互补——完备性由下方测试保证：新增 action 落 [`action_docs`]
/// 表时两边都要归类，漏归类测试红（CI 抓住，`:ro` 机制不会静默漏权）。
pub fn is_write_action(domain: &str, action: &str) -> bool {
    matches!(
        (domain, action),
        (
            "memory",
            "kv_put"
                | "remember"
                | "correct"
                | "confirm"
                | "discard"
                | "persona_edit"
                | "distill"
                | "write_session"
                | "append_session"
                | "forget"
        ) | (
            "projects",
            "create"
                | "update"
                | "delete"
                | "batch_delete"
                | "location_add"
                | "location_update"
                | "location_delete"
                | "doc_add"
                | "doc_patch"
                | "doc_update"
                | "doc_delete"
                | "file_put"
                | "file_delete"
        ) | (
            "skills",
            "create" | "update" | "import" | "restore" | "delete" | "file_put"
        ) | (
            "wiki",
            "write_page"
                | "ingest"
                | "archive_query"
                | "restore_version"
                | "delete_source"
                | "lint_deep"
                | "document_add"
                | "review_resolve"
                | "archive"
                | "promote"
                | "delete_page"
                | "merge"
        ) | (
            "todos",
            "add" | "link" | "unlink" | "done" | "update" | "delete"
        ) | ("tickets", "add" | "link" | "unlink" | "update" | "delete")
            | (
                "codegraph",
                "register" | "index" | "sync" | "gc" | "delete" | "upload"
            )
            | ("jobs", "revive")
    )
}

/// 读类 action 清单（`:ro` 可用面）。与 [`is_write_action`] 互补。
pub fn is_read_action(domain: &str, action: &str) -> bool {
    matches!(
        (domain, action),
        (
            "memory",
            "context"
                | "search"
                | "distill_result"
                | "kv_get"
                | "kv_list"
                | "kv_search"
                | "list_sessions"
                | "get_session"
                | "list_atoms"
                | "entities"
        ) | (
            "projects",
            "types" | "list" | "get" | "doc_get" | "doc_search" | "file_get" | "file_list"
        ) | ("skills", "list" | "get" | "file_get" | "versions")
            | (
                "wiki",
                "search"
                    | "list_pages"
                    | "get_page"
                    | "versions"
                    | "version_content"
                    | "sources"
                    | "graph"
                    | "lint"
                    | "document_get"
                    | "documents_search"
                    | "reviews"
                    | "index"
                    | "purpose"
                    | "insights"
                    | "promotions"
            )
            | ("todos", "list" | "links" | "get")
            | ("tickets", "list" | "links" | "get")
            | ("codegraph", "list" | "query")
            | ("jobs", "list" | "get" | "events")
    )
}

/// 动作级权限检查：Admin/全量 scope 直过；`:ro` 变体只放行读类动作。
///
/// None 不在此拒——缺 scope 的拒绝保持在 handler 内的 require_*（原报错语义不变）。
/// `tool` 是域工具名（memory/projects/…，projects 与 scope 单复数差异由 tool_scope 换算）。
pub fn check_action_access(
    principal: &engram_core::auth::Principal,
    tool: &str,
    action: &str,
) -> Result<(), rmcp::ErrorData> {
    use engram_core::auth::DomainAccess;
    let scope = crate::tool_scope(tool);
    match principal.domain_access(scope) {
        DomainAccess::Full | DomainAccess::None => Ok(()),
        DomainAccess::ReadOnly => {
            if is_write_action(tool, action) {
                let reads: Vec<&str> = action_docs(tool)
                    .map(|docs| {
                        docs.iter()
                            .filter(|d| is_read_action(tool, d.action))
                            .map(|d| d.action)
                            .collect()
                    })
                    .unwrap_or_default();
                Err(mcp_err(
                    ErrorCode::INVALID_REQUEST,
                    format!(
                        "权限不足：key 的 {scope} scope 是只读变体（:ro），不能调用 {tool}.{action}。可用只读动作：{}。需要写权限请用全量 {scope} scope 的 key。",
                        reads.join("、")
                    ),
                ))
            } else {
                Ok(())
            }
        }
    }
}

/// L0 目录：一行一操作的紧凑清单（拼进域工具描述尾部，tools/list 与管理台共用）。
pub fn render_catalog(domain: &str, disabled: &[String]) -> Option<String> {
    let docs = action_docs(domain)?;
    let mut lines: Vec<String> = Vec::new();
    for d in docs {
        if disabled.iter().any(|x| x == &action_key(domain, d.action)) {
            continue;
        }
        let tag = if d.destructive { "【破坏性】" } else { "" };
        lines.push(format!("- {}：{}{}", d.action, tag, d.summary));
    }
    if lines.is_empty() {
        return None;
    }
    Some(format!(
        "【本域操作 {} 个】调用形态 {{\"action\":\"…\",…}}；参数细节用 action=\"help\" 取\n{}",
        lines.len(),
        lines.join("\n")
    ))
}

/// L1 手册：全部操作的参数 JSON Schema（模型按需取用的一轮发现）。
/// 已停用的操作不出现（对 AI 隐身，与 tools/list 同哲学）。
pub fn render_manual(domain: &str, disabled: &[String]) -> Value {
    let docs = action_docs(domain).unwrap_or(&[]);
    let actions: Vec<Value> = docs
        .iter()
        .filter(|d| !disabled.iter().any(|x| x == &action_key(domain, d.action)))
        .map(|d| {
            let mut v = json!({
                "action": d.action,
                "summary": d.summary,
                "destructive": d.destructive,
            });
            if let Ok(s) = serde_json::to_value((d.schema)()) {
                v["parameters"] = s;
            }
            v
        })
        .collect();
    json!({
        "domain": domain,
        "how_to_call": format!("{{\"action\":\"<操作名>\", ...该操作的参数（平铺）}}；本返回即 {domain} 域全部可用操作"),
        // 验收反馈：search_all 是独立工具，不在任何域 help 里——每份手册顶部指路，
        // 想跨域扫一遍时不用翻 tools/list
        "cross_domain_hint": "另有独立工具 search_all（非本域操作）：一次查询并发 memory/wiki/skills/todos/projects 各回 top-k 摘要——不确定信息在哪域时用",
        "actions": actions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DomainCall 的 inputSchema 根类型须为 object 且带 action 属性
    /// （MCP 规范要求；flatten 生成 permissive additionalProperties）。
    #[test]
    fn domain_call_schema_is_object_with_action() {
        let v = serde_json::to_value(schema_for!(DomainCall)).unwrap();
        assert_eq!(v["type"], "object", "根类型须为 object：{v}");
        assert!(
            v["properties"]["action"].is_object(),
            "action 属性须存在：{v}"
        );
    }

    /// 平铺参数解析：action 之外的全部键进 args，还原成强类型结构体。
    #[test]
    fn flat_args_parse_into_typed_struct() {
        let call: DomainCall =
            serde_json::from_value(json!({"action": "add", "title": "买牛奶", "priority": "high"}))
                .unwrap();
        assert_eq!(call.action, "add");
        assert_eq!(call.args["title"], "买牛奶");
        let parsed: crate::TodoAddParams = from_args("todos", "add", call.args).unwrap();
        assert_eq!(parsed.title, "买牛奶");
        assert_eq!(parsed.priority.as_deref(), Some("high"));
    }

    /// 六域全表自检：action 非空、schema 可序列化、无重复 action 名。
    #[test]
    fn all_domains_have_consistent_docs() {
        for domain in DOMAIN_TOOLS {
            let docs = action_docs(domain).unwrap_or_else(|| panic!("{domain} 缺 action 表"));
            assert!(!docs.is_empty());
            let mut seen = std::collections::HashSet::new();
            for d in docs {
                assert!(seen.insert(d.action), "{domain} 重复 action：{}", d.action);
                assert!(
                    (d.schema)().is_object(),
                    "{domain}.{} schema 须为 object",
                    d.action
                );
            }
        }
    }

    /// 目录/手册：未知域 None；停用操作被过滤；unknown 报错带合法清单。
    #[test]
    fn catalog_and_error_helpers() {
        assert!(action_docs("nope").is_none());
        let cat = render_catalog("todos", &["todos.delete".to_string()]).unwrap();
        assert!(!cat.contains("- delete："), "停用操作应从目录隐身：{cat}");
        assert!(cat.contains("- add："));
        let manual = render_manual("todos", &["todos.delete".to_string()]);
        let actions = manual["actions"].as_array().unwrap();
        assert_eq!(
            actions.len(),
            8,
            "停用操作应从手册隐身（todos 原有 5 + link/unlink/links）："
        );
        let err = unknown_action("todos", "nope");
        assert!(
            err.message.contains("add"),
            "报错应列合法操作：{}",
            err.message
        );
        assert!(
            err.message.contains("help"),
            "报错应提示 help：{}",
            err.message
        );
    }

    /// 完备性：每个域的每个 action 必须恰好归入读/写一类。
    /// 新增 action 漏归类 → 本测试红（CI 抓住，`:ro` 机制不会静默漏权）。
    #[test]
    fn action_rw_classification_covers_everything() {
        for domain in DOMAIN_TOOLS {
            let docs = action_docs(domain).unwrap_or_else(|| panic!("域 {domain} 缺 action 表"));
            for d in docs {
                let w = is_write_action(domain, d.action);
                let r = is_read_action(domain, d.action);
                assert!(
                    w ^ r,
                    "动作 {domain}.{} 归类缺失或重复（write={w} read={r}）——新增 action 请同步归类到 is_write_action / is_read_action",
                    d.action
                );
            }
        }
    }
}

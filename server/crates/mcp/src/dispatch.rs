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
    #[serde(default)]
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
            "remember", false, "存=一句话记忆。mode：atom（默认）/ session（成段会话）/ append（续写会话）/ kv（精确值逐字保存）；其余参数同名平铺。T018：旧名 write_session/append_session/kv_put 已删不兼容" => crate::RememberParams;
            "recall", false, "找=记忆检索总入口。mode：search（默认）/ context（开场装载）/ entities / kv_get / kv_search；其余参数同名平铺。T018：旧名已删不兼容" => crate::MemoryRecallParams;
            "browse", false, "翻=清单浏览。mode：atoms（默认）/ sessions / session / kv / scenarios（场景清单）/ persona（画像分面）；其余参数同名平铺。T018：旧名已删不兼容" => crate::MemoryBrowseParams;
            "revise", false, "改=纠错与修订。mode：correct（默认，取代链留痕）/ persona（分面编辑后蒸馏不覆盖）/ archive（原子归档）；其余参数同名平铺。T018：旧名已删不兼容；实体合并按域界在 circles" => crate::MemoryReviseParams;
            "review", false, "审=蒸馏与复核。mode：distill（默认）/ result（蒸馏回执）/ confirm / discard / duplicates（原子重复检测）；其余参数同名平铺。T018：旧名已删不兼容；实体重复检测按域界在 circles" => crate::MemoryReviewParams;
            "forget", true, "忘=遗忘。mode：void（默认，作废会话级联归档产物）/ erase（物理删除，需 erase scope）/ restore（撤销 void）/ kv（删 KV 精确值，需 original scope）" => crate::ForgetParams;
        ],
        "projects" => action_docs![
            "types", false, "列出项目**场景**模板（dev=开发 / ops=运维 / research=调研 / study=学习 / life=生活 / create=创作，各带预设文档分类）" => crate::ProjectTypesParams;
            "list", false, "列出项目（按创建时间倒序；可按场景 type 过滤）" => crate::ProjectListParams;
            "get", false, "项目详情（目标/位置/文档索引；默认索引模式不带正文）" => crate::ProjectGetParams;
            "create", false, "新建项目（type=场景，决定初始文档分类；不确定先跑 types）" => crate::ProjectCreateParams;
            "update", false, "编辑项目（改名/状态/描述/分类；补丁式）" => crate::ProjectUpdateParams;
            "delete", true, "删除项目（级联删除位置与文档，不可逆）" => crate::ProjectDeleteParams;
            "batch_delete", true, "批量删除项目（不可逆）" => crate::ProjectBatchDeleteParams;
            "location_add", false, "登记项目在主机上的位置（多主机登记制）" => crate::ProjectLocationAddParams;
            "location_get", false, "读单个位置登记（详情：host/ip/os/path/用途/关联资产）" => crate::ProjectLocationGetParams;
            "location_update", false, "编辑已登记的位置" => crate::ProjectLocationUpdateParams;
            "location_delete", true, "删除一条位置登记" => crate::ProjectLocationDeleteParams;
            "link", false, "建工作线关联（part_of 隶属 / related 相关；自环与同向同类重复被拒）" => crate::ProjectLinkAddParams;
            "unlink", true, "解绑一条工作线关联（按 link_id）" => crate::ProjectLinkRefParams;
            "links", false, "列某项目的关系（两向合并：隶属 / 下属 / 相关）" => crate::ProjectLinksParams;
            "doc_add", false, "项目下新增分类文档（Markdown）" => crate::ProjectDocAddParams;
            "doc_get", false, "读项目文档（全文或按行区间精读）" => crate::ProjectDocGetParams;
            "doc_search", false, "grep 式跨文档按行检索（需 project_id/project_name 定位项目，先 list；定位到哪篇哪行）" => crate::ProjectDocSearchParams;
            "doc_patch", false, "行级补丁（replace/insert/delete 一个行区间；行号定位或 anchor 锚点——原文短语按行包含匹配唯一命中才执行，串行多 patch 不漂移）" => crate::ProjectDocPatchParams;
            "doc_update", false, "编辑项目文档（补丁式）" => crate::ProjectDocUpdateParams;
            "doc_delete", true, "删除项目文档（不可逆）" => crate::ProjectDocDeleteParams;
            "file_put", false, "写项目文件（架构图 HTML/配置/报告等制品；同 name 覆盖 version+1，旧版进快照）" => crate::ProjectFileUpsertParams;
            "file_get", false, "读项目文件（当前或指定版本；支持 project_id/project_name 定位）" => crate::ProjectFileRefParams;
            "file_list", false, "列出项目文件（name/mime/version/大小；不含内容）" => crate::ProjectFileListParams;
            "file_delete", true, "删除项目文件（历史快照级联删，不可逆）" => crate::ProjectFileDeleteParams
        ],
        "assets" => action_docs![
            "kinds", false, "列出资产类型（建档选 kind 用）：host=主机 / cloud=云实例 / domain=域名 / account=账号 / device=设备 / other=其他" => crate::AssetKindsParams;
            "list", false, "列出资产台账（可按类型过滤 / 按 名称·别名·IP 检索）" => crate::AssetListParams;
            "get", false, "读资产详情（本体 + 被哪些项目位置引用）" => crate::AssetGetParams;
            "add", false, "建档一台资产（主机/云实例/域名/账号/设备；别名收历史写法，引用匹配也认它）" => crate::AssetAddParams;
            "update", false, "编辑资产（补丁式；aliases 传了就整体替换）" => crate::AssetUpdateParams;
            "delete", true, "删除资产（被项目位置引用的会被拒绝——先解绑，不可逆）" => crate::AssetDeleteParams;
            "runbook", false, "读资产运行手册（Markdown 全文——硬件/网络/服务/端口/变更/踩坑；看一眼即知这台机器什么情况）" => crate::AssetRunbookParams;
            "runbook_save", true, "保存运行手册（Markdown 整体替换；旧文自动入修订史——错改可回滚）" => crate::AssetRunbookSaveParams;
            "runbook_versions", false, "运行手册修订史清单（新→旧；old_runbook_md=该次保存前的正文）" => crate::AssetRunbookVersionsParams;
            "runbook_restore", true, "回滚运行手册到某修订（回滚前正文先入史——反复横跳可逆）" => crate::AssetRunbookRestoreParams
        ],
        // EN-252：skills 域裁撤（原 action_docs 块移除；存量已迁 wiki/projects/本地 git）
        "circles" => action_docs![
            "graph", false, "实体坐标系全景（nodes+共现边+类型化关系）" => crate::CirclesGraphParams;
            "entity", false, "实体详情（档案+原子时间线+关系+场景；superseded 旧原子默认不显示）" => crate::CirclesEntityParams;
            "create", false, "建实体（同名同类型幂等返回已有）" => crate::CirclesCreateParams;
            "update", false, "改实体名/摘要（摘要手编留修订史）" => crate::CirclesUpdateParams;
            "relate", true, "建类型化关系（member_of/located_in/works_on/part_of/related_to；同向同类型 upsert）" => crate::CirclesRelateParams;
            "unrelate", true, "删关系" => crate::CirclesUnrelateParams;
            "relations", false, "实体关系清单（双向）" => crate::CirclesRelationsParams;
            "forget", true, "归档式遗忘（T021/Q004）：实体归档+挂链活跃原子级联归档，可恢复——默认安全路径" => crate::CirclesTargetParams;
            "delete", true, "物理删除实体（Q004：档案蒸发，显式清理意图，不可恢复）" => crate::CirclesTargetParams;
            "merge", true, "实体合并（T021）：from 墓碑化，原子改挂到 into（档案延续）" => crate::CirclesMergeParams;
            "attach", false, "挂原子到实体（T021：atom_entities 边管理归 circles）" => crate::CirclesAtomLinkParams;
            "detach", true, "从实体摘原子" => crate::CirclesAtomLinkParams;
            "duplicates", false, "实体重复检测（T021 从 memory 挪入）：同名同类异形行" => crate::CirclesGraphParams
        ],
        "credentials" => action_docs![
            "put", true, "写入/更新凭据（值加密落库；同名换值清零旧取用审计）" => crate::CredentialPutParams;
            "get", false, "按名取用（返回直接可用值，取用留审计痕）" => crate::CredentialGetParams;
            "list", false, "台账列表（元数据，永不回显值）" => crate::CredentialListParams;
            "reads", false, "取用审计流水（谁/何时，最近在前）" => crate::CredentialReadsParams;
            "delete", true, "删除凭据（级联清取用审计，不可逆）" => crate::CredentialDeleteParams
        ],
        "wiki" => action_docs![
                  "search", false, "Wiki 检索（FTS + 向量融合；命中带片段，全文按需 get_page；按库）" => crate::wiki::WikiSearchParams;
                  "list_pages", false, "浏览页面列表（可按页型过滤；不含正文）" => crate::wiki::WikiListPagesParams;
                  "get_page", false, "读页面全文（含 frontmatter 与版本）" => crate::wiki::WikiGetPageParams;
                  "write_page", false, "写/覆盖一个页面（正文字段 content，Markdown + [[wikilink]]；覆盖前先 get_page，旧文自动留版本快照）" => crate::wiki::WikiWritePageParams;
                  "archive_query", false, "把一条问答存档为 queries 页（幂等跳过重复）" => crate::wiki::WikiArchiveQueryParams;
                  "versions", false, "页面版本列表（含已删除页的最后状态快照）" => crate::wiki::WikiVersionsParams;
                  "version_content", false, "读某版本快照的正文（回滚前预览）" => crate::wiki::WikiVersionContentParams;
                  "restore_version", false, "回滚到历史版本（已删除页面从快照重建）" => crate::wiki::WikiRestoreVersionParams;
                  "sources", false, "列出来源记录（wiki_sources 及其状态；delete_source 级联清理的入口）" => crate::wiki::WikiSourcesParams;
                  "delete_source", true, "删除一条来源记录及其全部产出（级联，不可逆）" => crate::wiki::WikiDeleteSourceParams;
                  "graph", false, "Wiki 链接图全貌（节点/边/社区划分；按库）" => crate::wiki::WikiLibParams;
                  "lint", false, "Wiki 体检（死链/孤页/缺源；只报告不修改；按库）" => crate::wiki::WikiLibParams;
                  "lint_deep", false, "语义 lint（LLM 深度检查页面间矛盾/过时声明/缺页概念；异步入队，结果随 job report 查看，日志页可按 job_id 跟踪；slugs 可限定范围控成本）" => crate::wiki::WikiLintDeepParams;
                  "merge", false, "合并页面：duplicate 并入 primary（冗余丢弃或内容并入 + 全库链接改指 + 快照兜底删除）——处置重复页用" => crate::wiki::WikiMergeParams;
                  "ingest", false, "喂原料给 wiki 维护 Agent Harness（url 或 text 二选一）——harness 自主抓取/检索/建页/互链（异步 job，返回 job_id）" => crate::wiki::WikiIngestParams;
        "document_add", false, "入库文档（text 或 url）——分块+嵌入进原文 RAG（完成后维护 Agent 自动接力）；幂等去重" => crate::wiki::WikiDocumentAddParams;
                  "document_delete", true, "删除一条入库文档及其分块/嵌入（document_add 返回的 id；documents 体系，非 delete_source）" => crate::wiki::WikiDocumentDeleteParams;
                  "document_get", false, "文档状态（status/error 即处理进度）" => crate::wiki::WikiDocumentGetParams;
                  "documents_search", false, "原文检索（chunk 级 FTS+向量混合——与页面级 search 互补）" => crate::wiki::WikiDocumentsSearchParams;
                  "index", false, "内容目录（按页型分组的全库目录：slug/标题/入链数/首段摘要；只读动态聚合）" => crate::wiki::WikiLibParams;
                  "archive", false, "问答/分析产物归档为 analysis 页（related 自动建双向 wikilinks——好答案不该消失在聊天记录里）" => crate::wiki::WikiArchiveParams;
                  "purpose", false, "读取库的方向意图（每库一份——写页前先读，避免写跑题）" => crate::wiki::WikiLibParams;
                  "insights", false, "列出库的洞察（图结构异常与稀疏社区的观察项）" => crate::wiki::WikiLibParams;
                  "promote", false, "知识晋升（EN-59）：把项目文档里的一条跨项目知识提炼成 synthesis 页（frontmatter 带源回链）+ 源文档自动追加 ⛳ 标记——提炼由调用方完成。必填 7 项一次给全：project、doc_id、anchor（源文定位短语）、slug、title、content（提炼正文）、library 缺省 main" => crate::wiki::WikiPromoteParams;
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
            "update", false, "编辑（标题/详情/优先级/状态/截止）；clear=[due_at/project_hint] 显式清空字段" => crate::TodoUpdateParams;
            "delete", true, "删除待办（不可逆）" => crate::TodoIdParams
        ],
        "tickets" => action_docs![
            "add", false, "开工单（kind 固定 ticket——结构化问题跟踪；建议填 severity/symptom/acceptance）" => crate::TicketAddParams;
            "list", false, "工单列表（仅 kind=ticket；status 含 confirmed/in_progress/resolved/verified 六态；默认摘要模式 brief 只回短号/标题/状态/分级/关联计数）" => crate::TicketListParams;
            "link", false, "建立关联：blocked_by（被阻塞）/ relates_to（相关）/ parent（父子），幂等；from/to 支持 EN-短号" => crate::TodoLinkParams;
            "unlink", false, "解除关联" => crate::TodoUnlinkParams;
            "links", false, "双向关联列表（含 EN-短号与方向）——「谁阻塞我」反查入口" => crate::TodoLinksParams;
            "get", false, "详情（含 severity/symptom/acceptance/resolution）" => crate::TodoIdParams;
            "update", false, "编辑与状态流转（confirmed/in_progress/resolved/verified/archived；工单字段 severity/symptom/acceptance/resolution）；clear=[due_at/severity/project_hint] 显式清空字段" => crate::TodoUpdateParams;
            "events", false, "活动时间线（状态流转 event 自动留痕 + 评论 comment，升序）" => crate::TicketEventsParams;
            "comment", false, "工单评论（入活动时间线）" => crate::TicketCommentParams;
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
            "list", false, "列出后台执行（内部调度面；可按 kind/status 过滤——codegraph index/sync 与 gc 自愈的 job 都在这。面向人的查询请用 logs 域）" => crate::jobs::JobsListParams;
            "get", false, "查后台执行详情（状态/错误/进度/attempts——job_id 从 codegraph index/sync 返回拿）" => crate::jobs::JobsGetParams;
            "events", false, "后台执行的过程行（增量轮询；与 logs 域同源——logs.query 带 job_id 是同一条时间线）" => crate::jobs::JobsEventsParams;
            "revive", true, "复活 dead/failed 的后台执行重跑（仅管理员——amk_ key 会收到明确拒绝）" => crate::jobs::JobsReviveParams
        ],
        "logs" => action_docs![
            "query", false, "查系统日志（系统里发生的一切：请求/错误/后台执行都在同一条时间线；可按 level/q/domain/job_id/request_id/时间窗过滤；domain=memory 看记忆蒸馏与审计）" => crate::logs::LogsQueryParams;
            "stats", false, "日志聚合（按 level 或 target 分组计数，一眼看系统态势）" => crate::logs::LogsStatsParams
        ],
        "study" => action_docs![
            "add", false, "开题（新学习领域）——name+goal（学到什么程度算完，归档锚）" => crate::study::StudyAddParams;
            "get", true, "topic 全量一次拿全【进度+下一步队列+资料清单】——跨会话恢复学习上下文的核心查询" => crate::study::StudyGetParams;
            "list", true, "全部学习领域简报" => crate::study::StudyListParams;
            "topic_update", true, "补丁更新 topic（name/goal/status active|paused|done——done=归档，数据保留）" => crate::study::StudyTopicUpdateParams;
            "topic_delete", false, "删 topic（级联全部节点，不可恢复）" => crate::study::StudyTopicDeleteParams;
            "item_add", false, "加知识点（position 缺省排尾部）" => crate::study::StudyItemAddParams;
            "unit_set", false, "知识点状态机 not_started|learning|learned（learned 记时间戳；允许回退）" => crate::study::StudyUnitSetParams;
            "item_link", false, "知识点挂资料（wiki_slugs/doc_ids 覆盖式更新）" => crate::study::StudyItemLinkParams;
            "item_set_review", false, "SRS 复习标记：needs_review 开关+review_due_at 到期时间（缺省立即到期）" => crate::study::StudyItemSetReviewParams;
            "reviews_due", true, "复习队列：已标记且到期的知识点（review_due_at 升序）" => crate::study::StudyReviewsDueParams;
            "journal_add", false, "进度时间线记一笔（学了什么/卡在哪/下一步）" => crate::study::StudyJournalAddParams;
            "journal_list", true, "查 topic 最近进度时间线（新→旧）" => crate::study::StudyJournalListParams
        ],
        _ => return None,
    })
}

/// 域工具名（= scope 名，projects 例外——scope 叫 project）。
pub const DOMAIN_TOOLS: &[&str] = &[
    "memory",
    "projects",
    "assets",
    "credentials",
    "circles",
    "wiki",
    "todos",
    "tickets",
    "codegraph",
    "jobs",
    "logs",
    "study",
];

pub fn is_domain_tool(name: &str) -> bool {
    DOMAIN_TOOLS.contains(&name)
}

/// action 级开关键（disabled_tools 里用 `域.action` 表示停用某个操作）。
pub fn action_key(domain: &str, action: &str) -> String {
    format!("{domain}.{action}")
}

/// 域参数解析：把 DomainCall 平铺参数还原成该操作的强类型结构体（历史参数结构体全复用）。
///
/// 两段式（ADR-20）：先严格解析——合法请求零额外开销；失败时按该操作 inputSchema 做
/// 受控宽容重试：声明为 integer/number/boolean 的字段收到可解析字符串时转原生类型
/// （劣质客户端把参数 stringify 的兜底，如 pi-mcp-adapter EN-224）。合法字符串值
/// 永不误转——它们首过即严格通过，不进重试路径。重试仍失败时报**首次**严格错误
/// （最贴近用户的原始问题）。
pub fn from_args<T: serde::de::DeserializeOwned + schemars::JsonSchema>(
    domain: &str,
    action: &str,
    args: serde_json::Map<String, Value>,
) -> Result<T, rmcp::ErrorData> {
    let value = Value::Object(args);
    match serde_json::from_value::<T>(value.clone()) {
        Ok(parsed) => Ok(parsed),
        Err(strict_err) => {
            let root = serde_json::to_value(rmcp::schemars::schema_for!(T)).unwrap_or(Value::Null);
            let defs = root.get("$defs").cloned().unwrap_or(Value::Null);
            // EN-11 指路提示在 move 前算好：coerce 只转值不动键集，
            // strict 与 coerced 的 missing field 判定等价
            let hint = param_name_hint(&value, &root, &strict_err);
            let mut coerced = value;
            coerce_by_schema(&root, &defs, &mut coerced);
            serde_json::from_value::<T>(coerced).map_err(|coerced_err| {
                mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    format!(
                        "{domain}.{action} 参数错误：{coerced_err}（宽容解析后仍失败；首次严格错误：{strict_err}）{hint}。用 action=\"help\" 查看该操作的参数说明。"
                    ),
                )
            })
        }
    }
}

/// EN-11 报错指路：missing field 时点名调用方传入的未识别参数。
/// 跨域主键名不统一（jobs/tickets 用 id、assets 用 asset_id、codegraph register 用
/// source_uri、location_add 用 asset 名字符串），调用方按相邻域直觉传名必踩一次
/// missing field 打回——报错直接指出「你传的这些键本操作不认识」，打回即学会。
/// 不做自动改名（D7 同哲学：宽容解析可以，静默改语义不行，掩盖真实错误）。
fn param_name_hint(args: &Value, root_schema: &Value, err: &serde_json::Error) -> String {
    if !err.to_string().contains("missing field") {
        return String::new();
    }
    let (Some(map), Some(props)) = (
        args.as_object(),
        root_schema.get("properties").and_then(Value::as_object),
    ) else {
        return String::new();
    };
    let unknown: Vec<&str> = map
        .keys()
        .filter(|k| !props.contains_key(*k))
        .map(String::as_str)
        .collect();
    if unknown.is_empty() {
        return String::new();
    }
    let total = unknown.len();
    let shown: Vec<&str> = unknown.into_iter().take(8).collect();
    // 必须保持 {:?}（escape_debug 转义换行/控制字符）——改 {} 会丢转义，日志注入面重开
    format!(
        "；你传入的这些参数本操作不认识（共 {total} 个）：{shown:?}——疑似参数名不匹配（对照 action=\"help\" 的参数手册改名重试）"
    )
}

/// 按 inputSchema（JSON Schema Value 形态）递归做受控 string→原生 转换：
/// `type` 断言为 integer/number/boolean 的位置，收到可解析字符串才转；
/// 其余（含 string 字段）一律不动。
fn coerce_by_schema(schema: &Value, defs: &Value, v: &mut Value) {
    // $ref 解析（schemars 对嵌套类型生成 $defs + $ref）
    if let Some(name) = schema.get("$ref").and_then(|r| r.as_str()) {
        let name = name.rsplit('/').next().unwrap_or("");
        if let Some(def) = defs.get(name) {
            coerce_by_schema(def, defs, v);
        }
        return;
    }
    // 1) type 断言：string → boolean / integer / number
    let types: Vec<&str> = match schema.get("type") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    if let Value::String(s) = v {
        let is_bool = types.contains(&"boolean") && matches!(s.as_str(), "true" | "false");
        let is_num = types.contains(&"integer") || types.contains(&"number");
        if is_bool {
            *v = Value::Bool(s == "true");
        } else if is_num {
            if let Ok(n) = s.parse::<i64>() {
                *v = Value::Number(n.into());
            } else if let Ok(f) = s.parse::<f64>()
                && let Some(n) = serde_json::Number::from_f64(f)
            {
                *v = Value::Number(n);
            }
        }
    }
    // 2) object properties 递归
    if let Some(props) = schema.get("properties").and_then(Value::as_object)
        && let Some(map) = v.as_object_mut()
    {
        for (key, sub) in props {
            if let Some(x) = map.get_mut(key) {
                coerce_by_schema(sub, defs, x);
            }
        }
    }
    // 3) array items 递归
    if let Some(sub) = schema.get("items")
        && let Some(items) = v.as_array_mut()
    {
        for x in items {
            coerce_by_schema(sub, defs, x);
        }
    }
    // 4) 复合形态递归（Option<T> 生成 anyOf [null, T]；oneOf/allOf 一并覆盖）
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(list) = schema.get(key).and_then(Value::as_array) {
            for sub in list {
                coerce_by_schema(sub, defs, v);
            }
        }
    }
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
                | "revise"
                | "review"
                | "correct"
                | "confirm"
                | "discard"
                | "persona_edit"
                | "distill"
                | "write_session"
                | "append_session"
                | "forget"
                | "atom_archive"
                | "kv_delete"
                | "entity_merge"
        ) | (
            "projects",
            "create"
                | "update"
                | "delete"
                | "batch_delete"
                | "location_add"
                | "location_update"
                | "location_delete"
                | "link"
                | "unlink"
                | "doc_add"
                | "doc_patch"
                | "doc_update"
                | "doc_delete"
                | "file_put"
                | "file_delete"
        ) | ("assets", "add" | "update" | "delete")
            | ("assets", "runbook_save" | "runbook_restore")
            | ("credentials", "put" | "delete")
            | ("circles", "create" | "update" | "relate" | "unrelate")
            | (
                "circles",
                "forget" | "delete" | "merge" | "attach" | "detach"
            )
            | (
                "study",
                "add"
                    | "topic_update"
                    | "topic_delete"
                    | "item_add"
                    | "unit_set"
                    | "item_link"
                    | "item_set_review"
                    | "journal_add"
            )
            | (
                "wiki",
                "write_page"
                    | "archive_query"
                    | "restore_version"
                    | "delete_source"
                    | "lint_deep"
                    | "ingest"
                    | "document_add"
                    | "document_delete"
                    | "archive"
                    | "promote"
                    | "delete_page"
                    | "merge"
                    | "purpose_set"
                    | "repair"
                    | "repair_async"
                    | "insight_dismiss"
                    | "insight_reset"
                    | "rebuild_links"
                    | "rebuild_tsv"
                    | "reembed"
            )
            | (
                "todos",
                "add" | "link" | "unlink" | "done" | "update" | "delete"
            )
            | (
                "tickets",
                "add" | "link" | "unlink" | "update" | "delete" | "comment"
            )
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
                | "recall"
                | "browse"
                | "kv_get"
                | "kv_list"
                | "kv_search"
                | "list_sessions"
                | "get_session"
                | "list_atoms"
                | "entities"
                | "scenarios_list"
                | "persona_get"
                | "atom_duplicates"
                | "entity_duplicates"
        ) | (
            "projects",
            "types"
                | "list"
                | "get"
                | "doc_get"
                | "doc_search"
                | "file_get"
                | "file_list"
                | "links"
                | "location_get"
        ) | ("assets", "kinds" | "list" | "get")
            | ("assets", "runbook" | "runbook_versions")
            | ("credentials", "list" | "get" | "reads")
            | ("circles", "graph" | "entity" | "relations" | "duplicates")
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
                    | "index"
                    | "purpose"
                    | "insights"
                    | "promotions"
                    | "duplicates"
                    | "query_gaps"
                    | "folders"
            )
            | ("todos", "list" | "links" | "get")
            | ("tickets", "list" | "links" | "get" | "events")
            | ("codegraph", "list" | "query")
            | ("jobs", "list" | "get" | "events")
            | ("logs", "query" | "stats")
            | ("study", "get" | "list" | "reviews_due" | "journal_list")
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
    let mut root = json!({
        "domain": domain,
        "how_to_call": format!("{{\"action\":\"<操作名>\", ...该操作的参数（平铺）}}；本返回即 {domain} 域全部可用操作"),
        // 验收反馈：search_all 是独立工具，不在任何域 help 里——每份手册顶部指路，
        // 想跨域扫一遍时不用翻 tools/list
        "cross_domain_hint": "另有独立工具 search_all（非本域操作）：一次查询并发 memory/wiki/todos/projects 各回 top-k 摘要——不确定信息在哪域时用",
        "actions": actions,
    });
    // EN-236：wiki 域动作最多（28 个），help 平铺可发现性差——按九大用途分组的导航层（纯增量：
    // actions 平铺原样保留，旧客户端不受影响；每组列 action 名单，数量不减语义不变）
    if domain == "wiki" {
        root["groups"] = wiki_groups_hint();
    }
    root
}

/// EN-236：wiki 用途组导航（找/读/写/原料/体检/版本/整理/晋升）——全部动作入组不重不漏（人审组随人审移除退役）
///（含工单后增动作：list_pages 归读、document_delete 归原料——验收③数量不减，按组可导航到每个动作）。
fn wiki_groups_hint() -> Value {
    let groups: &[(&str, &str, &[&str])] = &[
        (
            "找",
            "检索：页面级全文/向量 + 原文 chunk 级",
            &["search", "documents_search"],
        ),
        (
            "读",
            "浏览：页面/内容目录/库意图",
            &["get_page", "index", "purpose", "list_pages"],
        ),
        (
            "写",
            "页面与问答沉淀：写页/归档分析产物/问答存档",
            &["write_page", "archive", "archive_query"],
        ),
        (
            "原料",
            "原料通道：喂给维护 Agent / 原文入库（ingest 走 Agent Harness，document_add 入原文 RAG）",
            &["ingest", "document_add", "document_get", "document_delete"],
        ),
        (
            "体检",
            "质量：lint 快检/LLM 深检/链接图/洞察",
            &["lint", "lint_deep", "graph", "insights"],
        ),
        (
            "版本",
            "版本史/快照预览/回滚",
            &["versions", "version_content", "restore_version"],
        ),
        (
            "整理",
            "合并/删页/原料清单/删原料",
            &["merge", "delete_page", "sources", "delete_source"],
        ),
        ("晋升", "跨项目知识晋升与登记", &["promote", "promotions"]),
    ];
    let items: Vec<Value> = groups
        .iter()
        .map(|(name, why, actions)| json!({ "group": name, "why": why, "actions": actions }))
        .collect();
    json!({
        "hint": "动作多，按用途找组——先看组名定位意图，再看组内 action；全部 action 在下方 actions 平铺列表（参数以该处为准）。26 动作已全部入组",
        "groups": items,
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

    /// 全表自检：action 非空、schema 可序列化、无重复 action 名。
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

#[cfg(test)]
mod ro_unify_tests {
    use super::*;
    use engram_core::auth::Principal;
    use uuid::Uuid;

    /// RJ-20/A2 验收：同一把 `memory:ro` key 在 MCP 侧读动作放行、写动作拒绝
    /// （与 HTTP 侧 require_scope_read / require_scope 语义一致）。
    #[test]
    fn ro_key_reads_ok_writes_rejected_mcp() {
        let ro = Principal::ApiKey {
            key_id: Uuid::nil(),
            name: "t".into(),
            scopes: vec!["memory:ro".into()],
        };
        let actions: Vec<String> = action_docs("memory")
            .expect("memory 域应有 action 文档表")
            .iter()
            .map(|d| d.action.to_string())
            .collect();
        let write = actions
            .iter()
            .find(|a| is_write_action("memory", a))
            .expect("memory 应有写动作");
        let read = actions
            .iter()
            .find(|a| !is_write_action("memory", a))
            .expect("memory 应有读动作");
        assert!(
            check_action_access(&ro, "memory", read).is_ok(),
            ":ro 读动作应放行：{read}"
        );
        assert!(
            check_action_access(&ro, "memory", write).is_err(),
            ":ro 写动作应拒绝：{write}"
        );
    }
}

#[cfg(test)]
mod lenient_args_tests {
    use super::*;
    use schemars::JsonSchema;
    use serde_json::json;

    #[derive(Debug, Deserialize, JsonSchema)]
    struct PatchLike {
        doc_id: String,
        start_line: i64,
        end_line: i64,
        with_ln: Option<bool>,
    }

    #[derive(Deserialize, JsonSchema)]
    struct HasStringNum {
        name: String,
        ids: Vec<i64>,
    }

    fn args(v: Value) -> serde_json::Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    /// EN-224 现场：客户端把 int/bool stringify——宽容层应转换成功
    #[test]
    fn stringified_int_and_bool_are_coerced() {
        let got: PatchLike = from_args(
            "t",
            "x",
            args(
                json!({"doc_id": "d1", "start_line": "188", "end_line": "188", "with_ln": "true"}),
            ),
        )
        .expect("stringify 参数应被宽容解析");
        assert_eq!((got.start_line, got.end_line), (188, 188));
        assert_eq!(got.with_ln, Some(true));
        assert_eq!(got.doc_id, "d1");
    }

    /// 防误伤：合法 string 字段值（"188"）必须原样保留
    #[test]
    fn legit_string_field_not_tampered() {
        let got: HasStringNum =
            from_args("t", "x", args(json!({"name": "188", "ids": ["1", "2"]}))).unwrap();
        assert_eq!(got.name, "188", "合法 string 字段不得被转换");
        assert_eq!(got.ids, vec![1, 2], "数组元素内的 string int 也应转换");
    }

    /// 垃圾值仍报错，且报错保留首次严格信息（可行动）
    #[test]
    fn garbage_still_reports_strict_error() {
        let err = from_args::<PatchLike>(
            "t",
            "x",
            args(json!({"doc_id": "d1", "start_line": "abc", "end_line": 1})),
        )
        .expect_err("不可解析字符串仍应报错");
        assert!(
            err.message.contains("expected i64"),
            "报错应保留首次严格信息：{}",
            err.message
        );
    }
}

#[cfg(test)]
mod probe_doc_get {
    use super::*;
    use crate::project_docs::ProjectDocGetParams;
    use serde_json::json;

    #[test]
    fn real_doc_get_params_stringified() {
        let got: ProjectDocGetParams = from_args(
            "projects",
            "doc_get",
            json!({"doc_id": "x", "start_line": "1", "end_line": "2", "with_line_numbers": "true"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .expect("真实 doc_get 结构体的 stringify 参数应被宽容解析");
        assert_eq!(got.start_line, Some(1));
        assert_eq!(got.with_line_numbers, Some(true));
        // 宽容解析的真实结构体契约测试（探针 println 已随 P003-T003 清理）
    }
}

#[cfg(test)]
mod doc_add_alias_tests {
    use super::*;
    use serde_json::json;

    /// EN-249：document_add 的标题字段兼容 `title` 别名（AI 语义上传 title 不再被静默忽略）
    #[test]
    fn document_add_title_alias_reaches_name() {
        let got: crate::wiki::WikiDocumentAddParams = from_args(
            "wiki",
            "document_add",
            json!({"text": "正文", "title": "我的标题"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .expect("title 别名应被接受");
        assert_eq!(got.name.as_deref(), Some("我的标题"), "title 应落到 name");
    }
}

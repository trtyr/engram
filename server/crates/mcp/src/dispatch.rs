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
            "remember", false, "一句话记忆（记条小事实不必手搓 turns；等价单轮 write_session+auto 蒸馏）" => crate::RememberParams;
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
            "doc_search", false, "grep 式跨文档按行检索（定位到哪篇哪行）" => crate::ProjectDocSearchParams;
            "doc_patch", false, "行级补丁（replace/insert/delete 一个行区间——改长文档不必取全文重发）" => crate::ProjectDocPatchParams;
            "doc_update", false, "编辑项目文档（补丁式）" => crate::ProjectDocUpdateParams;
            "doc_delete", true, "删除项目文档（不可逆）" => crate::ProjectDocDeleteParams
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
            "index", false, "内容目录（按页型分组的全库目录：slug/标题/入链数/首段摘要；只读动态聚合）" => crate::wiki::WikiLibParams;
            "archive", false, "问答/分析产物归档为 analysis 页（related 自动建双向 wikilinks——好答案不该消失在聊天记录里）" => crate::wiki::WikiArchiveParams;
            "libraries", false, "列出全部 wiki 库（多库；页面/原料计数一并返回；建库/删库走 Web）" => crate::wiki::WikiLibrariesParams;
            "delete_page", true, "删除页面（连带清理双向 wikilink；最后状态留快照可重建）" => crate::wiki::WikiDeletePageParams
        ],
        "todos" => action_docs![
            "add", false, "快速记一条待办（灵感/计划/操作/排查；不绑定项目）" => crate::TodoAddParams;
            "list", false, "待办列表（open 优先；status/priority/tag/q 过滤）" => crate::TodoListParams;
            "get", false, "待办详情" => crate::TodoIdParams;
            "done", false, "标记完成（记 done_at）" => crate::TodoIdParams;
            "update", false, "编辑待办（标题/详情/优先级/状态/截止；补丁式）" => crate::TodoUpdateParams;
            "delete", true, "删除待办（不可逆）" => crate::TodoIdParams
        ],
        "codegraph" => action_docs![
            "list", false, "列出已注册代码库（注册状态/索引规模）" => crate::CgNoParams;
            "register", false, "注册代码库（本地绝对路径按服务端文件系统校验，或 git URL）" => crate::CgRegisterParams;
            "query", false, "代码图谱查询（search/explore大纲/node/callers/callees/impact）" => crate::CgQueryParams;
            "index", false, "建索引/重建索引（异步 job）" => crate::CgNameParams;
            "sync", false, "增量同步索引（小改动后刷新）" => crate::CgNameParams;
            "delete", true, "注销代码图谱项目（删注册与索引；源码不动）" => crate::CgNameParams
        ],
        _ => return None,
    })
}

/// 域工具名（= scope 名，projects 例外——scope 叫 project）。
pub const DOMAIN_TOOLS: &[&str] = &["memory", "projects", "skills", "wiki", "todos", "codegraph"];

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
        assert_eq!(actions.len(), 5, "停用操作应从手册隐身");
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
}

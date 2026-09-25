//! memory 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ContextParams {
    /// 可选相关性查询（缺省按最近 + 30 天半衰期重排）；会话开始时通常不传
    #[schemars(
        description = "可选：相关性查询词。会话冷启动时不传（按最近+新鲜度），带着具体问题回忆时传。"
    )]
    pub query: Option<String>,
    /// 各层条数预算（默认 20）
    #[schemars(description = "各层返回条数预算，默认 20。通常不需要调。")]
    pub budget_items: Option<usize>,
    /// 总字符预算（默认 8000）
    #[schemars(description = "总字符数预算，默认 8000。通常不需要调。")]
    pub budget_chars: Option<usize>,
    /// 证据溯源（默认关）
    #[schemars(
        description = "可选：true = 携带证据溯源字段（persona.evidence_refs / atom.source_refs / scenario.atom_refs 的 ID 数组）。默认 false——溯源 ID 客户端几乎不消费，省上下文（R 报告 P1-5）；审计需要时再开。"
    )]
    pub include_evidence: Option<bool>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct MemoryDistillResultParams {
    /// 会话 id（write_session 返回的 id）
    #[schemars(description = "会话 id（write_session 返回的 id）。")]
    pub session_id: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SearchParams {
    /// 检索词
    #[schemars(description = "检索词。中英文均可，混合检索（全文+向量）。")]
    pub query: String,
    /// 层过滤：["l1","l2","l3","entities"]，空 = 全部
    #[schemars(
        description = "可选：限定检索层。l1=原子事实, l2=场景, l3=画像, entities=人物/项目/主题/群组/地点。空 = 全部层。"
    )]
    pub layers: Option<Vec<String>>,
    /// 每层最大命中数（默认 20）
    #[schemars(description = "每层最大命中数，默认 20。")]
    pub max_items: Option<i64>,
    /// 时间范围起点（ISO8601；occurred_at 优先，NULL 回退 created_at）
    #[schemars(
        description = "可选：时间范围起点，ISO8601（如 2026-09-01 或 2026-09-01T00:00:00Z）。"
    )]
    pub from: Option<String>,
    /// 时间范围终点
    #[schemars(description = "可选：时间范围终点，ISO8601。")]
    pub to: Option<String>,
    /// 证据溯源（默认关）
    #[schemars(
        description = "可选：true = L3 画像命中携带 evidence_refs（溯源 ID 数组）。默认 false——检索消费者不用溯源 ID，与 context 同口径（验收遗留 #1）；审计需要时再开。"
    )]
    pub include_evidence: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ListAtomsParams {
    /// 原子类型：preference/fact/decision/event/insight/correction/failure/convention
    #[schemars(
        description = "可选：按类型过滤。preference=偏好, fact=事实, decision=决策, event=事件, insight=洞察, correction=纠正, failure=失败, convention=惯例。"
    )]
    pub kind: Option<String>,
    /// active（默认）/ superseded / archived / candidate / all
    #[schemars(
        description = "可选：按状态过滤，默认 \"active\"（只看有效记忆）。superseded=被取代, archived=归档, candidate=候选；\"all\" = 全部状态（巡检历史时用）。"
    )]
    pub status: Option<String>,
    /// true = 只看待审（低置信度）；false = 只看已审
    #[schemars(
        description = "可选：按待审标记过滤（needs_review=true 是低置信度、建议用户复核的条目）。"
    )]
    pub needs_review: Option<bool>,
    /// keyset 分页游标（上一页最后一条的 created_at，ISO8601）
    #[schemars(description = "可选：分页游标。传上一页最后一条的 created_at（ISO8601）取下一页。")]
    pub cursor: Option<String>,
    /// 每页条数（默认 100）
    #[schemars(description = "每页条数，默认 100。")]
    pub limit: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct EntitiesParams {
    /// 检索词（人名/项目名/主题词）
    #[schemars(description = "检索词：人名、项目名、主题词等。名字命中权重最高。")]
    pub q: String,
    /// 返回条数（默认 20）
    #[schemars(description = "返回条数，默认 20。")]
    pub limit: Option<i64>,
}

#[tool_router(router = memory_router)]
impl EngramMcpServer {
    /// 装载用户记忆上下文包（L3 画像 + L2 场景 + L1 原子事实 + 实体，按预算裁剪）。
    ///
    /// 何时用：会话开始时调用一次，冷启动装载「这个用户是谁、在忙什么、有什么偏好与约束」。
    /// 何时不用：需要回忆某个具体细节时用 memory_search（更省 token）；本工具是全景而非定向检索。
    /// 返回：persona（画像分面）、scenarios（场景）、atoms（原子事实）、entities（实体）、
    /// pending_review（待用户复核的低置信度条目，可顺带提醒用户）。
    pub(crate) async fn memory_context(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ContextParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let pack = self
            .svc()
            .context_pack(
                params.0.query.as_deref(),
                params.0.budget_items.unwrap_or(20),
                params.0.budget_chars.unwrap_or(8000),
                false,
            )
            .await
            .map_err(from_memory)?;
        let mut v = serde_json::to_value(&pack).unwrap_or(serde_json::json!({}));
        // P1-5：证据溯源 ID 默认不携带（审计时显式 include_evidence=true）
        if !params.0.include_evidence.unwrap_or(false) {
            strip_keys(&mut v, &["evidence_refs", "source_refs", "atom_refs"]);
        }
        ok_json(v)
    }

    /// 定向检索用户记忆（全文 + 向量混合，跨 L1/L2/L3/实体四层）。
    ///
    /// 何时用：对话中需要回忆与当前话题相关的用户背景、既往决策、偏好、历史事件时。
    /// 何时不用：会话开场的全景装载用 memory_context；浏览全量列表用 memory_list_atoms。
    /// 命中会回写热度（hit_count），常被检索的内容会在整理中获得更高权重。
    /// sensitive 条目默认可见（2026-09-12 口径放开——标记保留不隐身）；返回 {entities, l1, l2, l3}，各元素含 score/title/snippet；
    /// L3 画像默认不带 evidence_refs（与 context 同口径，include_evidence=true 开）。
    pub(crate) async fn memory_search(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SearchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let sp = params.0;
        let from = sp.from.as_deref().map(parse_flex_datetime).transpose()?;
        let to = sp.to.as_deref().map(parse_flex_datetime).transpose()?;
        let layers: Vec<&str> = sp.layers.iter().flatten().map(|s| s.as_str()).collect();
        let resp = self
            .svc()
            .search(
                &sp.query,
                &layers,
                sp.max_items.unwrap_or(20),
                false,
                from,
                to,
            )
            .await
            .map_err(from_memory)?;
        let mut v = serde_json::to_value(&resp).unwrap_or(serde_json::json!({}));
        // 与 context 同口径（验收遗留 #1）：L3 画像命中的 evidence_refs 默认不携带
        if !sp.include_evidence.unwrap_or(false) {
            strip_keys(&mut v, &["evidence_refs", "source_refs", "atom_refs"]);
        }
        // EN-233②：空命中可观测——区分「没存过」vs「存了但蒸馏未完成/已归档」
        let layer_empty = |k: &str| {
            v.get(k)
                .and_then(|x| x.as_array())
                .is_none_or(|a| a.is_empty())
        };
        if ["sessions", "atoms", "scenarios", "persona", "entities"]
            .iter()
            .all(|k| layer_empty(k))
        {
            v["hint"] = json!(
                "检索无命中——两种可能：①确实没存过（write_session/remember 写入）；②存了但蒸馏尚未完成（写入后几分钟）或内容已归档。排查：action=\"list_sessions\" 确认会话在不在、action=\"list_atoms\"（status=\"all\"）翻库存、action=\"distill_result\"（session_id）看蒸馏产物"
            );
        }
        ok_json(v)
    }

    /// 浏览 L1 原子事实列表（keyset 分页，可按类型/状态/待审过滤）。
    ///
    /// 何时用：需要系统性浏览用户的事实条目（而非定向检索）时；或巡检 needs_review 条目。
    /// 何时不用：有明确主题的回忆用 memory_search；开场装载用 memory_context。
    pub(crate) async fn memory_list_atoms(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ListAtomsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let lp = params.0;
        let cursor = lp.cursor.as_deref().map(parse_flex_datetime).transpose()?;
        // P1-6：默认 active——文档一直写着「默认查这个」，行为此前却是全量。
        // "all" 是显式出口（巡检历史）。
        let status = match lp.status.as_deref() {
            None | Some("") | Some("active") => Some("active"),
            Some("all") => None,
            Some(other) => {
                return Err(mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    format!(
                        "status 只支持 active/superseded/archived/candidate/all（收到 {other:?}）"
                    ),
                ));
            }
        };
        let atoms = self
            .svc()
            .list_atoms(
                lp.kind.as_deref(),
                status,
                lp.needs_review,
                cursor,
                lp.limit.unwrap_or(100),
            )
            .await
            .map_err(from_memory)?;
        ok_json(serde_json::to_value(&atoms).unwrap_or(serde_json::json!([])))
    }

    // ---------- 渐进式发现：域入口工具（每域一个，域内操作按需发现） ----------
    //
    // 历史上的 53 个扁平工具全部收编为「域 + action」，action 表（摘要/破坏性/参数
    // schema）是 dispatch::action_docs 的静态表——L0 目录、help 手册、管理台三方同源。
    // scope 检查在各域实现方法内原样保留；这里只做 help 渲染与 action 分发。

    /// 用户记忆域（单一入口）。记忆四层：L0 会话 →（蒸馏）→ L1 原子 → L2 场景 → L3 画像，
    /// 实体坐标系横向串联。开场用 action="context" 装载，定向回忆用 "search"，
    /// 收尾用 "write_session" 写入；遗忘用 "forget"。
    /// 速记：remember 正文字段名是 text；strength=fact 直写原话限 500 字，默认蒸馏路径成段内容也可，更长走 write_session。操作全景：action="help"。
    #[tool(
        name = "memory",
        annotations(
            title = "用户记忆域",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(crate) async fn memory_tool(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(call): Parameters<dispatch::DomainCall>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        // 工具级 gate 按 action 分流（收录哲学线：原件独立）——
        // kv_* 要求 original scope（凭据/精确值读写权，memory 不再顺手携带）；
        // 其余动作要求 memory scope。各 handler 内部还有二次校验（防御性）。
        const KV_ACTIONS: [&str; 4] = ["kv_put", "kv_get", "kv_list", "kv_search"];
        if KV_ACTIONS.contains(&call.action.as_str()) {
            require_original(&p)?;
        } else {
            require_memory(&p)?;
        }
        if call.action == "help" {
            let cfg = load_config(&self.state.pool).await;
            return ok_json(dispatch::render_manual("memory", &cfg.disabled_tools));
        }
        match call.action.as_str() {
            "context" => {
                self.memory_context(
                    ctx,
                    Parameters(dispatch::from_args("memory", "context", call.args)?),
                )
                .await
            }
            "search" => {
                self.memory_search(
                    ctx,
                    Parameters(dispatch::from_args("memory", "search", call.args)?),
                )
                .await
            }
            "remember" => {
                self.memory_remember(
                    ctx,
                    Parameters(dispatch::from_args("memory", "remember", call.args)?),
                )
                .await
            }
            "correct" => {
                self.memory_correct(
                    ctx,
                    Parameters(dispatch::from_args("memory", "correct", call.args)?),
                )
                .await
            }
            "confirm" => {
                self.memory_confirm(
                    ctx,
                    Parameters(dispatch::from_args("memory", "confirm", call.args)?),
                )
                .await
            }
            "discard" => {
                self.memory_discard(
                    ctx,
                    Parameters(dispatch::from_args("memory", "discard", call.args)?),
                )
                .await
            }
            "persona_edit" => {
                self.memory_persona_edit(
                    ctx,
                    Parameters(dispatch::from_args("memory", "persona_edit", call.args)?),
                )
                .await
            }
            "distill" => {
                self.memory_distill(
                    ctx,
                    Parameters(dispatch::from_args("memory", "distill", call.args)?),
                )
                .await
            }
            "write_session" => {
                self.memory_write_session(
                    ctx,
                    Parameters(dispatch::from_args("memory", "write_session", call.args)?),
                )
                .await
            }
            "append_session" => {
                self.memory_append_session(
                    ctx,
                    Parameters(dispatch::from_args("memory", "append_session", call.args)?),
                )
                .await
            }
            "list_sessions" => {
                self.memory_list_sessions(
                    ctx,
                    Parameters(dispatch::from_args("memory", "list_sessions", call.args)?),
                )
                .await
            }
            "get_session" => {
                self.memory_get_session(
                    ctx,
                    Parameters(dispatch::from_args("memory", "get_session", call.args)?),
                )
                .await
            }
            "distill_result" => {
                self.memory_distill_result(
                    ctx,
                    Parameters(dispatch::from_args("memory", "distill_result", call.args)?),
                )
                .await
            }
            "kv_put" => {
                self.memory_kv_put(
                    ctx,
                    Parameters(dispatch::from_args("memory", "kv_put", call.args)?),
                )
                .await
            }
            "kv_get" => {
                self.memory_kv_get(
                    ctx,
                    Parameters(dispatch::from_args("memory", "kv_get", call.args)?),
                )
                .await
            }
            "kv_list" => {
                self.memory_kv_list(
                    ctx,
                    Parameters(dispatch::from_args("memory", "kv_list", call.args)?),
                )
                .await
            }
            "kv_search" => {
                self.memory_kv_search(
                    ctx,
                    Parameters(dispatch::from_args("memory", "kv_search", call.args)?),
                )
                .await
            }
            "list_atoms" => {
                self.memory_list_atoms(
                    ctx,
                    Parameters(dispatch::from_args("memory", "list_atoms", call.args)?),
                )
                .await
            }
            "scenarios_list" => {
                self.memory_scenarios_list(
                    ctx,
                    Parameters(dispatch::from_args("memory", "scenarios_list", call.args)?),
                )
                .await
            }
            "persona_get" => {
                self.memory_persona_get(
                    ctx,
                    Parameters(dispatch::from_args("memory", "persona_get", call.args)?),
                )
                .await
            }
            "atom_duplicates" => {
                self.memory_atom_duplicates(
                    ctx,
                    Parameters(dispatch::from_args("memory", "atom_duplicates", call.args)?),
                )
                .await
            }
            "entity_duplicates" => {
                self.memory_entity_duplicates(
                    ctx,
                    Parameters(dispatch::from_args(
                        "memory",
                        "entity_duplicates",
                        call.args,
                    )?),
                )
                .await
            }
            "entities" => {
                self.memory_entities(
                    ctx,
                    Parameters(dispatch::from_args("memory", "entities", call.args)?),
                )
                .await
            }
            "forget" => {
                self.memory_forget(
                    ctx,
                    Parameters(dispatch::from_args("memory", "forget", call.args)?),
                )
                .await
            }
            "atom_archive" => {
                self.memory_atom_archive(
                    ctx,
                    Parameters(dispatch::from_args("memory", "atom_archive", call.args)?),
                )
                .await
            }
            "kv_delete" => {
                self.memory_kv_delete(
                    ctx,
                    Parameters(dispatch::from_args("memory", "kv_delete", call.args)?),
                )
                .await
            }
            other => Err(dispatch::unknown_action("memory", other)),
        }
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_memory() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::memory_router()
}

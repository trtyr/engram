//! memory_write 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

/// 一句话记忆（R 报告 P1-9）：记条小事实不必手搓 turns 数组。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct RememberParams {
    /// 要记住的一句话（字段名 text，≤500 字）
    #[schemars(
        description = "要记住的事实/偏好/事件，一句话 ≤500 字（如「用户的猫叫墨鱼，喜欢趴键盘上睡觉」）。字段名是 text。等价于单轮 write_session + auto 蒸馏；成段内容请走 write_session。"
    )]
    pub text: Option<String>,
    /// 会话级敏感标记
    #[schemars(
        description = "可选：敏感内容（医疗/感情/财务）置 true——敏感是标记不是隐身，产物默认可见并带 sensitive 标记。"
    )]
    pub sensitive: Option<bool>,
    /// agent 归因
    #[schemars(description = "可选：agent 归因名。缺省用连接本服务的 API key 名。")]
    pub agent: Option<String>,
    /// 显式断言强度：仅支持 "fact"——用户亲口明示的事实用这个（直写落库不走蒸馏，保原话）；
    /// 缺省走蒸馏（产物默认 inference，保守不升格）
    #[schemars(
        description = "可选：仅支持 fact——用户亲口明示的事实直写落库（不走蒸馏，原话保真）。缺省走蒸馏（默认 inference）。"
    )]
    pub strength: Option<String>,
}

/// 更正记忆（快路径取代链）：AI 记忆管家——用户说「你记错了」时使用。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CorrectParams {
    /// 目标原子 id（先 search 定位；仅 active 且非敏感原子可被取代）
    #[schemars(
        description = "要更正的旧原子 id——先 search 定位。仅 active 且非敏感原子可被取代；更正后旧原子标记 superseded 并指向本条新事实（取代链留痕）。"
    )]
    pub target_id: String,
    /// 更正后的新事实（一句话，≤500 字）
    #[schemars(
        description = "更正后的新事实，一句话 ≤500 字（如「用户现居杭州」）。新原子 active，旧原子自动 superseded。"
    )]
    pub text: String,
}

/// 待审复核处置（AI 代管复核）：confirm 摘标记 / discard 归档。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ReviewActionParams {
    /// 待审原子 id（仅 needs_review=true 的条目可处置）
    #[schemars(
        description = "要处置的待审原子 id。仅 needs_review=true 的条目可处置——正常记忆对 AI 只读。"
    )]
    pub atom_id: String,
}

/// 编辑画像分面（AI 记忆管家）：version+1 落钉，蒸馏不再覆盖该分面。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct PersonaEditParams {
    /// 分面名（七值之一）
    #[schemars(
        description = "画像分面：identity | preferences | skills | constraints | communication_style | goals | routines。"
    )]
    pub aspect: String,
    /// 分面的完整新内容（1~4000 字，自包含描述——落库后蒸馏不再覆盖）
    #[schemars(
        description = "该分面的完整新版本内容（1~4000 字，中文自包含描述）。落库即钉住（manually_edited）——蒸馏产出对该分面不再生效。"
    )]
    pub content: String,
}

/// 手动触发蒸馏链（AI 记忆管家）：撞车守卫——正在蒸馏时只提示不重复投递。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct DistillParams {
    /// true 时附带 consolidate 全量整理
    #[schemars(
        description = "可选：true 时在蒸馏链外附带 consolidate 全量整理（近重复合并）。缺省 false。"
    )]
    pub full: Option<bool>,
    /// 动作模式：distill（默认）| rebuild（画像全量重建）| sleep（预留——内置节律上线后开放）
    #[schemars(
        description = "可选动作模式：\"distill\"（默认，触发蒸馏链）/ \"rebuild\"（画像全量重建——以全部场景重算所有非钉住分面，记忆清理/修订后用）/ \"sleep\"（记忆巩固——内置节律上线后开放，当前返回未上线提示）。"
    )]
    pub mode: Option<String>,
}

#[tool_router(router = memory_write_router)]
impl EngramMcpServer {
    /// 蒸馏回执（distill_result）：查一次会话蒸馏产出了哪些原子（id/内容/强度/状态）。
    pub(crate) async fn memory_distill_result(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<MemoryDistillResultParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let sid = uuid::Uuid::parse_str(params.0.session_id.trim()).map_err(|_| {
            rmcp::ErrorData::invalid_params(
                format!("session_id 不是合法 UUID: {}", params.0.session_id),
                None,
            )
        })?;
        let v = self.svc().distill_result(sid).await.map_err(from_memory)?;
        ok_json(v)
    }

    /// 一句话记忆（R 报告 P1-9）：记条小事实不必手搓 turns 数组。
    ///
    /// 何时用：用户说了值得长期记住的一句话事实/偏好/事件时（「记住我的猫叫墨鱼」）。
    /// 何时不用：成段对话收尾用 write_session（上下文更完整，蒸馏质量更高）。
    pub(crate) async fn memory_remember(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<RememberParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let principal = principal_of(&ctx)?;
        require_memory(&principal)?;
        let rp = params.0;
        // R（参数摩擦收口）：缺字段/为空/超长统一给一条完整约束提示——
        // 此前 serde missing field 报错不带字段名与上限，AI 每次冷启动要试错多轮
        let Some(raw) = rp.text else {
            return Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "remember 需要正文字段 text（上限 {} 字）——注意字段名是 text 不是 content；strength=fact 直写原话时内容需 ≤{} 字；成段内容走默认蒸馏路径或 write_session",
                    engram_core::memory::TURN_TEXT_MAX_CHARS,
                    engram_core::memory::ATOM_MAX_CHARS
                ),
            ));
        };
        let text = raw.trim().to_string();
        if text.is_empty() {
            return Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "text 不能为空——要记住的内容一句话写清楚（上限 {} 字）；成段内容请走 write_session",
                    engram_core::memory::TURN_TEXT_MAX_CHARS
                ),
            ));
        }
        if text.chars().count() > engram_core::memory::TURN_TEXT_MAX_CHARS {
            return Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "text 超长（当前 {} 字，上限 {} 字）——remember text 自身可到上限；但 strength=fact 直写原话限 {} 字，超长请去掉 strength=fact 走默认蒸馏或用 write_session",
                    text.chars().count(),
                    engram_core::memory::TURN_TEXT_MAX_CHARS,
                    engram_core::memory::ATOM_MAX_CHARS
                ),
            ));
        }
        let agent = rp.agent.unwrap_or_else(|| match &principal {
            Principal::ApiKey { name, .. } => name.clone(),
            Principal::Admin => "admin".into(),
        });
        // 显式 fact：用户亲口明示的事实直写落库（不走蒸馏——原话保真，不走概括）。
        // 这是用户授权的提格入口；蒸馏默认 inference 的保守原则只约束无声明路径。
        if rp.strength.as_deref() == Some("fact") {
            if rp.sensitive.unwrap_or(false) {
                return Err(mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    "sensitive 与 strength=fact 互斥——敏感内容走会话蒸馏通道",
                ));
            }
            let a = self
                .svc()
                .create_atom(
                    "fact",
                    &text,
                    0.9,
                    None,
                    None,
                    false,
                    Some("fact"),
                    Some("user_stated"),
                )
                .await
                .map_err(from_memory)?;
            let mut v = serde_json::to_value(&a).unwrap_or(serde_json::json!({}));
            v["hint"] = json!("已记住（显式 fact 直写——原话保真，不走蒸馏）");
            return ok_json(v);
        }
        let turns = serde_json::json!([{ "speaker": "user", "text": text }]);
        let s = self
            .svc()
            .write_session(&agent, turns, "auto", rp.sensitive.unwrap_or(false))
            .await
            .map_err(from_memory)?;
        let mut v = slim_session(serde_json::to_value(&s).unwrap_or(serde_json::json!({})));
        v["hint"] = json!(
            "已记住（auto 蒸馏，几分钟内可 search 命中）。写入验收：记下响应里的 session_id，用 action=\"distill_result\" 查蒸馏产物，或 action=\"list_atoms\" 浏览原子"
        );
        ok_json(v)
    }

    /// 更正记忆（快路径取代链）：用户说「你记错了，是 B 不是 A」时使用。
    ///
    /// 先 search 定位旧原子，再 correct(target_id, text)。单事务取代链：
    /// 新原子 active（继承旧条目的 kind，confidence 0.95/fact/user_stated），
    /// 旧原子 superseded 并指向新条目——历史可溯，检索立即生效。
    /// 治理：仅 active 且非敏感原子可被取代；更正对话照常写会话（蒸馏重抽会被仲裁判重复）。
    pub(crate) async fn memory_correct(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<CorrectParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let cp = params.0;
        let target_id = Uuid::parse_str(&cp.target_id).map_err(|_| {
            mcp_err(
                ErrorCode::INVALID_PARAMS,
                "target_id 不是合法 UUID——先 search 拿原子 id",
            )
        })?;
        let a = self
            .svc()
            .correct_atom(target_id, &cp.text)
            .await
            .map_err(from_memory)?;
        let mut v = serde_json::to_value(&a).unwrap_or(serde_json::json!({}));
        v["hint"] = json!("已更正（取代链留痕）——旧原子 superseded 指向本条，检索立即生效");
        ok_json(v)
    }

    /// 待审复核通过：摘掉 needs_review 标记（AI 代管复核）。
    ///
    /// 仅可处置 needs_review=true 的条目；处置留痕。批量复核场景：
    /// 用户说「把待审的过一遍」→ list_atoms(needs_review=true) 逐条判断后 confirm。
    pub(crate) async fn memory_confirm(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ReviewActionParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let rp = params.0;
        let id = Uuid::parse_str(&rp.atom_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "atom_id 不是合法 UUID"))?;
        let a = self.svc().confirm_review(id).await.map_err(from_memory)?;
        let mut v = serde_json::to_value(&a).unwrap_or(serde_json::json!({}));
        v["hint"] = json!("已复核通过——待审标记已摘除");
        ok_json(v)
    }

    /// 待审复核丢弃：归档该条（AI 代管复核）。
    ///
    /// 仅可处置 needs_review=true 的条目。归档后检索/上下文不再返回（留痕可溯）。
    pub(crate) async fn memory_discard(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ReviewActionParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let rp = params.0;
        let id = Uuid::parse_str(&rp.atom_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "atom_id 不是合法 UUID"))?;
        let a = self.svc().discard_review(id).await.map_err(from_memory)?;
        let mut v = serde_json::to_value(&a).unwrap_or(serde_json::json!({}));
        v["hint"] = json!("已丢弃（archived）——检索与上下文不再返回，留痕可溯");
        ok_json(v)
    }

    /// 编辑画像分面（AI 记忆管家）：用户说「画像里加上/改掉 XXX」时使用。
    ///
    /// version+1 落钉（manually_edited=true）——蒸馏产出对该分面落库前被守卫丢弃，
    /// 即编辑后蒸馏不会再覆盖。分面仅限七值之一；解冻走 Web 解钉（persona_unpin）。
    pub(crate) async fn memory_persona_edit(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<PersonaEditParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let ep = params.0;
        const ASPECTS: [&str; 7] = [
            "identity",
            "preferences",
            "skills",
            "constraints",
            "communication_style",
            "goals",
            "routines",
        ];
        if !ASPECTS.contains(&ep.aspect.as_str()) {
            return Err(mcp_err(
                ErrorCode::INVALID_PARAMS,
                format!("aspect 仅限 {}（收到 {}）", ASPECTS.join("/"), ep.aspect),
            ));
        }
        let actor = match &p {
            engram_core::auth::Principal::Admin => "admin".to_string(),
            engram_core::auth::Principal::ApiKey { name, .. } => format!("key:{name}"),
        };
        let v = self
            .svc()
            .persona_edit(&ep.aspect, &ep.content, &actor)
            .await
            .map_err(from_memory)?;
        let mut out = serde_json::to_value(&v).unwrap_or(serde_json::json!({}));
        out["hint"] =
            json!("分面已编辑并钉住（manually_edited）——蒸馏产出对该分面不再生效；解冻走 Web 解钉");
        ok_json(out)
    }

    /// 手动触发蒸馏链（AI 记忆管家）：撞车守卫——正在蒸馏时只提示不重复投递。
    ///
    /// 用户说「触发一下蒸馏」「把积压的蒸馏了」时使用。running 的 extract_atoms
    /// 存在时返回 already_running（不重复入队）；mode=sleep 为睡眠巩固预留位。
    pub(crate) async fn memory_distill(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<DistillParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let dp = params.0;
        let actor = match &p {
            engram_core::auth::Principal::Admin => "admin".to_string(),
            engram_core::auth::Principal::ApiKey { name, .. } => format!("key:{name}"),
        };
        let v = self
            .svc()
            .trigger_distill_manual(
                dp.full.unwrap_or(false),
                dp.mode.as_deref().unwrap_or("distill"),
                &actor,
            )
            .await
            .map_err(from_memory)?;
        ok_json(v)
    }

    /// 检索实体（用户记忆的横向透镜：人物/项目/主题/群组/地点）。
    ///
    /// 何时用：想按「某个具体的人/项目/主题」横向拉出相关记忆线索时；
    /// 或对话中出现新人物/项目，先查一下是否已有档案。
    /// 实体由蒸馏从会话中自动抽取维护——发现信息更新请写会话，不要要求直接改实体。
    pub(crate) async fn memory_entities(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<EntitiesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let hits = engram_search::search_entities(
            &self.state.pool,
            &params.0.q,
            params.0.limit.unwrap_or(20),
        )
        .await
        .map_err(|e| mcp_err(ErrorCode::INTERNAL_ERROR, e.to_string()))?;
        ok_json(serde_json::to_value(&hits).unwrap_or(serde_json::json!([])))
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_memory_write() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::memory_write_router()
}

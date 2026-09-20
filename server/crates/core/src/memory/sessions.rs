//! `memory` 的实现切片（架构治理 2026-09-20：自 memory.rs 纯搬移，零行为变化）。

use super::*;

impl MemoryService {
    // ---------- L0 ----------

    /// 写 L0 会话并按策略触发蒸馏。
    pub async fn write_session(
        &self,
        agent: &str,
        turns: serde_json::Value,
        distill: &str,
        sensitive: bool,
    ) -> Result<SessionDto, MemoryError> {
        self.write_session_identity(agent, turns, distill, sensitive, None, None, None)
            .await
    }

    /// 带身份归因与幂等键的写入（公网多Agent P001 步骤1）。
    /// api_key_id/key_name 由请求凭据自动注入；client_ref 命中唯一索引时返回既有会话
    /// （幂等——网络重试不产生重复会话）。
    #[allow(clippy::too_many_arguments)]
    pub async fn write_session_identity(
        &self,
        agent: &str,
        turns: serde_json::Value,
        distill: &str,
        sensitive: bool,
        api_key_id: Option<Uuid>,
        key_name_snapshot: Option<&str>,
        client_ref: Option<&str>,
    ) -> Result<SessionDto, MemoryError> {
        validate_distill(distill, &["auto", "manual", "off"])?;
        // D 观察项：空串 agent 归一化（回读 agent="" 无意义）
        let agent = agent.trim();
        let agent = if agent.is_empty() { "unknown" } else { agent };
        let Some(arr) = turns.as_array() else {
            return Err(MemoryError::BadRequest("content 必须是轮次数组".into()));
        };
        if arr.is_empty() {
            return Err(MemoryError::BadRequest("会话至少一轮".into()));
        }
        validate_turns(arr)?;
        let id = Uuid::now_v7();
        // v2 修复（H-A2）：distill=off 落 metadata 标记——蒸馏认领扫描据此豁免，
        // off 会话不再被后续任何 extract 任务的 pending 全量扫描"顺走"蒸掉。
        // off 是会话级永久语义（直到 void/erase），append 不改变它。
        let metadata = if distill == "off" {
            json!({"distill": "off"})
        } else {
            json!({})
        };
        let row = repo::insert_session_identity(
            &self.pool,
            id,
            agent,
            &turns,
            sensitive,
            &metadata,
            api_key_id,
            key_name_snapshot,
            client_ref,
        )
        .await?;

        // LLM 未配置时显式暴露（MCP 黑盒测试 D1：蒸馏静默失败不可接受——
        // manual 最应显式失败；auto 已入库但提示不会蒸馏）
        self.require_llm_for_distill(distill).await?;

        match distill {
            "auto" => {
                engram_distill::trigger_auto_extract(&self.queue, self.debounce_secs)
                    .await
                    .ok();
            }
            "manual" => {
                self.queue
                    .enqueue(
                        JobTemplate::new("extract_atoms").with_payload(json!({"reason": "manual"})),
                    )
                    .await
                    .ok();
            }
            _ => {}
        }
        Ok(row)
    }

    /// 批量导入历史对话为会话（phase-2）：JSONL（每行 {role, content}）或纯文本（空行分段）。
    /// metadata.source = "import"——蒸馏据此感知「导入的历史，对方的话是素材不是用户事实」。
    pub async fn import_session(
        &self,
        agent: &str,
        content: &str,
        format: &str,
        distill: &str,
    ) -> Result<SessionDto, MemoryError> {
        validate_distill(distill, &["auto", "manual", "off"])?;
        let turns = Self::parse_import(content, format)?;
        // M-1/SEC-B 同口径：导入的 turns 也过校验（jsonl 行 content 为空同样挡）
        if let Some(arr) = turns.as_array() {
            validate_turns(arr)?;
        }
        let id = Uuid::now_v7();
        let row = repo::insert_session_import(&self.pool, id, agent, &turns).await?;
        match distill {
            "auto" => {
                engram_distill::trigger_auto_extract(&self.queue, self.debounce_secs)
                    .await
                    .ok();
            }
            "manual" => {
                self.queue
                    .enqueue(
                        JobTemplate::new("extract_atoms").with_payload(json!({"reason": "import"})),
                    )
                    .await
                    .ok();
            }
            _ => {}
        }
        Ok(row)
    }

    /// 增量追加轮次到既有会话（长对话分片落库，不等收尾——自动节律 b 配套）。
    /// 只允许追加未蒸馏（pending）会话：已蒸馏的会话追加会割裂 L1 溯源。
    /// agent 可选补记（首个 append 补上会话归属，多 agent 视角的数据从现在记对）。
    pub async fn append_session(
        &self,
        id: Uuid,
        turns: serde_json::Value,
        agent: Option<&str>,
        distill: &str,
    ) -> Result<SessionDto, MemoryError> {
        // append 无 manual 语义（off 会话本就被豁免）；乱传 manual 此前被静默忽略
        validate_distill(distill, &["auto", "off"])?;
        let Some(arr) = turns.as_array() else {
            return Err(MemoryError::BadRequest("content 必须是轮次数组".into()));
        };
        if arr.is_empty() {
            return Err(MemoryError::BadRequest("追加至少一轮".into()));
        }
        validate_turns(arr)?;
        let cur = repo::find_session(&self.pool, id)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("会话 {id} 不存在")))?;
        if cur.distill_status != "pending" {
            return Err(MemoryError::BadRequest(format!(
                "会话已蒸馏（{}），不可追加——请开新会话",
                cur.distill_status
            )));
        }
        let row = repo::append_session_update(
            &self.pool,
            id,
            &serde_json::Value::Array(arr.to_vec()),
            agent,
        )
        .await?;
        // 追加同样走防抖：同窗口的追加与首写共用一个 extract 任务
        if distill == "auto" {
            engram_distill::trigger_auto_extract(&self.queue, self.debounce_secs)
                .await
                .ok();
        }
        Ok(row)
    }

    pub async fn list_sessions(
        &self,
        agent: Option<&str>,
        cursor: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<SessionDto>, MemoryError> {
        if limit < 0 {
            return Err(MemoryError::BadRequest(format!(
                "limit 不能为负（收到 {limit}）"
            )));
        }
        Ok(repo::list_sessions(&self.pool, agent, cursor, limit.min(200)).await?)
    }

    /// 会话列表轻量行（浏览/定位用）：轮次数 + 首条消息预览，不含正文数组。
    /// MCP memory_list_sessions 用——按需取用而非截断全文。
    pub async fn list_sessions_meta(
        &self,
        agent: Option<&str>,
        cursor: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<Value>, MemoryError> {
        if limit < 0 {
            return Err(MemoryError::BadRequest(format!(
                "limit 不能为负（收到 {limit}）"
            )));
        }
        Ok(repo::list_sessions_meta(&self.pool, agent, cursor, limit.min(200)).await?)
    }

    pub async fn get_session(&self, id: Uuid) -> Result<SessionDto, MemoryError> {
        repo::find_session(&self.pool, id)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("会话 {id} 不存在")))
    }

    /// L0 擦除：删会话 + 引用它的 atoms 标记来源失效。
    /// erase 是「物理删除」语义——派生原子也必须撤出检索：删除前把源自该会话的
    /// active/superseded 原子级联归档（与 void/purge_agent 同口径；多源原子同被归档，
    /// 保守可恢复）。此前只标 source_refs erased，原子仍在检索里，违背遗忘承诺。
    pub async fn erase_session(&self, id: Uuid) -> Result<(), MemoryError> {
        let archived = repo::archive_atoms_by_session(&self.pool, &id.to_string()).await?;
        let affected = repo::delete_session(&self.pool, id).await?;
        if affected == 0 {
            return Err(MemoryError::NotFound(format!("会话 {id} 不存在")));
        }
        // 来源失效标记：source_refs 里含该会话的原子加 erased 标记
        let atoms = repo::list_atom_refs_like(&self.pool, id).await?;
        for (aid, refs) in atoms {
            let marked = mark_erased(refs, id);
            repo::update_atom_source_refs(&self.pool, aid, &marked).await?;
        }
        if archived > 0 {
            self.audit(
                "session_erase_cascade",
                json!({"session": id.to_string(), "archived_atoms": archived}),
            )
            .await;
        }
        // 与 void 同一套孤儿清扫（2026-09-08 用户：画像空了圈子里怎么还有东西）——
        // erase 归档原子后，挂链原子全部失效的实体也要退场，否则圈子和画像口径分裂。
        // 场景收敛一并触发（快照里可能引用被归档的成员）。
        self.queue
            .enqueue(
                JobTemplate::new("organize_scenarios")
                    .with_payload(json!({ "converge_only": true }))
                    .with_idempotency_key(format!("erase-converge-{id}")),
            )
            .await
            .ok();
        repo::archive_orphan_entities(&self.pool).await?;
        Ok(())
    }

    /// P5 会话作废：「这段白记了」——标记 void，蒸馏跳过（claim 只取 pending）。
    /// 只允许 pending 会话作废（已蒸馏的产出用 purge 清场处理）。
    /// M-2（2026-09-03）：「不存在」404 与「非 pending」400 分开报，不再合并一句。
    /// P5 会话作废（v2 扩大语义）：「这段白记了」。
    /// - pending/off 会话：标记 void，蒸馏跳过（原文保留可审计）；
    /// - done 会话：void + **级联归档**其蒸馏产出的 active 原子（v2 修复 P0-3 遗忘断层——
    ///   此前已蒸馏会话无任何遗忘手段，错误文案指路的 purge 又不在 MCP 工具面）。
    ///   归档保留原文与原子（可审计、可恢复），检索/context 不再返回。
    /// - processing：仍拒绝（蒸馏 worker 持有中，稍后重试）。
    pub async fn void_session(&self, id: Uuid) -> Result<SessionDto, MemoryError> {
        let status = repo::session_distill_status(&self.pool, id).await?;
        match status.as_deref() {
            None => return Err(MemoryError::NotFound(format!("会话 {id} 不存在"))),
            Some("processing") => {
                return Err(MemoryError::BadRequest(format!(
                    "会话 {id} 正在蒸馏（processing）——请稍后重试作废"
                )));
            }
            Some("void") => {
                return Err(MemoryError::BadRequest(format!(
                    "会话 {id} 已处理（当前状态 void）——已作废，无需重复操作"
                )));
            }
            _ => {}
        }
        let row = repo::void_session_update(&self.pool, id).await?;
        let row = row.ok_or_else(|| {
            // 查询与更新之间的竞态兜底（状态刚被蒸馏 worker 抢走）
            MemoryError::BadRequest(format!(
                "会话 {id} 刚被蒸馏任务取走（processing）——请稍后重试作废"
            ))
        })?;
        // done 会话：级联归档其蒸馏产出的 active 原子（与 purge_agent 同口径）
        if status.as_deref() == Some("done") {
            let archived = repo::archive_atoms_by_session(&self.pool, &id.to_string()).await?;
            // 审计链：job 行记遗忘动作（与 purge/erase 同哲学）
            self.audit(
                "session_void_cascade",
                json!({"session": id.to_string(), "archived_atoms": archived as i64}),
            )
            .await;
        }
        // D15/D16：遗忘级联（任何 void 都触发；converge 与孤儿归档均幂等）——
        // L2 场景收敛 + L3/实体层的孤儿回收，保证「遗忘」后各层即时干净。
        // 历史遗留的孤儿实体（修复前产生）也能借此回收
        self.queue
            .enqueue(
                JobTemplate::new("organize_scenarios")
                    .with_payload(json!({ "converge_only": true }))
                    .with_idempotency_key(format!("void-converge-{id}")),
            )
            .await
            .ok();
        repo::archive_orphan_entities(&self.pool).await?;
        Ok(row)
    }

    /// 恢复 void 会话（forget mode="restore"，R 报告建议 #7：void 原文保留，恢复成本低）：
    /// - distill_status 还原自作废时的存档（pending→pending 可继续蒸馏，done→done）；
    /// - 级联归档的原子一并恢复（superseded_by 在 → superseded；否则 → active）。
    /// - 返回 (恢复的会话, 恢复的原子数)。
    pub async fn unvoid_session(&self, id: Uuid) -> Result<(SessionDto, u64), MemoryError> {
        let status = repo::session_distill_status(&self.pool, id).await?;
        match status.as_deref() {
            None => return Err(MemoryError::NotFound(format!("会话 {id} 不存在"))),
            Some("processing") => {
                return Err(MemoryError::BadRequest(format!(
                    "会话 {id} 正在蒸馏（processing）——等蒸馏完成后再作废/恢复"
                )));
            }
            Some("void") => {}
            Some(other) => {
                return Err(MemoryError::BadRequest(format!(
                    "会话 {id} 未被作废（当前状态 {other}）——restore 只对 void 会话有意义"
                )));
            }
        }
        let row = repo::unvoid_session_update(&self.pool, id)
            .await?
            .ok_or_else(|| {
                MemoryError::BadRequest(format!("会话 {id} 刚被蒸馏任务取走——请稍后重试"))
            })?;
        let restored = repo::restore_atoms_by_session(&self.pool, &id.to_string()).await?;
        self.audit(
        "session_unvoid_restore",
        json!({"session": id.to_string(), "restored_atoms": restored, "to": row.distill_status}),
    )
    .await;
        // 还原成 pending 的会话恢复蒸馏资格——触发一次防抖扫描（off 豁免由 metadata 保留）
        if row.distill_status == "pending" {
            engram_distill::trigger_auto_extract(&self.queue, self.debounce_secs)
                .await
                .ok();
        }
        // 恢复的 active 原子需要场景收敛重算（幂等）
        if restored > 0 {
            self.queue
                .enqueue(
                    JobTemplate::new("organize_scenarios")
                        .with_payload(json!({ "converge_only": true }))
                        .with_idempotency_key(format!("unvoid-converge-{id}")),
                )
                .await
                .ok();
        }
        Ok((row, restored))
    }

    /// 批量撤销作废（Web「恢复所选」）：逐条 unvoid，单条失败不影响其余——
    /// 混合选择（含非 void）时失败项逐条带原因返回，前端据此汇总提示。
    pub async fn unvoid_sessions(&self, ids: &[Uuid]) -> BatchOutcome {
        let mut out = BatchOutcome::default();
        for id in ids {
            match self.unvoid_session(*id).await {
                Ok(_) => out.succeeded += 1,
                Err(e) => out.failed.push(BatchFailure {
                    id: *id,
                    error: e.to_string(),
                }),
            }
        }
        out
    }

    /// 批量擦除（Web「擦除所选」）：逐条 erase_session（含原子级联），逐条成败互不影响。
    pub async fn erase_sessions(&self, ids: &[Uuid]) -> BatchOutcome {
        let mut out = BatchOutcome::default();
        for id in ids {
            match self.erase_session(*id).await {
                Ok(()) => out.succeeded += 1,
                Err(e) => out.failed.push(BatchFailure {
                    id: *id,
                    error: e.to_string(),
                }),
            }
        }
        out
    }

    /// P-C 阶段一：arm——入队 5 分钟冷却的 deep_purge job（后悔药窗口）。
    pub async fn arm_deep_purge(&self, source: &str) -> Result<Job, MemoryError> {
        // 秒级防抖：同秒连点只 arm 一次；跨秒可重新 arm（取消后立刻重 arm 是正当操作）
        let bucket = chrono::Utc::now().timestamp();
        self.queue
            .enqueue(
                JobTemplate::new("deep_purge")
                    .with_idempotency_key(format!("deep-purge-arm-{bucket}"))
                    .with_payload(json!({
                        "phase": "armed",
                        "confirm": PURGE_CONFIRM_PHRASE,
                        "authorized_by": source,
                    }))
                    .with_due(chrono::Utc::now() + chrono::Duration::minutes(5)),
            )
            .await
            .map_err(|e| MemoryError::Storage(e.to_string()))
    }
    /// LLM 未配置时显式暴露（MCP 黑盒测试 D1：蒸馏静默失败不可接受——
    /// manual 最应显式失败；auto 已入库但提示不会蒸馏）。
    async fn require_llm_for_distill(&self, distill: &str) -> Result<(), MemoryError> {
        if distill != "off"
            && let Err(e) = self
                .registry
                .resolve(engram_llm::types::Purpose::Extract)
                .await
        {
            let msg = format!(
                "LLM 未配置或不可用（{e}）——蒸馏无法执行。请管理员在「设置 → AI 功能」配置模型后重试"
            );
            if distill == "manual" {
                return Err(MemoryError::LlmNotConfigured(msg));
            }
            tracing::warn!("{msg}（auto 会话已入库，distill_status 保持 pending）");
        }
        Ok(())
    }
}

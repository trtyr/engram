//! `memory` 的实现切片（架构治理 2026-09-20：自 memory.rs 纯搬移，零行为变化）。

use super::*;

impl MemoryService {
    /// 手动触发蒸馏（AI 记忆管家）：撞车守卫——running extract_atoms 存在时只提示不投递
    /// （任务列表不留空跑记录）。mode："distill"（默认）/ "rebuild"（画像全量重建）/
    /// "sleep"（预留——内置节律线后开放）。
    /// P015：手动触发离线整理 Agent（判重/归档/画像）。撞车守卫：运行中即提示。
    pub async fn trigger_maintain(&self, by: &str) -> Result<serde_json::Value, MemoryError> {
        let running = repo::count_running_maintain(&self.pool).await?;
        if running > 0 {
            return Ok(serde_json::json!({
                "already_running": true,
                "hint": "整理巡逻进行中——等它完成看效果",
            }));
        }
        let job = self
            .queue
            .enqueue(
                engram_jobs::JobTemplate::new("maintain_memory")
                    .with_payload(serde_json::json!({"reason": "manual", "triggered_by": by})),
            )
            .await
            .map_err(|e| MemoryError::Storage(e.to_string()))?;
        Ok(serde_json::json!({
            "already_running": false,
            "jobs": [{ "id": job.id.to_string(), "kind": job.kind }],
        }))
    }

    /// 画像活文档（P015）：当前版 + 近 N 版历史。
    pub async fn persona_doc(&self, history_limit: i64) -> Result<serde_json::Value, MemoryError> {
        let doc = repo::persona_doc_get(&self.pool)
            .await
            .map_err(|e| MemoryError::Storage(e.to_string()))?;
        let history = repo::persona_doc_history(&self.pool, history_limit)
            .await
            .map_err(|e| MemoryError::Storage(e.to_string()))?;
        Ok(serde_json::json!({
            "doc": doc,
            "history": history,
        }))
    }

    // ---------- KV 值保值通道（蒸馏零介入——value 逐字保存） ----------

    /// JEV 哨兵解析（T017）：settings + cipher → 可用客户端；不可用 → None（放行）。
    async fn jev_client(&self) -> Option<engram_llm::decisions::JevClient> {
        let cipher = self.cipher.as_ref()?;
        let cfg: engram_llm::decisions::JevConfig = engram_storage::repo::settings::get_json(
            &self.pool,
            engram_llm::decisions::SETTINGS_KEY,
        )
        .await
        .unwrap_or_default();
        engram_llm::decisions::resolve(&cfg, cipher, reqwest::Client::new()).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "JEV 配置解析失败——KV 闸降级放行");
            None
        })
    }

    /// 写入/更新一个结构化值。同 key 就地覆盖（可变状态不产生取代链）。
    /// T017：JEV 准入闸前置——非 kv_fit 判定拒绝入库并建议去处（失败/未配置放行）。
    pub async fn kv_put(
        &self,
        key: &str,
        value: &str,
        context: Option<&str>,
        tags: Option<Vec<String>>,
        source: Option<&str>,
    ) -> Result<KvEntryDto, MemoryError> {
        let k = key.trim();
        if k.is_empty() || k.chars().count() > 200 {
            return Err(MemoryError::BadRequest("key 不能为空且 ≤200 字符".into()));
        }
        let v = value;
        if v.is_empty() {
            return Err(MemoryError::BadRequest(
                "value 不能为空——KV 是精确值通道，不放空话".into(),
            ));
        }
        // T017 准入闸：哨兵可用才判定；失败/未配置降级放行（显式动作不阻塞）
        if let Some(client) = self.jev_client().await {
            let state =
                serde_json::json!({ "key": k, "value": v, "context": context.unwrap_or("") });
            match client
                .decide(state, engram_llm::decisions::kv_gate_questions())
                .await
            {
                Ok(result) => {
                    let verdict = result
                        .answers
                        .get("gate")
                        .and_then(|a| a.choice())
                        .unwrap_or("kv_fit")
                        .to_string();
                    if verdict != "kv_fit" {
                        return Err(MemoryError::BadRequest(format!(
                            "KV 准入闸拒绝（JEV 判定: {verdict}）——{}",
                            engram_llm::decisions::kv_gate_reject_hint(&verdict)
                        )));
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, key = %k, "JEV KV 闸失败——放行（降级直通）");
                }
            }
        }
        const SOURCES: [&str; 4] = ["user_stated", "verified_probe", "agent_inferred", "doc"];
        let src = source.unwrap_or("user_stated");
        if !SOURCES.contains(&src) {
            return Err(MemoryError::BadRequest(format!(
                "source 仅接受 user_stated/verified_probe/agent_inferred/doc（收到 {src}）"
            )));
        }
        let row = repo::kv_upsert(
            &self.pool,
            k,
            v,
            context.unwrap_or("").trim(),
            &tags.unwrap_or_default(),
            src,
        )
        .await?;
        // T020：写入生命周期（元数据零语义——记 key 与长度，不打 value 正文）
        self.emit_mem_log(
            "kv_put",
            serde_json::json!({ "key": k, "value_chars": v.chars().count() }),
        )
        .await;
        Ok(row)
    }

    pub async fn kv_get(&self, key: &str) -> Result<Option<KvEntryDto>, MemoryError> {
        Ok(repo::kv_get(&self.pool, key.trim())
            .await?
            .map(kv_stale_hint))
    }

    pub async fn kv_list(&self, limit: i64) -> Result<Vec<KvEntryDto>, MemoryError> {
        Ok(repo::kv_list(&self.pool, limit.clamp(1, 500))
            .await?
            .into_iter()
            .map(kv_stale_hint)
            .collect())
    }

    /// 字面量直查（key/value/context ILIKE）——精确值不依赖分词。
    pub async fn kv_search(&self, query: &str, limit: i64) -> Result<Vec<KvEntryDto>, MemoryError> {
        let q = query.trim();
        if q.chars().count() < 3 {
            return Err(MemoryError::BadRequest(
                "检索词至少 3 字符（防全表扫短串）".into(),
            ));
        }
        Ok(repo::kv_search_literal(&self.pool, q, limit.clamp(1, 100))
            .await?
            .into_iter()
            .map(kv_stale_hint)
            .collect())
    }

    /// 用户手动编辑画像活文档（与离线整理 Agent 同一条 save_doc 通道，版本链留痕）。
    pub async fn persona_doc_edit(
        &self,
        content: &str,
        summary: Option<&str>,
    ) -> Result<serde_json::Value, MemoryError> {
        repo::persona_doc_save(&self.pool, content, summary)
            .await
            .map_err(|e| MemoryError::Storage(e.to_string()))?;
        let doc = repo::persona_doc_get(&self.pool)
            .await
            .map_err(|e| MemoryError::Storage(e.to_string()))?;
        Ok(serde_json::json!({ "doc": doc }))
    }

    pub async fn purge_agent(&self, agent: &str) -> Result<(i64, i64), MemoryError> {
        Ok(repo::purge_agent_tx(&self.pool, agent).await?)
    }

    /// F1/F2 deep purge（终极清空测试）：记忆域四层 + 实体链一键清空，单事务，
    /// 返回五计数。TRUNCATE CASCADE 一发解 FK——API 层负责 erase scope + confirm 双因子。
    pub async fn purge_deep(&self) -> Result<serde_json::Value, MemoryError> {
        purge_deep_pool(&self.pool).await.map_err(MemoryError::from)
    }

    /// 编辑/清空类审计：写一条已完成的 job 行（谁、何时、干了什么）——不可抵赖凭证。
    pub async fn audit(&self, kind: &str, payload: serde_json::Value) {
        repo::audit(&self.pool, kind, payload).await;
    }

    /// T020：记忆域生命周期日志——元数据零语义（不打正文），domain=memory。
    pub async fn emit_mem_log(&self, action: &str, fields: serde_json::Value) {
        let mut f = serde_json::Map::new();
        f.insert("action".into(), serde_json::Value::String(action.into()));
        if let Some(obj) = fields.as_object() {
            for (k, v) in obj {
                f.insert(k.clone(), v.clone());
            }
        }
        engram_storage::repo::logs::emit_log(
            &self.pool,
            "info",
            "memory.lifecycle",
            action,
            Some(serde_json::Value::Object(f)),
            "memory",
        )
        .await
        .ok();
    }

    /// P4 全量导出（数据主权）：记忆域三表完整快照，JSON 随身带走（P015：scenarios/persona 已退役）。
    /// sensitive 原子是否包含由调用方决定（决策 001 后生产路由默认包含——
    /// 隐私面语义随决策 001 收敛为「标记不排除」；include_sensitive 只是开关）。
    pub async fn export(&self, include_sensitive: bool) -> Result<serde_json::Value, MemoryError> {
        let sessions = repo::list_all_sessions(&self.pool).await?;
        let atoms = repo::list_atoms_all(&self.pool, include_sensitive).await?;
        let entities = repo::entities_for_export(&self.pool).await?;
        Ok(serde_json::json!({
            "format": "engram-memory-export",
            "version": 1,
            "exported_at": chrono::Utc::now(),
            "counts": {
                "sessions": sessions.len(), "atoms": atoms.len(),
                "entities": entities.len(),
            },
            "sensitive_excluded": !include_sensitive,
            "sessions": sessions, "atoms": atoms, "entities": entities,
        }))
    }

    /// 全局记忆时间轴：原子（occurred_at 优先）/场景/实体按时间倒序合并。
    pub async fn timeline(&self, limit: i64) -> Result<Vec<TimelineEvent>, MemoryError> {
        Ok(repo::timeline(&self.pool, limit).await?)
    }

    /// EN-241：KV 删除——物理删除指定 key（返回是否删了；不存在返回 false 不报错）。
    pub async fn kv_delete(&self, key: &str) -> Result<bool, MemoryError> {
        let k = key.trim();
        if k.is_empty() {
            return Err(MemoryError::BadRequest("key 不能为空".into()));
        }
        let deleted = repo::kv_delete(&self.pool, k).await?;
        if deleted {
            self.audit("kv_delete", serde_json::json!({ "key": k }))
                .await;
        }
        Ok(deleted)
    }

    /// EN-242：同名实体检测（大小写/首尾空白不敏感分组）——返回 (归一化名, 数量, id 列表)。
    pub async fn entity_duplicates(&self) -> Result<Vec<(String, i64, Vec<Uuid>)>, MemoryError> {
        let rows = repo::duplicate_name_rows(&self.pool).await?;
        let mut groups: std::collections::BTreeMap<String, Vec<Uuid>> =
            std::collections::BTreeMap::new();
        for (id, key) in rows {
            groups.entry(key).or_default().push(id);
        }
        let mut out: Vec<(String, i64, Vec<Uuid>)> = groups
            .into_iter()
            .map(|(k, ids)| (k, ids.len() as i64, ids))
            .collect();
        out.sort_by_key(|x| std::cmp::Reverse(x.1));
        Ok(out)
    }
}

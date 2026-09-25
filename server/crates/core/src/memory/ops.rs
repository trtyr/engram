//! `memory` 的实现切片（架构治理 2026-09-20：自 memory.rs 纯搬移，零行为变化）。

use super::*;

impl MemoryService {
    pub async fn trigger_distill(
        &self,
        full: bool,
        via: &str,
        by: &str,
    ) -> Result<Vec<Job>, MemoryError> {
        engram_distill::chain::trigger(&self.queue, full, via, by)
            .await
            .map_err(|e| MemoryError::Storage(e.to_string()))
    }

    /// 手动触发蒸馏（AI 记忆管家）：撞车守卫——running extract_atoms 存在时只提示不投递
    /// （任务列表不留空跑记录）。mode："distill"（默认）/ "rebuild"（画像全量重建）/
    /// "sleep"（预留——内置节律线后开放）。
    pub async fn trigger_distill_manual(
        &self,
        full: bool,
        mode: &str,
        by: &str,
    ) -> Result<serde_json::Value, MemoryError> {
        if mode == "sleep" {
            return Err(MemoryError::BadRequest(
                "睡眠（记忆巩固）尚未上线——依赖内置节律线，敬请期待".into(),
            ));
        }
        if mode == "rebuild" {
            // 画像全量重建（收录哲学线 task-10）：不入蒸馏链，直接投递 distill_persona
            // 全量重建任务（payload.full_rebuild → persona.rs 以全部场景重算所有非钉住分面）。
            // 撞车守卫：running 的 distill_persona 存在时只提示不投递（与 extract 同模）。
            let running = repo::count_running_persona(&self.pool).await?;
            if running > 0 {
                return Ok(serde_json::json!({
                    "already_running": true,
                    "hint": "画像全量重建进行中——等它完成看效果，再决定是否重来",
                }));
            }
            let job = self
                .queue
                .enqueue(
                    engram_jobs::JobTemplate::new("distill_persona")
                        .with_payload(serde_json::json!({"full_rebuild": true}))
                        .with_idempotency_key(format!(
                            "persona-rebuild-{}",
                            chrono::Utc::now().format("%Y%m%d%H%M")
                        )),
                )
                .await
                .map_err(|e| MemoryError::Storage(e.to_string()))?;
            return Ok(serde_json::json!({
                "already_running": false,
                "hint": "画像全量重建已触发：以全部场景重算所有非钉住分面（manually_edited 豁免）",
                "jobs": [{ "id": job.id.to_string(), "kind": job.kind }],
            }));
        }
        if mode != "distill" {
            return Err(MemoryError::BadRequest(format!(
                "mode 仅支持 distill / sleep（收到 {mode}）"
            )));
        }
        let running = repo::count_running_extract(&self.pool).await?;
        if running > 0 {
            return Ok(serde_json::json!({
                "already_running": true,
                "running": running,
                "hint": "当前正在蒸馏中——等它完成看看效果，再决定是否手动触发",
            }));
        }
        let jobs = self.trigger_distill(full, "manual", by).await?;
        let list: Vec<serde_json::Value> = jobs
            .iter()
            .map(|j| serde_json::json!({ "id": j.id.to_string(), "kind": j.kind }))
            .collect();
        Ok(serde_json::json!({
            "already_running": false,
            "hint": if full {
                "已触发蒸馏 + 全量整理（consolidate）"
            } else {
                "已触发蒸馏链（抽取→仲裁→归组→画像）"
            },
            "jobs": list,
        }))
    }

    // ---------- KV 值保值通道（蒸馏零介入——value 逐字保存） ----------

    /// 写入/更新一个结构化值。同 key 就地覆盖（可变状态不产生取代链）。
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

    // ---------- L2 / L3 ----------

    pub async fn list_scenarios(&self, limit: i64) -> Result<Vec<ScenarioDto>, MemoryError> {
        Ok(repo::list_scenarios(&self.pool, limit.min(200)).await?)
    }

    pub async fn get_scenario(&self, id: Uuid) -> Result<ScenarioDto, MemoryError> {
        repo::find_scenario(&self.pool, id)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("场景 {id} 不存在")))
    }

    /// 当前画像（每分面最新版）。
    pub async fn persona(&self) -> Result<Vec<PersonaVersion>, MemoryError> {
        Ok(repo::persona_current(&self.pool).await?)
    }

    /// 分面版本历史。
    pub async fn persona_history(&self, aspect: &str) -> Result<Vec<PersonaVersion>, MemoryError> {
        Ok(repo::persona_history(&self.pool, aspect).await?)
    }

    /// P11/SEC-E 按 agent 清场（测试隔离，2026-09-03 彻底化）：该 agent **全部**会话
    /// 物理删除（pending/processing/done/void 一视同仁，sensitive 原文不留——
    /// 此前 done 会话残留曾导致敏感原始对话留库）+ 其产出的 active 原子归档（可恢复）。
    /// 返回 (erased_sessions, archived_atoms)。顺序敏感：先归档原子（JOIN 会话判归属）
    /// 再删会话——删会话后 JOIN 不可判归属。
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

    /// P4 全量导出（数据主权）：记忆域五表完整快照，JSON 随身带走。
    /// R4：sensitive 原子默认排除（隐私面不随导出扩大到文件系统），
    /// include_sensitive=true 显式包含——与检索 reveal 同权。
    pub async fn export(&self, include_sensitive: bool) -> Result<serde_json::Value, MemoryError> {
        let sessions = repo::list_all_sessions(&self.pool).await?;
        let atoms = repo::list_atoms_all(&self.pool, include_sensitive).await?;
        let scenarios = repo::list_scenarios_all(&self.pool).await?;
        let persona = repo::persona_all(&self.pool).await?;
        let entities = repo::entities_for_export(&self.pool).await?;
        Ok(serde_json::json!({
            "format": "engram-memory-export",
            "version": 1,
            "exported_at": chrono::Utc::now(),
            "counts": {
                "sessions": sessions.len(), "atoms": atoms.len(),
                "scenarios": scenarios.len(), "persona": persona.len(),
                "entities": entities.len(),
            },
            "sensitive_excluded": !include_sensitive,
            "sessions": sessions, "atoms": atoms, "scenarios": scenarios,
            "persona": persona, "entities": entities,
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

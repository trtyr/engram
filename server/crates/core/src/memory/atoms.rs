//! `memory` 的实现切片（架构治理 2026-09-20：自 memory.rs 纯搬移，零行为变化）。

use super::*;

impl MemoryService {
    /// 解析导入文本 → turns（[{speaker, text}]）。jsonl：每行 {role, content}；text：空行分段交替。
    pub(super) fn parse_import(
        content: &str,
        format: &str,
    ) -> Result<serde_json::Value, MemoryError> {
        let turns: Vec<serde_json::Value> = match format {
        "jsonl" => {
            let mut out = Vec::new();
            for (i, line) in content.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let v: serde_json::Value = serde_json::from_str(line).map_err(|e| {
                    MemoryError::BadRequest(format!(
                        "第 {} 行不是合法 JSON：{}（格式：每行 {{\"role\":\"user\"|\"assistant\",\"content\":\"...\"}}）",
                        i + 1,
                        e
                    ))
                })?;
                let role = v.get("role").and_then(|r| r.as_str()).unwrap_or("");
                let text = v.get("content").and_then(|c| c.as_str()).unwrap_or("").trim();
                if text.is_empty() {
                    continue;
                }
                let speaker = match role {
                    "user" | "human" => "user",
                    "assistant" | "ai" | "bot" => "assistant",
                    _ => {
                        return Err(MemoryError::BadRequest(format!(
                            "第 {} 行 role 必须是 user/assistant（含 human/ai 别名），实得 {:?}",
                            i + 1,
                            role
                        )))
                    }
                };
                out.push(serde_json::json!({"speaker": speaker, "text": text}));
            }
            out
        }
        "text" => content
            .split("\n\n")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .enumerate()
            .map(|(i, seg)| {
                serde_json::json!({"speaker": if i % 2 == 0 { "user" } else { "assistant" }, "text": seg})
            })
            .collect(),
        _ => {
            return Err(MemoryError::BadRequest(format!(
                "未知导入格式 {:?}：支持 jsonl / text",
                format
            )))
        }
    };
        if turns.is_empty() {
            return Err(MemoryError::BadRequest(
                "导入内容为空——没有任何有效轮次".into(),
            ));
        }
        Ok(serde_json::Value::Array(turns))
    }

    // ---------- L1 ----------

    pub async fn list_atoms(
        &self,
        kind: Option<&str>,
        status: Option<&str>,
        needs_review: Option<bool>,
        cursor: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<AtomDto>, MemoryError> {
        if limit < 0 {
            return Err(MemoryError::BadRequest(format!(
                "limit 不能为负（收到 {limit}）"
            )));
        }
        Ok(repo::list_atoms(
            &self.pool,
            kind,
            status,
            needs_review,
            cursor,
            limit.min(500),
        )
        .await?)
    }

    /// 手工新增（待审补充；active 直接入库）。
    #[allow(clippy::too_many_arguments)]
    pub async fn create_atom(
        &self,
        kind: &str,
        content: &str,
        confidence: f32,
        occurred_at: Option<DateTime<Utc>>,
        valid_until: Option<DateTime<Utc>>,
        sensitive: bool,
        strength: Option<&str>,
        source_kind: Option<&str>,
    ) -> Result<AtomDto, MemoryError> {
        const STRENGTHS: [&str; 3] = ["fact", "inference", "assumption"];
        const SOURCES: [&str; 4] = ["user_stated", "verified_probe", "agent_inferred", "doc"];
        if let Some(v) = strength
            && !STRENGTHS.contains(&v)
        {
            return Err(MemoryError::BadRequest(format!(
                "strength 仅接受 fact/inference/assumption（收到 {v}）"
            )));
        }
        if let Some(v) = source_kind
            && !SOURCES.contains(&v)
        {
            return Err(MemoryError::BadRequest(format!(
                "source 仅接受 user_stated/verified_probe/agent_inferred/doc（收到 {v}）"
            )));
        }
        let text = content.trim();
        // 输入校验：空内容 + 超长（对齐蒸馏链的 1~120 字契约）
        if text.is_empty() {
            return Err(MemoryError::BadRequest("原子内容不能为空".into()));
        }
        if text.chars().count() > 120 {
            return Err(MemoryError::BadRequest(format!(
                "原子内容超长：最多 120 字，当前 {} 字——原子只收一句话；成段内容走 write_session（蒸馏自动切分），remember 用默认蒸馏路径（不带 strength=fact）",
                text.chars().count()
            )));
        }
        // A4 幂等护栏：同 kind + 同内容（trim 后）的 active 原子已存在则直接返回它——
        // AI 重试/重复直写不会双份（2026-08-31 测试方实测两条一模一样的生日原子）。
        // 近重复的语义合并仍归 arbitrate/consolidate，这里只挡精确重复。
        if let Some(existing) = repo::find_active_atom(&self.pool, kind, text).await? {
            tracing::info!(atom_id = %existing.id, "直写命中已有同内容原子，幂等返回");
            return Ok(existing);
        }
        // A1：与蒸馏链同规则——置信 <0.55 自动进待审，不直接生效污染记忆库
        // （此前直写硬编码 needs_review=false，文档/CLI 提示/实现三方打架）。
        let needs_review = confidence < 0.55;
        let id = Uuid::now_v7();
        let emb = self.try_embed(&[text.to_string()]).await;
        let row = repo::insert_atom(
            &self.pool,
            id,
            kind,
            text,
            confidence,
            needs_review,
            sensitive,
            occurred_at,
            valid_until,
            emb.and_then(|v| v.into_iter().next()),
            &engram_search::tokenize::tsv_text(text),
            strength.unwrap_or("fact"),
            source_kind.unwrap_or("user_stated"),
        )
        .await?;
        Ok(row)
    }

    /// correct 快路径（AI 记忆管家，收录哲学线）：单事务取代链——
    /// 新原子 active（继承 target 的 kind，confidence 0.95/fact/user_stated），
    /// 旧原子 superseded + superseded_by 指针。治理：仅 active 且非 sensitive 的目标。
    pub async fn correct_atom(&self, target_id: Uuid, text: &str) -> Result<AtomDto, MemoryError> {
        let cur = repo::find_atom(&self.pool, target_id)
            .await?
            .ok_or_else(|| {
                MemoryError::NotFound(format!("目标原子 {target_id} 不存在——先 search 定位再更正"))
            })?;
        if cur.status != "active" {
            return Err(MemoryError::BadRequest(format!(
                "目标原子已是 {} 状态，仅 active 原子可被取代——先 search 找最新条目",
                cur.status
            )));
        }
        if cur.sensitive {
            return Err(MemoryError::BadRequest(
                "敏感原子禁走 correct 快路径——相关更正走会话蒸馏通道".into(),
            ));
        }
        let t = text.trim();
        if t.is_empty() {
            return Err(MemoryError::BadRequest("更正内容不能为空".into()));
        }
        if t.chars().count() > 120 {
            return Err(MemoryError::BadRequest(format!(
                "更正内容超长：最多 120 字，当前 {} 字",
                t.chars().count()
            )));
        }
        let new_id = Uuid::now_v7();
        let emb = self.try_embed(&[t.to_string()]).await;
        let row = repo::correct_atom(
            &self.pool,
            new_id,
            target_id,
            &cur.kind,
            t,
            emb.and_then(|v| v.into_iter().next()),
            &engram_search::tokenize::tsv_text(t),
        )
        .await?
        .ok_or_else(|| {
            MemoryError::BadRequest(
                "目标原子已被并发变更（非 active 或敏感）——重新 search 后再试".into(),
            )
        })?;
        self.audit(
            "correct_atom",
            json!({
                "target": target_id.to_string(),
                "new": row.id.to_string(),
                "old_content": cur.content,
                "new_content": t,
            }),
        )
        .await;
        Ok(row)
    }

    /// 待审复核 confirm：摘 needs_review 标记（AI 代管复核，仅 needs_review=true 可处置）。
    pub async fn confirm_review(&self, id: Uuid) -> Result<AtomDto, MemoryError> {
        let row = repo::review_confirm(&self.pool, id).await?.ok_or_else(|| {
            MemoryError::NotFound(format!(
                "原子 {id} 不存在或不在待审状态——confirm 仅可处置 needs_review=true 的条目"
            ))
        })?;
        self.audit(
            "review_confirm",
            json!({ "atom_id": id.to_string(), "content": row.content }),
        )
        .await;
        Ok(row)
    }

    /// 待审复核 discard：归档（仅 needs_review=true 且 active）；归档触发场景快照收敛。
    pub async fn discard_review(&self, id: Uuid) -> Result<AtomDto, MemoryError> {
        let row = repo::review_discard(&self.pool, id).await?.ok_or_else(|| {
            MemoryError::NotFound(format!(
                "原子 {id} 不存在或不在待审状态——discard 仅可处置 needs_review=true 的条目"
            ))
        })?;
        self.audit(
            "review_discard",
            json!({ "atom_id": id.to_string(), "content": row.content }),
        )
        .await;
        let bucket = chrono::Utc::now().timestamp() / self.debounce_secs;
        // 有意忽略：快照刷新建队是 best-effort（原子已归档，收敛失败由下次快照自愈）
        let _ = self
            .queue
            .enqueue(
                JobTemplate::new("organize_scenarios")
                    .with_idempotency_key(format!("snapshot-refresh-{bucket}"))
                    .with_payload(
                        serde_json::json!({"converge_only": true, "atom_id": id.to_string()}),
                    )
                    .with_due(chrono::Utc::now() + chrono::Duration::seconds(self.debounce_secs)),
            )
            .await;
        Ok(row)
    }

    /// 蒸馏回执：一次会话蒸馏产出了什么（原子 id + 内容预览 + 状态）。
    pub async fn distill_result(&self, session_id: Uuid) -> Result<serde_json::Value, MemoryError> {
        let Some((distill_status, metadata)) =
            repo::session_distill_meta(&self.pool, session_id).await?
        else {
            return Err(MemoryError::NotFound(format!("会话 {session_id} 不存在")));
        };
        let status = if metadata.get("distill").and_then(|v| v.as_str()) == Some("off") {
            "off（该会话标记为不蒸馏）".to_string()
        } else {
            distill_status
        };
        let atoms = repo::atoms_by_session(&self.pool, session_id).await?;
        let preview: Vec<serde_json::Value> = atoms
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "kind": a.kind,
                    "content": a.content,
                    "strength": a.strength,
                    "source_kind": a.source_kind,
                    "status": a.status,
                    "needs_review": a.needs_review,
                })
            })
            .collect();
        Ok(serde_json::json!({
            "session_id": session_id,
            "distill_status": status,
            "atom_count": atoms.len(),
            "atoms": preview,
        }))
    }

    // 选项袋式更新：8 个可选字段一一对应列；struct 化留给下一轮接口收敛
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub async fn update_atom(
        &self,
        id: Uuid,
        content: Option<&str>,
        kind: Option<&str>,
        confidence: Option<f32>,
        status: Option<&str>,
        needs_review: Option<bool>,
        superseded_by: Option<Uuid>,
        occurred_at: Option<DateTime<Utc>>,
        valid_until: Option<DateTime<Utc>>,
        sensitive: Option<bool>,
        // 编辑来源（"admin" / "key:名"）——改写语义时落 atom_revisions + 审计
        actor: &str,
    ) -> Result<AtomDto, MemoryError> {
        let cur = repo::find_atom(&self.pool, id)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("原子 {id} 不存在")))?;

        let new_content = content.unwrap_or(&cur.content).to_string();
        let new_conf = confidence.unwrap_or(cur.confidence);
        let new_status = match status {
            Some("archived") => "archived",
            Some("active") if cur.status == "archived" => "active",
            Some("active") | Some("superseded") | Some("candidate") => {
                return Err(MemoryError::BadRequest(
                    "status 只允许 active/archived 切换；supersede 走矛盾流程".into(),
                ));
            }
            _ => cur.status.as_str(),
        };
        let content_changed = new_content != cur.content;
        let new_kind = kind.unwrap_or(&cur.kind);
        // 编辑能力：改写语义（content/kind/confidence）变化 → 旧值进 atom_revisions + 审计行。
        // AI 走 correction（新原子+superseded_by）不产生 revision；这条是用户轻量修正路。
        let rewrite = content_changed
            || new_kind != cur.kind
            || confidence.is_some_and(|c| (c - cur.confidence).abs() > f32::EPSILON);
        if rewrite {
            repo::insert_atom_revision(
                &self.pool,
                Uuid::now_v7(),
                id,
                &cur.content,
                &cur.kind,
                cur.confidence,
                actor,
            )
            .await?;
            self.audit(
            "edit_atom",
            json!({
                "atom_id": id.to_string(),
                "by": actor,
                "old": {"content": cur.content, "kind": cur.kind, "confidence": cur.confidence},
                "new": {
                    "content": if content_changed { new_content.clone() } else { cur.content.clone() },
                    "kind": new_kind,
                    "confidence": confidence.unwrap_or(cur.confidence),
                },
            }),
        )
        .await;
        }
        let emb = if content_changed {
            self.try_embed(std::slice::from_ref(&new_content)).await
        } else {
            None
        };

        let row = repo::update_atom_full(
            &self.pool,
            id,
            &new_content,
            new_conf,
            new_status,
            needs_review,
            superseded_by,
            occurred_at,
            valid_until,
            sensitive,
            emb.and_then(|v| v.into_iter().next()),
            &engram_search::tokenize::tsv_text(&new_content),
            new_kind,
        )
        .await?;

        // F4 治：归档或标敏感 → 受影响场景快照需要收敛重算（best-effort 异步，
        // 30s 防抖合并批量归档；重算仅活跃非敏感成员、0 活跃则解散——organize 收敛段）
        if new_status == "archived" || sensitive == Some(true) {
            let bucket = chrono::Utc::now().timestamp() / self.debounce_secs;
            self.queue
                .enqueue(
                    JobTemplate::new("organize_scenarios")
                        .with_idempotency_key(format!("snapshot-refresh-{bucket}"))
                        .with_payload(
                            serde_json::json!({"converge_only": true, "atom_id": id.to_string()}),
                        )
                        .with_due(
                            chrono::Utc::now() + chrono::Duration::seconds(self.debounce_secs),
                        ),
                )
                .await
                .ok();
        }
        Ok(row)
    }

    /// 原子改写历史（新→旧）。
    pub async fn atom_revisions(&self, atom_id: Uuid) -> Result<Vec<AtomRevision>, MemoryError> {
        Ok(repo::atom_revisions(&self.pool, atom_id).await?)
    }
}

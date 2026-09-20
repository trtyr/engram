//! `memory` 的实现切片（架构治理 2026-09-20：自 memory.rs 纯搬移，零行为变化）。

use super::*;

impl MemoryService {
    /// 回滚分面到历史版本（以新版本号落地当前内容——历史不可变）。
    /// 编辑分面（version+1 落钉：manually_edited=true，蒸馏产出对该分面落库前被守卫丢弃）。
    /// 编辑能力：用户 Web 编辑与 AI persona_edit（收录哲学线授权的执行器）共用此路；
    /// 钉住 = 蒸馏绕开，解冻 = persona_unpin。
    pub async fn persona_edit(
        &self,
        aspect: &str,
        content: &str,
        actor: &str,
    ) -> Result<PersonaVersion, MemoryError> {
        let content = content.trim();
        if content.is_empty() || content.chars().count() > 4000 {
            return Err(MemoryError::BadRequest("分面内容需 1~4000 字".into()));
        }
        let next_v = repo::persona_max_version(&self.pool, aspect)
            .await?
            .flatten()
            .map(|v| v + 1)
            .unwrap_or(1);
        repo::insert_persona_pinned(&self.pool, Uuid::now_v7(), aspect, content, next_v).await?;
        self.audit(
            "edit_persona",
            json!({
                "aspect": aspect, "by": actor, "action": "edit", "version": next_v,
            }),
        )
        .await;
        self.persona_history(aspect)
            .await
            .map(|mut v| v.swap_remove(0))
    }

    /// 解除钉住：分面回归蒸馏管辖（下次 consolidate/退休可重写）。
    pub async fn persona_unpin(&self, aspect: &str, actor: &str) -> Result<(), MemoryError> {
        repo::persona_unpin(&self.pool, aspect).await?;
        self.audit(
            "edit_persona",
            json!({
                "aspect": aspect, "by": actor, "action": "unpin",
            }),
        )
        .await;
        Ok(())
    }

    /// 实体摘要版本链（圈子强化）：手编档案的历史，最近在前。
    pub async fn entity_revisions(
        &self,
        entity_id: Uuid,
    ) -> Result<Vec<EntityRevision>, MemoryError> {
        Ok(repo::entity_revisions(&self.pool, entity_id).await?)
    }

    /// 实体关系列表（有向类型化；可选按实体过滤 from/to 两端）。
    pub async fn list_relations(
        &self,
        entity_id: Option<Uuid>,
    ) -> Result<Vec<EntityRelationDto>, MemoryError> {
        let rows = match entity_id {
            Some(eid) => repo::entity_relations(&self.pool, eid).await?,
            None => repo::list_relations(&self.pool).await?,
        };
        Ok(rows)
    }

    /// 建关系（有向）：from --rel_type--> to；同向同类型 upsert（weight 累加）。
    pub async fn create_relation(
        &self,
        from: Uuid,
        to: Uuid,
        rel_type: &str,
        source: &str,
    ) -> Result<EntityRelationDto, MemoryError> {
        if from == to {
            return Err(MemoryError::BadRequest("关系两端不能是同一实体".into()));
        }
        if !REL_TYPES.contains(&rel_type) {
            return Err(MemoryError::BadRequest(format!(
                "rel_type 只允许 {}",
                REL_TYPES.join("/")
            )));
        }
        self.entity_row(from).await?;
        self.entity_row(to).await?;
        let row =
            repo::insert_relation(&self.pool, Uuid::now_v7(), from, to, rel_type, source).await?;
        Ok(row)
    }

    /// 删关系。
    pub async fn delete_relation(&self, id: Uuid) -> Result<(), MemoryError> {
        let n = repo::delete_relation(&self.pool, id).await?;
        if n == 0 {
            return Err(MemoryError::NotFound(format!("关系 {id} 不存在")));
        }
        Ok(())
    }

    /// 重新钉住（用户解锁后想再钉：为当前内容写一条钉住版本）。
    pub async fn persona_repin(
        &self,
        aspect: &str,
        actor: &str,
    ) -> Result<PersonaVersion, MemoryError> {
        let (content, v) = repo::persona_latest(&self.pool, aspect)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("分面 {aspect} 不存在")))?;
        repo::insert_persona_pinned(&self.pool, Uuid::now_v7(), aspect, &content, v + 1).await?;
        self.audit(
            "edit_persona",
            json!({
                "aspect": aspect, "by": actor, "action": "repin", "version": v + 1,
            }),
        )
        .await;
        self.persona_history(aspect)
            .await
            .map(|mut h| h.swap_remove(0))
    }

    pub async fn persona_rollback(
        &self,
        aspect: &str,
        to_version: i32,
        actor: &str,
    ) -> Result<PersonaVersion, MemoryError> {
        let target = repo::persona_version(&self.pool, aspect, to_version)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("版本 {aspect}#{to_version} 不存在")))?;

        let next_v = repo::persona_max_version(&self.pool, aspect)
            .await?
            .flatten()
            .map(|v| v + 1)
            .unwrap_or(1);

        repo::insert_persona_rollback(
            &self.pool,
            Uuid::now_v7(),
            aspect,
            &target.content,
            next_v,
            &json!({"rollback_to": to_version}),
        )
        .await?;
        // 回滚 = 人工钉住（蒸馏绕开，直到解锁）
        self.audit(
            "edit_persona",
            json!({
                "aspect": aspect, "by": actor, "action": "rollback", "to_version": to_version,
            }),
        )
        .await;
        self.persona_history(aspect)
            .await
            .map(|mut v| v.swap_remove(0))
    }

    // ---------- 实体（记忆星系） ----------

    /// 活体实体列表（按记忆密度降序）。
    pub async fn list_entities(&self, kind: Option<&str>) -> Result<Vec<EntityDto>, MemoryError> {
        Ok(repo::list_entities(&self.pool, kind).await?)
    }

    pub(super) async fn entity_row(&self, id: Uuid) -> Result<EntityDto, MemoryError> {
        repo::entity_row(&self.pool, id)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("实体 {id} 不存在")))
    }

    /// 实体详情：画像摘要 + 相关原子时间线 + 相关场景。
    pub async fn get_entity(&self, id: Uuid) -> Result<EntityDetail, MemoryError> {
        let entity = self.entity_row(id).await?;
        let atoms = repo::entity_atoms(&self.pool, id).await?;
        let scenarios = repo::entity_scenarios(&self.pool, id).await?;
        let neighbors = repo::entity_neighbors(&self.pool, id).await?;
        let relations = repo::entity_relations(&self.pool, id).await?;
        Ok(EntityDetail {
            entity,
            atoms,
            scenarios,
            neighbors,
            relations,
        })
    }

    /// 手动建实体（蒸馏自动抽取之外的人工入口；同名同类活体只许一个）。
    /// 不可失败（架构治理 task-5 分类 A：不可失败，保留并注明理由）。
    #[allow(clippy::expect_used)]
    pub async fn create_entity(
        &self,
        name: &str,
        kind: &str,
        summary: &str,
    ) -> Result<EntityDto, MemoryError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 60 {
            return Err(MemoryError::BadRequest("实体名需 1~60 字".into()));
        }
        if !ENTITY_KINDS.contains(&kind) {
            return Err(MemoryError::BadRequest(format!(
                "kind 只允许 {}",
                ENTITY_KINDS.join("/")
            )));
        }
        let dup = repo::entity_id_by_name_kind(&self.pool, name, kind).await?;
        if dup.is_some() {
            return Err(MemoryError::BadRequest(format!(
                "同名同类实体已存在：{name}"
            )));
        }
        repo::insert_entity(&self.pool, Uuid::now_v7(), name, kind, summary).await?;
        let id = repo::entity_id_by_name_kind(&self.pool, name, kind)
            .await?
            .expect("刚插入的实体必能查到");
        self.entity_row(id).await
    }

    pub async fn update_entity(
        &self,
        id: Uuid,
        name: Option<&str>,
        summary: Option<&str>,
        actor: &str,
    ) -> Result<EntityDto, MemoryError> {
        let mut changed = false;
        if let Some(n) = name {
            let n = n.trim();
            if n.is_empty() || n.chars().count() > 60 {
                return Err(MemoryError::BadRequest("实体名需 1~60 字".into()));
            }
            repo::update_entity_name(&self.pool, id, n).await?;
            changed = true;
        }
        if let Some(s) = summary {
            // 版本链：旧摘要进 entity_revisions（轻量历史，复用原子 revisions 模式）
            if let Some(old_summary) = repo::entity_summary(&self.pool, id).await?
                && old_summary != s
            {
                repo::insert_entity_revision(&self.pool, Uuid::now_v7(), id, &old_summary, actor)
                    .await?;
            }
            repo::update_entity_summary(&self.pool, id, s).await?;
            changed = true;
        }
        if changed {
            // 用户手编实体档案 → 钉住（consolidate 档案重生成绕开）；审计
            repo::pin_entity(&self.pool, id).await?;
            self.audit(
                "edit_entity",
                json!({
                    "entity_id": id.to_string(), "by": actor,
                }),
            )
            .await;
        }
        self.entity_row(id).await
    }

    pub async fn delete_entity(&self, id: Uuid) -> Result<(), MemoryError> {
        let n = repo::delete_entity(&self.pool, id).await?;
        if n == 0 {
            return Err(MemoryError::NotFound(format!("实体 {id} 不存在")));
        }
        // 审计（2026-09-01 补：entity 删除曾无审计行，排查全靠猜）——best-effort
        self.audit(
            "delete_entity",
            serde_json::json!({ "entity_id": id, "tombstones": n - 1 }),
        )
        .await;
        Ok(())
    }

    /// 实体级遗忘（「把小王忘了」）：级联归档挂链 active 原子 → 摘链 → 删实体+墓碑。
    /// archived/superseded 等非 active 原子不动（本来就是历史）；待审候选一并归档。
    pub async fn forget_entity(&self, id: Uuid) -> Result<usize, MemoryError> {
        let cur = repo::live_entity_id(&self.pool, id).await?;
        if cur.is_none() {
            return Err(MemoryError::NotFound(format!("实体 {id} 不存在或已合并")));
        }
        let n = repo::archive_atoms_by_entity(&self.pool, id).await? as usize;
        repo::detach_entity_links(&self.pool, id).await?;
        self.delete_entity(id).await?;
        Ok(n)
    }

    /// 挂原子到实体（幂等）。
    pub async fn attach_atom(&self, entity_id: Uuid, atom_id: Uuid) -> Result<(), MemoryError> {
        self.entity_row(entity_id).await?;
        let n = repo::count_atom(&self.pool, atom_id).await?;
        if n == 0 {
            return Err(MemoryError::NotFound(format!("原子 {atom_id} 不存在")));
        }
        repo::attach_atom_link(&self.pool, atom_id, entity_id).await?;
        repo::touch_entity(&self.pool, entity_id).await?;
        Ok(())
    }

    pub async fn detach_atom(&self, entity_id: Uuid, atom_id: Uuid) -> Result<(), MemoryError> {
        let n = repo::detach_atom_link(&self.pool, atom_id, entity_id).await?;
        if n == 0 {
            return Err(MemoryError::NotFound(format!(
                "原子 {atom_id} 未关联到实体 {entity_id}"
            )));
        }
        Ok(())
    }

    /// 合并实体：from 的原子关联全部改挂 into，from 置 merged_into 让出唯一名。
    pub async fn merge_entities(&self, from: Uuid, into: Uuid) -> Result<i64, MemoryError> {
        if from == into {
            return Err(MemoryError::BadRequest("不能合并到自身".into()));
        }
        self.entity_row(from).await?;
        self.entity_row(into).await?;
        let moved = repo::merge_entities_tx(&self.pool, from, into).await?;
        Ok(moved)
    }

    /// 星系图：节点（活体实体 + 密度）+ 共现边。
    pub async fn entity_graph(&self) -> Result<EntityGraph, MemoryError> {
        let nodes = self.list_entities(None).await?;
        let edges = repo::cooccurrence_edges(&self.pool).await?;
        let relations = self.list_relations(None).await?;
        Ok(EntityGraph {
            nodes,
            edges,
            relations,
        })
    }
}

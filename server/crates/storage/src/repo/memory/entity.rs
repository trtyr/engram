//! `memory` 的实现切片（架构治理 2026-09-21：自 memory.rs 纯搬移，零行为变化）。

use super::*;

/// 实体摘要版本链（圈子强化）：手编档案的历史，最近在前。
pub async fn entity_revisions(pool: &PgPool, entity_id: Uuid) -> StoreResult<Vec<EntityRevision>> {
    Ok(sqlx::query_as::<_, EntityRevision>(
        "SELECT * FROM entity_revisions WHERE entity_id = $1 ORDER BY created_at DESC",
    )
    .bind(entity_id)
    .fetch_all(pool)
    .await?)
}

/// 解绑原子与实体。返回生效行数（0 = 未关联）。
pub async fn detach_atom_link(pool: &PgPool, atom_id: Uuid, entity_id: Uuid) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM atom_entities WHERE atom_id = $1 AND entity_id = $2")
        .bind(atom_id)
        .bind(entity_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

pub async fn entity_row(pool: &PgPool, id: Uuid) -> StoreResult<Option<EntityDto>> {
    sqlx::query_as::<_, EntityDto>(
        "SELECT e.id, e.name, e.kind, e.summary, count(ae.atom_id)::bigint AS atom_count, bool_or(e.manually_edited) AS manually_edited, e.updated_at \
         FROM entities e LEFT JOIN atom_entities ae ON ae.entity_id = e.id \
         WHERE e.id = $1 AND e.merged_into IS NULL \
         GROUP BY e.id, e.name, e.kind, e.summary, e.manually_edited, e.updated_at",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

/// 活体实体列表（按记忆密度降序）。
pub async fn list_entities(pool: &PgPool, kind: Option<&str>) -> StoreResult<Vec<EntityDto>> {
    Ok(sqlx::query_as::<_, EntityDto>(
        "SELECT e.id, e.name, e.kind, e.summary, count(ae.atom_id)::bigint AS atom_count, e.manually_edited, e.updated_at \
         FROM entities e LEFT JOIN atom_entities ae ON ae.entity_id = e.id \
         WHERE e.merged_into IS NULL AND e.archived_at IS NULL AND ($1::text IS NULL OR e.kind = $1) \
         GROUP BY e.id, e.name, e.kind, e.summary, e.manually_edited, e.updated_at \
         ORDER BY atom_count DESC, e.updated_at DESC",
    )
    .bind(kind)
    .fetch_all(pool)
    .await?)
}

/// 实体详情：相关原子时间线。
pub async fn entity_atoms(pool: &PgPool, entity_id: Uuid) -> StoreResult<Vec<AtomDto>> {
    let rows = sqlx::query_as::<_, AtomDto>(
        "SELECT a.* FROM atoms a JOIN atom_entities ae ON ae.atom_id = a.id \
         WHERE ae.entity_id = $1 ORDER BY a.created_at DESC LIMIT 200",
    )
    .bind(entity_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 实体详情：相关场景。
pub async fn entity_scenarios(pool: &PgPool, entity_id: Uuid) -> StoreResult<Vec<ScenarioDto>> {
    let rows = sqlx::query_as::<_, ScenarioDto>(
        "SELECT DISTINCT ON (s.id) s.* FROM scenarios s \
         JOIN atoms a ON a.scenario_id = s.id \
         JOIN atom_entities ae ON ae.atom_id = a.id \
         WHERE ae.entity_id = $1 LIMIT 50",
    )
    .bind(entity_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 实体详情：共现邻居（共享原子的其他实体，按共现次数降序）。
pub async fn entity_neighbors(pool: &PgPool, entity_id: Uuid) -> StoreResult<Vec<EntityDto>> {
    let rows = sqlx::query_as::<_, EntityDto>(
        "SELECT e.id, e.name, e.kind, e.summary, \
                (SELECT count(*) FROM atom_entities x WHERE x.entity_id = e.id)::bigint AS atom_count, \
                bool_or(e.manually_edited) AS manually_edited, e.updated_at \
         FROM atom_entities ae \
         JOIN entities e ON e.id = ae.entity_id \
         WHERE ae.atom_id IN (SELECT atom_id FROM atom_entities WHERE entity_id = $1) \
           AND ae.entity_id != $1 AND e.merged_into IS NULL \
         GROUP BY e.id, e.name, e.kind, e.summary, e.manually_edited, e.updated_at \
         ORDER BY count(*) DESC, e.updated_at DESC LIMIT 20",
    )
    .bind(entity_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 本实体作为 from 或 to 的有向关系。
pub async fn entity_relations(
    pool: &PgPool,
    entity_id: Uuid,
) -> StoreResult<Vec<EntityRelationDto>> {
    let rows = sqlx::query_as::<_, EntityRelationDto>(
        "SELECT * FROM entity_relations WHERE from_id = $1 OR to_id = $1 ORDER BY created_at DESC",
    )
    .bind(entity_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_relations(pool: &PgPool) -> StoreResult<Vec<EntityRelationDto>> {
    let rows = sqlx::query_as::<_, EntityRelationDto>(
        "SELECT * FROM entity_relations ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 同名同类活体实体 id（唯一性预检）。
pub async fn entity_id_by_name_kind(
    pool: &PgPool,
    name: &str,
    kind: &str,
) -> StoreResult<Option<Uuid>> {
    let id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM entities WHERE lower(btrim(name)) = lower(btrim($1)) AND kind = $2 AND merged_into IS NULL AND archived_at IS NULL",
    )
    .bind(name)
    .bind(kind)
    .fetch_optional(pool)
    .await?;
    Ok(id)
}

pub async fn insert_entity(
    pool: &PgPool,
    id: Uuid,
    name: &str,
    kind: &str,
    summary: &str,
) -> StoreResult<()> {
    sqlx::query("INSERT INTO entities (id, name, kind, summary) VALUES ($1, $2, $3, $4)")
        .bind(id)
        .bind(name)
        .bind(kind)
        .bind(summary)
        .execute(pool)
        .await?;
    Ok(())
}

/// 当前摘要（版本链写旧值用）。
pub async fn entity_summary(pool: &PgPool, entity_id: Uuid) -> StoreResult<Option<String>> {
    let old: Option<String> =
        sqlx::query_scalar("SELECT summary FROM entities WHERE id = $1 AND merged_into IS NULL")
            .bind(entity_id)
            .fetch_optional(pool)
            .await?;
    Ok(old)
}

pub async fn update_entity_name(pool: &PgPool, entity_id: Uuid, name: &str) -> StoreResult<()> {
    sqlx::query(
        "UPDATE entities SET name = $2, updated_at = now() WHERE id = $1 AND merged_into IS NULL",
    )
    .bind(entity_id)
    .bind(name)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn insert_entity_revision(
    pool: &PgPool,
    id: Uuid,
    entity_id: Uuid,
    old_summary: &str,
    edited_by: &str,
) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO entity_revisions (id, entity_id, old_summary, edited_by) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(entity_id)
    .bind(old_summary)
    .bind(edited_by)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_entity_summary(
    pool: &PgPool,
    entity_id: Uuid,
    summary: &str,
) -> StoreResult<()> {
    sqlx::query(
        "UPDATE entities SET summary = $2, manually_edited = true, updated_at = now() \
                 WHERE id = $1 AND merged_into IS NULL",
    )
    .bind(entity_id)
    .bind(summary)
    .execute(pool)
    .await?;
    Ok(())
}

/// 用户手编实体档案 → 钉住（consolidate 档案重生成绕开）。
pub async fn pin_entity(pool: &PgPool, entity_id: Uuid) -> StoreResult<()> {
    sqlx::query("UPDATE entities SET manually_edited = true WHERE id = $1 AND merged_into IS NULL")
        .bind(entity_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 只有活体可删；删活体时连带清掉并入它的墓碑（merged_into 指向它）——
/// 否则 FK(entities_merged_into_fkey) 拒绝删除。墓碑是合并的残迹，赢家没了它也不复活。
pub async fn delete_entity(pool: &PgPool, id: Uuid) -> StoreResult<u64> {
    let res = sqlx::query(
        "DELETE FROM entities WHERE ($1 IN (SELECT id FROM entities WHERE id = $1 AND merged_into IS NULL)) \
         AND (id = $1 OR merged_into = $1)",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 实体级遗忘的活体判定。
pub async fn live_entity_id(pool: &PgPool, id: Uuid) -> StoreResult<Option<Uuid>> {
    let cur: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM entities WHERE id = $1 AND merged_into IS NULL")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    Ok(cur)
}

/// 摘链（atom_entities 随实体删除本会级联，这里显式删保持语义清晰）。
pub async fn detach_entity_links(pool: &PgPool, entity_id: Uuid) -> StoreResult<()> {
    sqlx::query("DELETE FROM atom_entities WHERE entity_id = $1")
        .bind(entity_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 挂原子到实体（幂等）。
pub async fn attach_atom_link(pool: &PgPool, atom_id: Uuid, entity_id: Uuid) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO atom_entities (atom_id, entity_id) VALUES ($1, $2) \
         ON CONFLICT DO NOTHING",
    )
    .bind(atom_id)
    .bind(entity_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn touch_entity(pool: &PgPool, entity_id: Uuid) -> StoreResult<()> {
    sqlx::query("UPDATE entities SET updated_at = now() WHERE id = $1")
        .bind(entity_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 合并实体：from 的原子关联全部改挂 into，from 置 merged_into 让出唯一名。
/// 返回迁移的原子数。
pub async fn merge_entities_tx(pool: &PgPool, from: Uuid, into: Uuid) -> StoreResult<i64> {
    let mut tx = pool.begin().await?;
    let moved: Vec<(Uuid,)> = sqlx::query_as(
        "INSERT INTO atom_entities (atom_id, entity_id) \
         SELECT atom_id, $2 FROM atom_entities WHERE entity_id = $1 \
         ON CONFLICT DO NOTHING RETURNING atom_id",
    )
    .bind(from)
    .bind(into)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM atom_entities WHERE entity_id = $1")
        .bind(from)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE entities SET merged_into = $2, archived_at = now(), updated_at = now() WHERE id = $1",
    )
    .bind(from)
    .bind(into)
    .execute(&mut *tx)
    .await?;
    // EN-242 审计补强：摘要并入——主档 summary 为空时收副档的（参照 wiki merge 内容并入语义）
    let loser_summary: Option<String> = sqlx::query_scalar(
        "SELECT summary FROM entities WHERE id = $1 AND summary IS NOT NULL AND summary <> ''",
    )
    .bind(from)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(ls) = loser_summary {
        sqlx::query(
            "UPDATE entities SET summary = $2, updated_at = now() \
             WHERE id = $1 AND (summary IS NULL OR summary = '')",
        )
        .bind(into)
        .bind(ls)
        .execute(&mut *tx)
        .await?;
    }
    // EN-242 审计补强：关系边迁移——冲突安全（uniq_entity_relation 下先删语义被覆盖的边再改指，
    // 避免唯一冲突回滚整个合并事务），事后同向同类去重 + 自环清理
    sqlx::query(
        "DELETE FROM entity_relations r WHERE r.from_id = $1 AND EXISTS ( \
           SELECT 1 FROM entity_relations w WHERE w.from_id = $2 AND w.to_id = r.to_id AND w.rel_type = r.rel_type)",
    )
    .bind(from)
    .bind(into)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE entity_relations SET from_id = $2, updated_at = now() WHERE from_id = $1")
        .bind(from)
        .bind(into)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "DELETE FROM entity_relations r WHERE r.to_id = $1 AND EXISTS ( \
           SELECT 1 FROM entity_relations w WHERE w.to_id = $2 AND w.from_id = r.from_id AND w.rel_type = r.rel_type)",
    )
    .bind(from)
    .bind(into)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE entity_relations SET to_id = $2, updated_at = now() WHERE to_id = $1")
        .bind(from)
        .bind(into)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "DELETE FROM entity_relations a USING entity_relations b \
         WHERE a.id > b.id AND a.from_id = b.from_id AND a.to_id = b.to_id AND a.rel_type = b.rel_type",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM entity_relations WHERE from_id = to_id AND (from_id = $1 OR to_id = $1)",
    )
    .bind(into)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE entities SET updated_at = now() WHERE id = $1")
        .bind(into)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(moved.len() as i64)
}

/// 星系图共现边：同一原子同时关联的两个实体（weight = 共同原子数）。
pub async fn cooccurrence_edges(pool: &PgPool) -> StoreResult<Vec<GraphEdge>> {
    let rows = sqlx::query_as::<_, GraphEdge>(
        "SELECT ae1.entity_id AS a, ae2.entity_id AS b, count(*)::bigint AS weight \
         FROM atom_entities ae1 \
         JOIN atom_entities ae2 ON ae1.atom_id = ae2.atom_id AND ae1.entity_id < ae2.entity_id \
         GROUP BY ae1.entity_id, ae2.entity_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// context_pack 实体透镜：按命中序取实体（保持调用方给定的 id 集合）。
pub async fn entities_by_ids(pool: &PgPool, ids: &[Uuid]) -> StoreResult<Vec<EntityDto>> {
    let rows = sqlx::query_as(
        "SELECT id, name, kind, summary, \
         (SELECT count(*) FROM atom_entities ae WHERE ae.entity_id = entities.id) AS atom_count, \
         manually_edited, updated_at FROM entities WHERE id = ANY($1) AND merged_into IS NULL \
           AND archived_at IS NULL",
    )
    .bind(ids)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 全量导出（P4）：活体实体（密度降序）。
pub async fn entities_for_export(pool: &PgPool) -> StoreResult<Vec<EntityDto>> {
    let rows = sqlx::query_as(
        "SELECT id, name, kind, summary, \
         (SELECT count(*) FROM atom_entities ae WHERE ae.entity_id = entities.id) AS atom_count, \
         manually_edited, updated_at FROM entities WHERE merged_into IS NULL ORDER BY updated_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 建关系（有向）：同向同类型 upsert（weight 累加）。
pub async fn insert_relation(
    pool: &PgPool,
    id: Uuid,
    from: Uuid,
    to: Uuid,
    rel_type: &str,
    source: &str,
) -> StoreResult<EntityRelationDto> {
    let row = sqlx::query_as::<_, EntityRelationDto>(
        "INSERT INTO entity_relations (id, from_id, to_id, rel_type, weight, source) \
         VALUES ($1, $2, $3, $4, 1, $5) \
         ON CONFLICT (from_id, to_id, rel_type) DO UPDATE SET weight = entity_relations.weight + 1, updated_at = now() \
         RETURNING *",
    )
    .bind(id)
    .bind(from)
    .bind(to)
    .bind(rel_type)
    .bind(source)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn delete_relation(pool: &PgPool, id: Uuid) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM entity_relations WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// D16：实体层级联——把「有挂链原子但全部非 active（被 void 会话遗忘）」的实体
/// 打归档标记（archived_at），从列表/检索/图谱隐身；实体本身保留可审计。
/// 手动创建、从未挂原子的实体不受影响。
pub async fn archive_orphan_entities(pool: &PgPool) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE entities e SET archived_at = now() \
         WHERE e.archived_at IS NULL AND e.merged_into IS NULL \
           AND EXISTS (SELECT 1 FROM atom_entities ae WHERE ae.entity_id = e.id) \
           AND NOT EXISTS ( \
             SELECT 1 FROM atom_entities ae JOIN atoms a ON a.id = ae.atom_id \
             WHERE ae.entity_id = e.id AND a.status = 'active')",
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 蒸馏挂链（或手动挂原子）时复活归档实体：重新有活跃证据即回到可见层。
pub async fn revive_entity(pool: &PgPool, entity_id: Uuid) -> StoreResult<()> {
    sqlx::query("UPDATE entities SET archived_at = NULL WHERE id = $1 AND archived_at IS NOT NULL")
        .bind(entity_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// EN-242：同名实体检测——lower(btrim(name)) 完全相同的实体行（大小写/首尾空白异形）。
pub async fn duplicate_name_rows(pool: &PgPool) -> StoreResult<Vec<(Uuid, String)>> {
    Ok(sqlx::query_as(
        "SELECT id, lower(btrim(name)) FROM entities \
         WHERE lower(btrim(name)) IN ( \
           SELECT lower(btrim(name)) FROM entities GROUP BY 1 HAVING count(*) > 1) \
         ORDER BY 2, 1",
    )
    .fetch_all(pool)
    .await?)
}

//! 记忆域仓储：raw_sessions / atoms / scenarios / persona_aspects / entities /
//! entity_relations / atom_entities / atom_revisions / entity_revisions 读写。
//!
//! 事务边界：purge_agent / purge_deep / merge_entities 整体在事务内封装。

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::PgPool;
use crate::error::StoreResult;
use crate::models::memory::{
    AtomDto, AtomRevision, EntityDto, EntityRelationDto, EntityRevision, GraphEdge, PersonaVersion,
    ScenarioDto, SessionDto, TimelineEvent,
};

// ---------- L0 会话 ----------

pub async fn insert_session(
    pool: &PgPool,
    id: Uuid,
    agent: &str,
    turns: &Value,
    sensitive: bool,
    metadata: &Value,
) -> StoreResult<SessionDto> {
    let row = sqlx::query_as::<_, SessionDto>(
        "INSERT INTO raw_sessions (id, agent, content, sensitive, metadata) VALUES ($1, $2, $3, $4, $5) RETURNING *",
    )
    .bind(id)
    .bind(agent)
    .bind(sqlx::types::Json(turns))
    .bind(sensitive)
    .bind(sqlx::types::Json(metadata))
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn insert_session_import(
    pool: &PgPool,
    id: Uuid,
    agent: &str,
    turns: &Value,
) -> StoreResult<SessionDto> {
    let row = sqlx::query_as::<_, SessionDto>(
        "INSERT INTO raw_sessions (id, agent, content, metadata) VALUES ($1, $2, $3, $4) RETURNING *",
    )
    .bind(id)
    .bind(agent)
    .bind(sqlx::types::Json(turns))
    .bind(serde_json::json!({"source": "import"}))
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn find_session(pool: &PgPool, id: Uuid) -> StoreResult<Option<SessionDto>> {
    sqlx::query_as::<_, SessionDto>("SELECT * FROM raw_sessions WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn list_sessions(
    pool: &PgPool,
    agent: Option<&str>,
    cursor: Option<DateTime<Utc>>,
    limit: i64,
) -> StoreResult<Vec<SessionDto>> {
    Ok(sqlx::query_as::<_, SessionDto>(
        "SELECT * FROM raw_sessions \
         WHERE ($1::text IS NULL OR agent = $1) AND ($2::timestamptz IS NULL OR created_at < $2) \
         ORDER BY created_at DESC LIMIT $3",
    )
    .bind(agent)
    .bind(cursor)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

/// P12：原子 jsonb 数组拼接——单语句在行级天然串行，并发 append 不丢更新
/// （分立的读-改-写在池连接上无法持锁，后写会覆盖前写的合并结果）。
pub async fn append_session_update(
    pool: &PgPool,
    id: Uuid,
    turns: &Value,
    agent: Option<&str>,
) -> StoreResult<SessionDto> {
    let row = sqlx::query_as::<_, SessionDto>(
        "UPDATE raw_sessions SET content = content || $2, agent = COALESCE($3, agent) WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(sqlx::types::Json(turns))
    .bind(agent)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn delete_session(pool: &PgPool, id: Uuid) -> StoreResult<u64> {
    let res = sqlx::query("DELETE FROM raw_sessions WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

pub async fn session_distill_status(pool: &PgPool, id: Uuid) -> StoreResult<Option<String>> {
    let status: Option<String> =
        sqlx::query_scalar("SELECT distill_status FROM raw_sessions WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    Ok(status)
}

pub async fn void_session_update(pool: &PgPool, id: Uuid) -> StoreResult<Option<SessionDto>> {
    let row = sqlx::query_as::<_, SessionDto>(
        "UPDATE raw_sessions SET distill_status = 'void' \
         WHERE id = $1 AND distill_status IN ('pending','done') RETURNING *",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 节律积压：pending（排除 off）会话数 + 最老一条的创建时间。
pub async fn session_backlog(pool: &PgPool) -> StoreResult<(i64, Option<DateTime<Utc>>)> {
    let pending: (i64, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT count(*), min(created_at) FROM raw_sessions WHERE distill_status = 'pending' \
         AND COALESCE(metadata->>'distill','') <> 'off'",
    )
    .fetch_one(pool)
    .await?;
    Ok(pending)
}

/// 节律心跳：复用 jobs 审计行（kind=rhythm_heartbeat）取最近一条。
pub async fn rhythm_last_heartbeat(pool: &PgPool) -> StoreResult<Option<(DateTime<Utc>, String)>> {
    let heartbeat: Option<(DateTime<Utc>, String)> = sqlx::query_as(
        "SELECT created_at, payload->>'by' AS by FROM jobs \
         WHERE kind = 'rhythm_heartbeat' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(heartbeat)
}

/// P11/SEC-E 按 agent 清场：先归档该 agent 会话产出的 active 原子再删会话
/// （顺序敏感：删会话后 JOIN 不可判归属）。返回 (erased_sessions, archived_atoms)。
pub async fn purge_agent_tx(pool: &PgPool, agent: &str) -> StoreResult<(i64, i64)> {
    let mut tx = pool.begin().await?;
    let archived = sqlx::query(
        "UPDATE atoms SET status = 'archived', updated_at = now() \
         WHERE status = 'active' AND EXISTS ( \
            SELECT 1 FROM jsonb_array_elements(atoms.source_refs) e \
            JOIN raw_sessions s ON s.id::text = e->>'session_id' \
            WHERE s.agent = $1)",
    )
    .bind(agent)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let erased = sqlx::query("DELETE FROM raw_sessions WHERE agent = $1")
        .bind(agent)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    Ok((erased as i64, archived as i64))
}

/// F1/F2 deep purge：记忆域四层 + 实体链一键清空，单事务，返回五计数。
/// TRUNCATE CASCADE 一发解 FK——调用层负责 erase scope + confirm 双因子。
pub async fn purge_deep(pool: &PgPool) -> StoreResult<Value> {
    let mut tx = pool.begin().await?;
    let counts: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT count(*) FROM raw_sessions), \
            (SELECT count(*) FROM atoms), \
            (SELECT count(*) FROM entities WHERE merged_into IS NULL), \
            (SELECT count(*) FROM scenarios), \
            (SELECT count(*) FROM persona_aspects)",
    )
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "TRUNCATE atom_entities, entities, persona_aspects, scenarios, atoms, raw_sessions CASCADE",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(serde_json::json!({
        "sessions": counts.0, "atoms": counts.1, "entities": counts.2,
        "scenarios": counts.3, "persona": counts.4,
    }))
}

/// P2 进程崩溃自愈：上次运行中被认领（processing）的会话退回 pending。
/// 返回受影响行数。
pub async fn reset_processing_sessions(pool: &PgPool) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE raw_sessions SET distill_status = 'pending' WHERE distill_status = 'processing'",
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 全量导出（P4 数据主权）：全部会话（含 sensitive——export 显式决策面）。
pub async fn list_all_sessions(pool: &PgPool) -> StoreResult<Vec<SessionDto>> {
    let rows = sqlx::query_as("SELECT * FROM raw_sessions ORDER BY created_at")
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

// ---------- L1 原子 ----------

pub async fn find_atom(pool: &PgPool, id: Uuid) -> StoreResult<Option<AtomDto>> {
    sqlx::query_as::<_, AtomDto>("SELECT * FROM atoms WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn list_atoms(
    pool: &PgPool,
    kind: Option<&str>,
    status: Option<&str>,
    needs_review: Option<bool>,
    cursor: Option<DateTime<Utc>>,
    limit: i64,
) -> StoreResult<Vec<AtomDto>> {
    Ok(sqlx::query_as::<_, AtomDto>(
        "SELECT * FROM atoms \
         WHERE ($1::text IS NULL OR kind = $1) AND ($2::text IS NULL OR status = $2) \
           AND ($3::bool IS NULL OR needs_review = $3) \
           AND ($4::timestamptz IS NULL OR created_at < $4) \
         ORDER BY created_at DESC LIMIT $5",
    )
    .bind(kind)
    .bind(status)
    .bind(needs_review)
    .bind(cursor)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

/// A4 幂等护栏：同 kind + 同内容（trim 后）的 active 原子已存在则返回它。
pub async fn find_active_atom(
    pool: &PgPool,
    kind: &str,
    content: &str,
) -> StoreResult<Option<AtomDto>> {
    let row = sqlx::query_as::<_, AtomDto>(
        "SELECT * FROM atoms WHERE kind = $1 AND content = $2 AND status = 'active' LIMIT 1",
    )
    .bind(kind)
    .bind(content)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_atom(
    pool: &PgPool,
    id: Uuid,
    kind: &str,
    text: &str,
    confidence: f32,
    needs_review: bool,
    sensitive: bool,
    occurred_at: Option<DateTime<Utc>>,
    valid_until: Option<DateTime<Utc>>,
    embedding: Option<Vec<f32>>,
    tsv: &str,
) -> StoreResult<AtomDto> {
    let row = sqlx::query_as::<_, AtomDto>(
        "INSERT INTO atoms (id, kind, content, confidence, status, needs_review, sensitive, occurred_at, valid_until, source_refs, embedding, tsv) \
         VALUES ($1, $2, $3, $4, 'active', $5, $6, $7, $8, '[]'::jsonb, $9, to_tsvector('simple', $10)) RETURNING *",
    )
    .bind(id)
    .bind(kind)
    .bind(text)
    .bind(confidence)
    .bind(needs_review)
    .bind(sensitive)
    .bind(occurred_at)
    .bind(valid_until)
    .bind(embedding.map(pgvector::Vector::from))
    .bind(tsv)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// 原子改写历史（新→旧）。
pub async fn atom_revisions(pool: &PgPool, atom_id: Uuid) -> StoreResult<Vec<AtomRevision>> {
    Ok(sqlx::query_as::<_, AtomRevision>(
        "SELECT * FROM atom_revisions WHERE atom_id = $1 ORDER BY created_at DESC",
    )
    .bind(atom_id)
    .fetch_all(pool)
    .await?)
}

/// 实体摘要版本链（圈子强化）：手编档案的历史，最近在前。
pub async fn entity_revisions(pool: &PgPool, entity_id: Uuid) -> StoreResult<Vec<EntityRevision>> {
    Ok(sqlx::query_as::<_, EntityRevision>(
        "SELECT * FROM entity_revisions WHERE entity_id = $1 ORDER BY created_at DESC",
    )
    .bind(entity_id)
    .fetch_all(pool)
    .await?)
}

/// 指定历史版本（回滚目标）。
pub async fn persona_version(
    pool: &PgPool,
    aspect: &str,
    version: i32,
) -> StoreResult<Option<PersonaVersion>> {
    sqlx::query_as::<_, PersonaVersion>(
        "SELECT * FROM persona_aspects WHERE aspect = $1 AND version = $2",
    )
    .bind(aspect)
    .bind(version)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
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

pub async fn insert_atom_revision(
    pool: &PgPool,
    id: Uuid,
    atom_id: Uuid,
    old_content: &str,
    old_kind: &str,
    old_confidence: f32,
    edited_by: &str,
) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO atom_revisions (id, atom_id, old_content, old_kind, old_confidence, edited_by) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(atom_id)
    .bind(old_content)
    .bind(old_kind)
    .bind(old_confidence)
    .bind(edited_by)
    .execute(pool)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn update_atom_full(
    pool: &PgPool,
    id: Uuid,
    content: &str,
    confidence: f32,
    status: &str,
    needs_review: Option<bool>,
    superseded_by: Option<Uuid>,
    occurred_at: Option<DateTime<Utc>>,
    valid_until: Option<DateTime<Utc>>,
    sensitive: Option<bool>,
    embedding: Option<Vec<f32>>,
    tsv: &str,
    kind: &str,
) -> StoreResult<AtomDto> {
    let row = sqlx::query_as::<_, AtomDto>(
        "UPDATE atoms SET content = $2, confidence = $3, status = $4, kind = $12, needs_review = COALESCE($5, needs_review), \
             superseded_by = COALESCE($6, superseded_by), occurred_at = COALESCE($7, occurred_at), \
             valid_until = COALESCE($8, valid_until), sensitive = COALESCE($9, sensitive), \
             embedding = COALESCE($10, embedding), tsv = to_tsvector('simple', $11), updated_at = now() \
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(content)
    .bind(confidence)
    .bind(status)
    .bind(needs_review)
    .bind(superseded_by)
    .bind(occurred_at)
    .bind(valid_until)
    .bind(sensitive)
    .bind(embedding.map(pgvector::Vector::from))
    .bind(tsv)
    .bind(kind)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// 原子存在性（挂实体前校验）。
pub async fn count_atom(pool: &PgPool, atom_id: Uuid) -> StoreResult<i64> {
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM atoms WHERE id = $1")
        .bind(atom_id)
        .fetch_one(pool)
        .await?;
    Ok(n)
}

/// 归档挂在实体上的全部 active 原子。返回归档数。
pub async fn archive_atoms_by_entity(pool: &PgPool, entity_id: Uuid) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE atoms SET status = 'archived', updated_at = now() \
         WHERE status = 'active' AND id IN (SELECT atom_id FROM atom_entities WHERE entity_id = $1)",
    )
    .bind(entity_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 归档 source_refs 指向该会话的 active 原子（会话作废的级联遗忘）。
pub async fn archive_atoms_by_session(pool: &PgPool, session_id: &str) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE atoms SET status = 'archived', updated_at = now() \
         WHERE status = 'active' AND EXISTS ( \
            SELECT 1 FROM jsonb_array_elements(atoms.source_refs) e \
            WHERE e->>'session_id' = $1)",
    )
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 全量导出（P4）：sensitive 原子默认排除（隐私面不随导出扩大），显式包含才可见。
pub async fn list_atoms_all(pool: &PgPool, include_sensitive: bool) -> StoreResult<Vec<AtomDto>> {
    let rows: Vec<AtomDto> = sqlx::query_as(if include_sensitive {
        "SELECT * FROM atoms ORDER BY created_at"
    } else {
        "SELECT * FROM atoms WHERE NOT sensitive ORDER BY created_at"
    })
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn atoms_by_ids(pool: &PgPool, ids: &[Uuid]) -> StoreResult<Vec<AtomDto>> {
    let rows = sqlx::query_as::<_, AtomDto>("SELECT * FROM atoms WHERE id = ANY($1)")
        .bind(ids)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// context_pack 无 query 路径：热度头部（过滤过期与敏感）。
pub async fn recent_active_atoms(pool: &PgPool, limit: i64) -> StoreResult<Vec<AtomDto>> {
    let rows = sqlx::query_as(
        "SELECT * FROM atoms WHERE status = 'active' AND NOT sensitive \
         AND (valid_until IS NULL OR valid_until > now()) \
         ORDER BY hit_count DESC, confidence DESC, created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 人审代问（议题三）：队列里的低置信项带给 AI。
pub async fn pending_review_atoms(pool: &PgPool) -> StoreResult<Vec<AtomDto>> {
    let rows = sqlx::query_as(
        "SELECT * FROM atoms WHERE needs_review AND status = 'active' AND NOT sensitive ORDER BY created_at DESC LIMIT 5",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// L0 擦除：找出 source_refs 引用该会话的原子（加 erased 标记用）。
pub async fn list_atom_refs_like(
    pool: &PgPool,
    session_id: Uuid,
) -> StoreResult<Vec<(Uuid, Value)>> {
    let rows: Vec<(Uuid, Value)> =
        sqlx::query_as("SELECT id, source_refs FROM atoms WHERE source_refs::text LIKE $1")
            .bind(format!("%{session_id}%"))
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

pub async fn update_atom_source_refs(
    pool: &PgPool,
    atom_id: Uuid,
    refs: &Value,
) -> StoreResult<()> {
    sqlx::query("UPDATE atoms SET source_refs = $2, updated_at = now() WHERE id = $1")
        .bind(atom_id)
        .bind(sqlx::types::Json(refs))
        .execute(pool)
        .await?;
    Ok(())
}

/// B9 命中反馈：hit_count + 1（不刷 updated_at——hit 是使用热度而非内容变化）。
pub async fn bump_hit_counts(pool: &PgPool, table: &str, ids: &[Uuid]) -> StoreResult<()> {
    let sql = if table == "atoms" {
        "UPDATE atoms SET hit_count = hit_count + 1 WHERE id = ANY($1)"
    } else {
        "UPDATE scenarios SET hit_count = hit_count + 1 WHERE id = ANY($1)"
    };
    sqlx::query(sql).bind(ids).execute(pool).await?;
    Ok(())
}

// ---------- L2 场景 ----------

pub async fn list_scenarios(pool: &PgPool, limit: i64) -> StoreResult<Vec<ScenarioDto>> {
    Ok(sqlx::query_as::<_, ScenarioDto>(
        "SELECT * FROM scenarios ORDER BY updated_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

pub async fn find_scenario(pool: &PgPool, id: Uuid) -> StoreResult<Option<ScenarioDto>> {
    sqlx::query_as::<_, ScenarioDto>("SELECT * FROM scenarios WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn list_scenarios_all(pool: &PgPool) -> StoreResult<Vec<ScenarioDto>> {
    let rows = sqlx::query_as("SELECT * FROM scenarios ORDER BY created_at")
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

// ---------- L3 画像 ----------

/// 当前画像（每分面最新版）。
pub async fn persona_current(pool: &PgPool) -> StoreResult<Vec<PersonaVersion>> {
    Ok(sqlx::query_as::<_, PersonaVersion>(
        "SELECT DISTINCT ON (aspect) * FROM persona_aspects ORDER BY aspect, version DESC",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn persona_history(pool: &PgPool, aspect: &str) -> StoreResult<Vec<PersonaVersion>> {
    Ok(sqlx::query_as::<_, PersonaVersion>(
        "SELECT * FROM persona_aspects WHERE aspect = $1 ORDER BY version DESC",
    )
    .bind(aspect)
    .fetch_all(pool)
    .await?)
}

/// 全部历史（export，按 aspect, version 升序）。
pub async fn persona_all(pool: &PgPool) -> StoreResult<Vec<PersonaVersion>> {
    let rows = sqlx::query_as("SELECT * FROM persona_aspects ORDER BY aspect, version")
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

pub async fn persona_max_version(pool: &PgPool, aspect: &str) -> StoreResult<Option<Option<i32>>> {
    let cur: Option<Option<i32>> =
        sqlx::query_scalar("SELECT MAX(version) FROM persona_aspects WHERE aspect = $1")
            .bind(aspect)
            .fetch_optional(pool)
            .await?;
    Ok(cur)
}

/// 最新一版的内容与版本号（repin 用）。
pub async fn persona_latest(pool: &PgPool, aspect: &str) -> StoreResult<Option<(String, i32)>> {
    let cur: Option<(String, i32)> = sqlx::query_as(
        "SELECT content, version FROM persona_aspects WHERE aspect = $1 ORDER BY version DESC LIMIT 1",
    )
    .bind(aspect)
    .fetch_optional(pool)
    .await?;
    Ok(cur)
}

/// 编辑/重钉：以新版本落地人工内容（钉住 = 蒸馏绕开）。
pub async fn insert_persona_pinned(
    pool: &PgPool,
    id: Uuid,
    aspect: &str,
    content: &str,
    version: i32,
) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO persona_aspects (id, aspect, content, evidence_refs, version, prompt_version, manually_edited) \
         VALUES ($1, $2, $3, '[]'::jsonb, $4, 'human', true)",
    )
    .bind(id)
    .bind(aspect)
    .bind(content)
    .bind(version)
    .execute(pool)
    .await?;
    Ok(())
}

/// 回滚：以新版本号落地目标版本内容（历史不可变）。
pub async fn insert_persona_rollback(
    pool: &PgPool,
    id: Uuid,
    aspect: &str,
    content: &str,
    version: i32,
    evidence: &Value,
) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO persona_aspects (id, aspect, content, evidence_refs, version, prompt_version, manually_edited) \
         VALUES ($1, $2, $3, $4::jsonb, $5, 'rollback', true)",
    )
    .bind(id)
    .bind(aspect)
    .bind(content)
    .bind(sqlx::types::Json(evidence))
    .bind(version)
    .execute(pool)
    .await?;
    Ok(())
}

/// 解除钉住：分面回归蒸馏管辖。
pub async fn persona_unpin(pool: &PgPool, aspect: &str) -> StoreResult<()> {
    sqlx::query(
        "UPDATE persona_aspects SET manually_edited = false \
         WHERE id IN (SELECT id FROM persona_aspects WHERE aspect = $1 ORDER BY version DESC LIMIT 1)",
    )
    .bind(aspect)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------- 实体（记忆星系） ----------

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
         WHERE e.merged_into IS NULL AND ($1::text IS NULL OR e.kind = $1) \
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
        "SELECT id FROM entities WHERE name = $1 AND kind = $2 AND merged_into IS NULL",
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
    sqlx::query("UPDATE entities SET merged_into = $2, updated_at = now() WHERE id = $1")
        .bind(from)
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
         manually_edited, updated_at FROM entities WHERE id = ANY($1) AND merged_into IS NULL",
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

// ---------- 实体关系 ----------

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

// ---------- 汇总/时间轴/审计 ----------

/// 全局记忆时间轴：原子（occurred_at 优先）/场景/实体按时间倒序合并。
pub async fn timeline(pool: &PgPool, limit: i64) -> StoreResult<Vec<TimelineEvent>> {
    Ok(sqlx::query_as::<_, TimelineEvent>(
        "SELECT a.id, COALESCE(a.occurred_at, a.created_at) AS at, 'atom' AS kind, a.content \
         FROM atoms a WHERE a.status = 'active' AND NOT a.sensitive \
         UNION ALL \
         SELECT s.id, s.created_at AS at, 'scenario' AS kind, s.topic \
         FROM scenarios s \
         UNION ALL \
         SELECT e.id, e.created_at AS at, 'entity' AS kind, e.name \
         FROM entities e WHERE e.merged_into IS NULL \
         ORDER BY at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

/// 记忆域缺失向量统计（重嵌修复入口的状态面）：(atoms_missing, scenarios_missing)。
pub async fn embedding_missing_counts(pool: &PgPool) -> StoreResult<(i64, i64)> {
    let atoms_missing: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM atoms WHERE status = 'active' AND embedding IS NULL",
    )
    .fetch_one(pool)
    .await?;
    let scenarios_missing: i64 =
        sqlx::query_scalar("SELECT count(*) FROM scenarios WHERE embedding IS NULL")
            .fetch_one(pool)
            .await?;
    Ok((atoms_missing, scenarios_missing))
}

/// 编辑/清空类审计：写一条已完成的 job 行（谁、何时、干了什么）——不可抵赖凭证。
pub async fn audit(pool: &PgPool, kind: &str, payload: Value) {
    sqlx::query(
        "INSERT INTO jobs (id, kind, payload, status, attempts, max_attempts, \
         progress, started_at, finished_at) \
         VALUES ($1, $2, $3, 'succeeded', 1, 1, $3, now(), now())",
    )
    .bind(Uuid::now_v7())
    .bind(kind)
    .bind(payload)
    .execute(pool)
    .await
    .ok();
}

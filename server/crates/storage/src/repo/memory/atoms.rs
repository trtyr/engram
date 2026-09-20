//! `memory` 的实现切片（架构治理 2026-09-21：自 memory.rs 纯搬移，零行为变化）。

use super::*;

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
    strength: &str,
    source_kind: &str,
) -> StoreResult<AtomDto> {
    let row = sqlx::query_as::<_, AtomDto>(
        "INSERT INTO atoms (id, kind, content, confidence, status, needs_review, sensitive, occurred_at, valid_until, source_refs, embedding, tsv, strength, source_kind) \
         VALUES ($1, $2, $3, $4, 'active', $5, $6, $7, $8, '[]'::jsonb, $9, to_tsvector('simple', $10), $11, $12) RETURNING *",
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
    .bind(strength)
    .bind(source_kind)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// correct 快路径（收录哲学线）：单事务「新原子 active + 旧原子 superseded+指针」。
/// 治理守卫：target 必须 active 且非 sensitive（SQL WHERE 再守一道，core 层已前置校验）。
/// target 不满足守卫时回滚并返回 None（core 层转译为可行动报错）。
pub async fn correct_atom(
    pool: &PgPool,
    new_id: Uuid,
    target_id: Uuid,
    kind: &str,
    text: &str,
    embedding: Option<Vec<f32>>,
    tsv: &str,
) -> StoreResult<Option<AtomDto>> {
    let mut tx = pool.begin().await?;
    let new_row = sqlx::query_as::<_, AtomDto>(
        "INSERT INTO atoms (id, kind, content, confidence, status, needs_review, source_refs, embedding, tsv, strength, source_kind) \
         VALUES ($1, $2, $3, 0.95, 'active', false, '[]'::jsonb, $4, to_tsvector('simple', $5), 'fact', 'user_stated') RETURNING *",
    )
    .bind(new_id)
    .bind(kind)
    .bind(text)
    .bind(embedding.map(pgvector::Vector::from))
    .bind(tsv)
    .fetch_one(&mut *tx)
    .await?;
    let updated = sqlx::query(
        "UPDATE atoms SET status = 'superseded', superseded_by = $2, updated_at = now() \
         WHERE id = $1 AND status = 'active' AND NOT sensitive",
    )
    .bind(target_id)
    .bind(new_id)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() != 1 {
        tx.rollback().await?;
        return Ok(None);
    }
    tx.commit().await?;
    Ok(Some(new_row))
}

/// 待审 AI 复核：confirm——摘 needs_review 标记（仅 needs_review=true 且 active 可处置）。
pub async fn review_confirm(pool: &PgPool, id: Uuid) -> StoreResult<Option<AtomDto>> {
    let row = sqlx::query_as::<_, AtomDto>(
        "UPDATE atoms SET needs_review = false, updated_at = now() \
         WHERE id = $1 AND needs_review = true AND status = 'active' RETURNING *",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 待审 AI 复核：discard——归档（仅 needs_review=true 且 active 可处置）。
pub async fn review_discard(pool: &PgPool, id: Uuid) -> StoreResult<Option<AtomDto>> {
    let row = sqlx::query_as::<_, AtomDto>(
        "UPDATE atoms SET status = 'archived', needs_review = false, updated_at = now() \
         WHERE id = $1 AND needs_review = true AND status = 'active' RETURNING *",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// distill 触发撞车检测：是否存在 running 的 extract_atoms（手动触发防重复投递——
/// 收录哲学线：撞车时只提示不投递，任务列表不留空跑记录）。
pub async fn count_running_extract(pool: &PgPool) -> StoreResult<i64> {
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE kind = 'extract_atoms' AND status = 'running'",
    )
    .fetch_one(pool)
    .await?;
    Ok(n)
}

/// 画像全量重建撞车检测：是否存在 running 的 distill_persona（重建撞重建只提示不投递——
/// 收录哲学线 task-10，与 count_running_extract 同模）。
pub async fn count_running_persona(pool: &PgPool) -> StoreResult<i64> {
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE kind = 'distill_persona' AND status = 'running'",
    )
    .fetch_one(pool)
    .await?;
    Ok(n)
}

/// 按会话 id 反查蒸馏产物（source_refs 含该 session 的原子，新→旧）。
pub async fn atoms_by_session(pool: &PgPool, session_id: Uuid) -> StoreResult<Vec<AtomDto>> {
    let cond = serde_json::json!([{"session_id": session_id}]).to_string();
    Ok(sqlx::query_as::<_, AtomDto>(
        "SELECT * FROM atoms WHERE source_refs @> $1::jsonb ORDER BY created_at DESC LIMIT 200",
    )
    .bind(cond)
    .fetch_all(pool)
    .await?)
}

/// 字面量 ILIKE 兜底（工单「库里有一搜没有」）：FTS/向量双腿零命中后直查 content。
#[derive(sqlx::FromRow)]
pub struct AtomLiteralHit {
    pub id: Uuid,
    pub kind: String,
    pub content: String,
    pub needs_review: bool,
}

pub async fn atoms_literal_fallback(
    pool: &PgPool,
    limit: i64,
    needle: &str,
    reveal_sensitive: bool,
) -> StoreResult<Vec<AtomLiteralHit>> {
    let pat = format!("%{}%", needle.replace('%', "\\%").replace('_', "\\_"));
    let sens = if reveal_sensitive {
        "true"
    } else {
        "NOT sensitive"
    };
    let sql = format!(
        "SELECT id, kind, content, needs_review FROM atoms \
         WHERE status = 'active' AND ({sens}) AND content ILIKE $2 \
         ORDER BY created_at DESC LIMIT $1"
    );
    Ok(sqlx::query_as::<_, AtomLiteralHit>(&sql)
        .bind(limit)
        .bind(pat)
        .fetch_all(pool)
        .await?)
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

/// 归档 source_refs 指向该会话的原子（会话作废/擦除的级联遗忘）。
/// D2（R 报告）：superseded 一并归档——被取代的旧原子同样源自该会话，
/// 只归档 active 会留下「来源已遗忘、派生还挂 superseded」的不一致状态。
pub async fn archive_atoms_by_session(pool: &PgPool, session_id: &str) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE atoms SET status = 'archived', updated_at = now() \
         WHERE status IN ('active','superseded') AND EXISTS ( \
            SELECT 1 FROM jsonb_array_elements(atoms.source_refs) e \
            WHERE e->>'session_id' = $1)",
    )
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// unvoid 恢复：该会话源的被归档原子回到归档前状态——
/// superseded_by 还在 → 回 superseded（它确实被更新的原子取代）；
/// 否则回 active（被 void 误伤的正常记忆）。superseded_by 指针在归档时保留，恢复由此判别。
pub async fn restore_atoms_by_session(pool: &PgPool, session_id: &str) -> StoreResult<u64> {
    let res = sqlx::query(
        "UPDATE atoms SET status = CASE WHEN superseded_by IS NOT NULL THEN 'superseded' \
         ELSE 'active' END, updated_at = now() \
         WHERE status = 'archived' AND EXISTS ( \
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
        "SELECT * FROM atoms WHERE status = 'active' \
         AND (valid_until IS NULL OR valid_until > now()) \
         ORDER BY hit_count DESC, confidence DESC, created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 待审代问（议题三）：队列里的低置信项带给 AI。
pub async fn pending_review_atoms(pool: &PgPool) -> StoreResult<Vec<AtomDto>> {
    let rows = sqlx::query_as(
        "SELECT * FROM atoms WHERE needs_review AND status = 'active' ORDER BY created_at DESC LIMIT 5",
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

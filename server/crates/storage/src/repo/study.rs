//! study 学习路线图仓储（P007-T002）：track + item 两表 CRUD。
//! 过程状态机的转换合法性收口在 core::StudyService，本层只存取。

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::StoreResult;

/// 学习领域 track。
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct StudyTrackRow {
    pub id: Uuid,
    pub name: String,
    pub goal: String,
    pub status: String, // active|paused|done
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// 知识单元 item。
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct StudyItemRow {
    pub id: Uuid,
    pub track_id: Uuid,
    pub name: String,
    pub status: String, // not_started|learning|learned
    pub position: i32,
    pub wiki_slugs: serde_json::Value,
    pub doc_ids: serde_json::Value,
    pub learned_at: Option<chrono::DateTime<chrono::Utc>>,
    pub needs_review: bool,
    pub review_due_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// journal 进度时间线条目（P007 二期 T012）。
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct StudyJournalRow {
    pub id: Uuid,
    pub track_id: Uuid,
    pub note: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// ---------- track ----------

pub async fn track_create(
    pool: &PgPool,
    id: Uuid,
    name: &str,
    goal: &str,
) -> StoreResult<()> {
    sqlx::query("INSERT INTO study_tracks (id, name, goal) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(name)
        .bind(goal)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn track_get(pool: &PgPool, id: Uuid) -> StoreResult<Option<StudyTrackRow>> {
    let row = sqlx::query_as::<_, StudyTrackRow>(
        "SELECT id, name, goal, status, created_at, updated_at FROM study_tracks WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 全量 tracks（updated_at DESC）。
pub async fn track_list(pool: &PgPool) -> StoreResult<Vec<StudyTrackRow>> {
    let rows = sqlx::query_as::<_, StudyTrackRow>(
        "SELECT id, name, goal, status, created_at, updated_at FROM study_tracks \
         ORDER BY updated_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 补丁式更新（None=不动）。
pub async fn track_update(
    pool: &PgPool,
    id: Uuid,
    name: Option<&str>,
    goal: Option<&str>,
    status: Option<&str>,
) -> StoreResult<()> {
    sqlx::query(
        "UPDATE study_tracks SET \
         name = COALESCE($2, name), \
         goal = COALESCE($3, goal), \
         status = COALESCE($4, status), \
         updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(name)
    .bind(goal)
    .bind(status)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn track_delete(pool: &PgPool, id: Uuid) -> StoreResult<()> {
    sqlx::query("DELETE FROM study_tracks WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ---------- item ----------

pub async fn item_create(
    pool: &PgPool,
    id: Uuid,
    track_id: Uuid,
    name: &str,
    position: i32,
) -> StoreResult<()> {
    sqlx::query(
        "INSERT INTO study_track_items (id, track_id, name, position) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(track_id)
    .bind(name)
    .bind(position)
    .execute(pool)
    .await?;
    Ok(())
}

/// track 全部 items（position ASC, created 稳定排序）。
pub async fn items_by_track(pool: &PgPool, track_id: Uuid) -> StoreResult<Vec<StudyItemRow>> {
    let rows = sqlx::query_as::<_, StudyItemRow>(
        "SELECT id, track_id, name, status, position, wiki_slugs, doc_ids, learned_at, needs_review, review_due_at, \
         created_at, updated_at \
         FROM study_track_items WHERE track_id = $1 ORDER BY position ASC, created_at ASC",
    )
    .bind(track_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn item_get(pool: &PgPool, id: Uuid) -> StoreResult<Option<StudyItemRow>> {
    let row = sqlx::query_as::<_, StudyItemRow>(
        "SELECT id, track_id, name, status, position, wiki_slugs, doc_ids, learned_at, needs_review, review_due_at, \
         created_at, updated_at \
         FROM study_track_items WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 补丁式更新；status 置 learned 时记 learned_at，离开 learned 清空。
#[allow(clippy::too_many_arguments)]
pub async fn item_update(
    pool: &PgPool,
    id: Uuid,
    name: Option<&str>,
    status: Option<&str>,
    position: Option<i32>,
    wiki_slugs: Option<&serde_json::Value>,
    doc_ids: Option<&serde_json::Value>,
) -> StoreResult<()> {
    sqlx::query(
        "UPDATE study_track_items SET \
         name = COALESCE($2, name), \
         status = COALESCE($3, status), \
         position = COALESCE($4, position), \
         wiki_slugs = COALESCE($5, wiki_slugs), \
         doc_ids = COALESCE($6, doc_ids), \
         learned_at = CASE WHEN $3 = 'learned' THEN now() ELSE NULL END, \
         updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(name)
    .bind(status)
    .bind(position)
    .bind(wiki_slugs)
    .bind(doc_ids)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn item_delete(pool: &PgPool, id: Uuid) -> StoreResult<()> {
    sqlx::query("DELETE FROM study_track_items WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 进度统计（core topic_get 用）。
pub async fn track_progress(
    pool: &PgPool,
    track_id: Uuid,
) -> StoreResult<(i64, i64)> {
    let (total, learned): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE status = 'learned') \
         FROM study_track_items WHERE track_id = $1",
    )
    .bind(track_id)
    .fetch_one(pool)
    .await?;
    Ok((total, learned))
}

/// SRS 复习标记（P007 二期 T011）：needs_review 开关 + 到期时间。
pub async fn item_set_review(
    pool: &PgPool,
    id: Uuid,
    needs_review: bool,
    due: Option<chrono::DateTime<chrono::Utc>>,
) -> StoreResult<()> {
    sqlx::query(
        "UPDATE study_track_items SET needs_review = $2, review_due_at = $3, updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(needs_review)
    .bind(due)
    .execute(pool)
    .await?;
    Ok(())
}

/// 复习队列：已标记且（无到期时间=立即到期 或 已到期），按到期时间升序。
pub async fn reviews_due(pool: &PgPool) -> StoreResult<Vec<StudyItemRow>> {
    let rows = sqlx::query_as::<_, StudyItemRow>(
        "SELECT id, track_id, name, status, position, wiki_slugs, doc_ids, learned_at, \
         needs_review, review_due_at, created_at, updated_at \
         FROM study_track_items \
         WHERE needs_review = TRUE AND (review_due_at IS NULL OR review_due_at <= now()) \
         ORDER BY review_due_at ASC NULLS FIRST",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

// ---------- journal（P007 二期 T012） ----------

pub async fn journal_add(
    pool: &PgPool,
    id: Uuid,
    track_id: Uuid,
    note: &str,
) -> StoreResult<()> {
    sqlx::query("INSERT INTO study_track_journal (id, track_id, note) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(track_id)
        .bind(note)
        .execute(pool)
        .await?;
    Ok(())
}

/// track 最近时间线（新→旧）。
pub async fn journal_by_track(
    pool: &PgPool,
    track_id: Uuid,
    limit: i64,
) -> StoreResult<Vec<StudyJournalRow>> {
    let rows = sqlx::query_as::<_, StudyJournalRow>(
        "SELECT id, track_id, note, created_at FROM study_track_journal \
         WHERE track_id = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(track_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

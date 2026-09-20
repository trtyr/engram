//! `memory` 的实现切片（架构治理 2026-09-21：自 memory.rs 纯搬移，零行为变化）。

use super::*;

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

/// 当前画像（每分面最新版）。
/// 当前画像：每分面取最大版本。**空 content 的版本是蒸馏 F4 清退的退休标记**
/// （分面引用了被移除的表述 → 写空版本退场）——退休中的分面没有「当前内容」，
/// 不得返回：否则 Dashboard 把空壳算进画像数、context/search 把空分面注进预算
/// （2026-09-08 用户：画像明明空的前端却写 6）。下一轮蒸馏重写真实内容后分面自然回归。
pub async fn persona_current(pool: &PgPool) -> StoreResult<Vec<PersonaVersion>> {
    Ok(sqlx::query_as::<_, PersonaVersion>(
        "SELECT * FROM (              SELECT DISTINCT ON (aspect) * FROM persona_aspects ORDER BY aspect, version DESC          ) t WHERE content <> '' ORDER BY aspect",
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

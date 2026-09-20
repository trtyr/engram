//! `memory` 的实现切片（架构治理 2026-09-21：自 memory.rs 纯搬移，零行为变化）。

use super::*;

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

//! jobs 表的管理面读写（HTTP 控制台用）：队列本体见 [`crate::queue`]。
//!
//! jobs 表的 SQL 本就归本 crate 所有；这里收口的是「Web 管理面」的
//! 定点读写（deep purge 两阶段的取消/校验/收尾、wiki 提案聚合），
//! 让 api 层不再出现裸 SQL。

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// 取消 pending 的 deep_purge job（后悔药）。返回生效行数（0 = 不存在或已执行/已取消）。
pub async fn cancel_pending_deep_purge(pool: &PgPool, job_id: Uuid) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE jobs SET status = 'cancelled', finished_at = now() \
         WHERE id = $1 AND kind = 'deep_purge' AND status = 'pending'",
    )
    .bind(job_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 校验 armed 状态的 deep_purge job（阶段二 token 校验）。
pub async fn find_armed_deep_purge(
    pool: &PgPool,
    job_id: Uuid,
) -> Result<Option<(Uuid, Value)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, payload FROM jobs \
         WHERE id = $1 AND kind = 'deep_purge' AND status = 'pending' AND payload->>'phase' = 'armed'",
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await
}

/// deep_purge 执行收尾：armed job 标记 succeeded 并写入结果与执行者。
pub async fn complete_deep_purge(
    pool: &PgPool,
    job_id: Uuid,
    payload: &Value,
    progress: &Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE jobs SET status = 'succeeded', payload = $2, progress = $3, \
         started_at = now(), finished_at = now() WHERE id = $1",
    )
    .bind(job_id)
    .bind(payload)
    .bind(progress)
    .execute(pool)
    .await?;
    Ok(())
}

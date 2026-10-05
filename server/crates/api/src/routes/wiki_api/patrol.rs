//! wiki 巡逻（maintain_wiki）的 API 面：GET 最近巡检报告 + POST 手动触发。
//! P014 后运维操作面收敛——用户只看报告，巡逻由节律自动跑。

use super::*;

/// 最近一次巡逻结果（maintain_wiki 最新终态任务的 progress）+ 下次巡逻时间。
#[utoipa::path(get, path = "/wiki/patrol/latest",
    responses((status = 200, body = Object)))]
pub async fn patrol_latest(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_wiki_read(&principal)?;
    let row: Option<(
        uuid::Uuid,
        String,
        Option<serde_json::Value>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT id, status::text, progress, finished_at FROM jobs \
             WHERE kind = 'maintain_wiki' AND status IN ('succeeded', 'failed', 'dead') \
             ORDER BY finished_at DESC NULLS LAST LIMIT 1",
    )
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError::Unavailable(e.to_string()))?;

    // 下次巡逻 = rhythm_maintain_wiki 最早 pending 的 due
    let next_due: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT min(due_at) FROM jobs WHERE kind = 'rhythm_maintain_wiki' AND status = 'pending'",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|e| ApiError::Unavailable(e.to_string()))?;

    let patrol = row.map(|(id, status, progress, finished_at)| {
        serde_json::json!({
            "job_id": id,
            "status": status,
            "finished_at": finished_at,
            "report": progress,
        })
    });
    Ok(Json(serde_json::json!({
        "patrol": patrol,
        "next_due": next_due,
    })))
}

/// 手动触发巡逻（单飞守卫：running 即提示不重复投递）。
#[utoipa::path(post, path = "/wiki/patrol",
    responses((status = 200, body = Object)))]
pub async fn patrol_trigger(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_wiki(&principal)?;
    let running: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE kind = 'maintain_wiki' AND status = 'running'",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    if running > 0 {
        return Ok(Json(serde_json::json!({
            "already_running": true,
            "hint": "巡逻进行中——等它完成看报告",
        })));
    }
    let job = engram_jobs::JobQueue::new(state.pool.clone())
        .enqueue(
            engram_jobs::JobTemplate::new("maintain_wiki")
                .with_payload(serde_json::json!({"reason": "manual"})),
        )
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    Ok(Json(serde_json::json!({
        "already_running": false,
        "jobs": [{ "id": job.id.to_string(), "kind": job.kind }],
    })))
}

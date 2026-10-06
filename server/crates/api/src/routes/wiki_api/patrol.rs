//! wiki 巡逻（maintain_wiki）的 API 面：GET 最近巡检报告 + POST 手动触发。
//! P014 后运维操作面收敛——用户只看报告，巡逻由节律自动跑。

use super::*;
use serde_json::json;

/// sqlx 行解构 type alias（消 clippy type_complexity）。
type PatrolRow = (
    uuid::Uuid,
    String,
    Option<serde_json::Value>,
    Option<chrono::DateTime<chrono::Utc>>,
);
type PatrolDetailRow = (
    String,
    Option<chrono::DateTime<chrono::Utc>>,
    Option<serde_json::Value>,
);

/// 最近一次巡逻结果（maintain_wiki 最新终态任务的 progress）+ 下次巡逻时间。
#[utoipa::path(get, path = "/wiki/patrol/latest",
    responses((status = 200, body = Object)))]
pub async fn patrol_latest(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_wiki_read(&principal)?;
    let row: Option<PatrolRow> = sqlx::query_as(
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

/// 巡逻历史列表（最近 50 次）：每次巡逻一条，前端左列表用。
#[utoipa::path(get, path = "/wiki/patrol/list",
    responses((status = 200, body = Object)))]
pub async fn patrol_list(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_wiki_read(&principal)?;
    let rows: Vec<PatrolRow> = sqlx::query_as(
        "SELECT id, status::text, progress, finished_at FROM jobs \
             WHERE kind = 'maintain_wiki' AND status IN ('succeeded', 'failed', 'dead') \
             ORDER BY finished_at DESC NULLS LAST LIMIT 50",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|(id, status, progress, finished_at)| {
            let p = progress.as_ref();
            serde_json::json!({
                "job_id": id,
                "status": status,
                "finished_at": finished_at,
                "lint_issues": p.and_then(|v| v.get("lint_issues")).cloned().unwrap_or(json!(0)),
                "repair_actions": p.and_then(|v| v.get("repair_actions")).cloned().unwrap_or(json!(0)),
                "duplicate_candidates": p.and_then(|v| v.get("duplicate_candidates")).cloned().unwrap_or(json!(0)),
                "summary": p.and_then(|v| v.get("agent_summary")).cloned().unwrap_or(json!(null)),
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "items": items })))
}

/// 单次巡逻完整报告（含 markdown 存档）。
#[utoipa::path(get, path = "/wiki/patrol/{id}",
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = Object), (status = 404)))]
pub async fn patrol_detail(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_wiki_read(&principal)?;
    let row: Option<PatrolDetailRow> = sqlx::query_as(
        "SELECT status::text, finished_at, progress FROM jobs \
             WHERE kind = 'maintain_wiki' AND id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    let Some((status, finished_at, progress)) = row else {
        return Err(ApiError::NotFound(format!("巡逻任务 {id} 不存在")));
    };
    Ok(Json(serde_json::json!({
        "job_id": id,
        "status": status,
        "finished_at": finished_at,
        "report": progress,
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

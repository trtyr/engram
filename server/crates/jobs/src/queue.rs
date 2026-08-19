//! 队列操作：入队、抢占、心跳、完成/失败、回收、事件、查询。

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::types::{Job, JobError, JobEvent, JobStatus, JobTemplate};

/// PG-backed 任务队列。所有操作幂等或显式声明语义。
#[derive(Clone)]
pub struct JobQueue {
    pool: PgPool,
}

impl JobQueue {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 内部连接池（handler 直接操作域表用）。
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// 入队。幂等键命中时返回既有任务（不重复入队）。
    pub async fn enqueue(&self, template: JobTemplate) -> Result<Job, JobError> {
        // 幂等键先查（任何非终态或已成功的同键任务都直接复用）
        if let Some(key) = &template.idempotency_key {
            let existing =
                sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE idempotency_key = $1 LIMIT 1")
                    .bind(key)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|e| JobError::Retryable(e.to_string()))?;
            if let Some(job) = existing {
                tracing::debug!(job_id = %job.id, kind = %job.kind, "幂等键命中，复用既有任务");
                return Ok(job);
            }
        }

        let id = Uuid::now_v7();
        let due = template.due_at.unwrap_or_else(Utc::now);
        let job = sqlx::query_as::<_, Job>(
            "INSERT INTO jobs (id, kind, payload, max_attempts, idempotency_key, due_at, visibility_timeout_s)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING *",
        )
        .bind(id)
        .bind(&template.kind)
        .bind(sqlx::types::Json(&template.payload))
        .bind(template.max_attempts)
        .bind(&template.idempotency_key)
        .bind(due)
        .bind(template.visibility_timeout_s)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            // 幂等键唯一约束竞争窗口：并发插入同键
            if e.to_string().contains("duplicate key") {
                JobError::Permanent("idempotency_conflict".into())
            } else {
                JobError::Retryable(e.to_string())
            }
        })?;

        self.emit(job.id, "info", "任务入队", None).await.ok();
        Ok(job)
    }

    /// 批量抢占待处理任务（FOR UPDATE SKIP LOCKED），置 running 并锁定。
    pub async fn claim(&self, worker_id: &str, limit: i64) -> Result<Vec<Job>, JobError> {
        let jobs = sqlx::query_as::<_, Job>(
            "UPDATE jobs SET
                status = 'running',
                locked_by = $1,
                locked_at = now(),
                started_at = COALESCE(started_at, now()),
                attempts = attempts + 1
             WHERE id IN (
                SELECT id FROM jobs
                WHERE status = 'pending' AND due_at <= now()
                ORDER BY due_at
                LIMIT $2
                FOR UPDATE SKIP LOCKED
             )
             RETURNING *",
        )
        .bind(worker_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;

        for job in &jobs {
            tracing::info!(job_id = %job.id, kind = %job.kind, attempt = job.attempts, worker = worker_id, "任务抢占");
        }
        Ok(jobs)
    }

    /// 回收崩溃 worker 的僵尸任务（running 超过 visibility_timeout → 重排）。
    /// 返回回收数量。
    pub async fn reap_orphans(&self) -> Result<u64, JobError> {
        let result = sqlx::query(
            "UPDATE jobs SET
                status = 'pending',
                locked_by = NULL,
                locked_at = NULL,
                error = 'worker 崩溃或超时，任务回收重排',
                due_at = now()
             WHERE status = 'running'
               AND locked_at < now() - (visibility_timeout_s || ' seconds')::interval",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        if result.rows_affected() > 0 {
            tracing::warn!(count = result.rows_affected(), "回收僵尸任务");
        }
        Ok(result.rows_affected())
    }

    /// 标记成功。
    pub async fn complete(
        &self,
        job_id: Uuid,
        result: Option<serde_json::Value>,
    ) -> Result<(), JobError> {
        sqlx::query("UPDATE jobs SET status = 'succeeded', finished_at = now(), locked_by = NULL, locked_at = NULL, progress = COALESCE($2, progress) WHERE id = $1")
            .bind(job_id)
            .bind(result.clone().map(sqlx::types::Json))
            .execute(&self.pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
        self.emit(job_id, "info", "任务成功", result).await.ok();
        Ok(())
    }

    /// 标记失败。按错误分类决定重排 / 终态。
    /// 返回去向（Rescheduled/Failed/Dead）。
    pub async fn fail(
        &self,
        job_id: Uuid,
        error: &JobError,
    ) -> Result<crate::types::FailOutcome, JobError> {
        let job = self
            .get(job_id)
            .await?
            .ok_or_else(|| JobError::Permanent(format!("任务 {job_id} 不存在")))?;
        let (message, retryable) = match error {
            JobError::Retryable(m) => (m.clone(), true),
            JobError::Permanent(m) => (m.clone(), false),
        };

        let can_retry = retryable && job.attempts < job.max_attempts;
        let outcome = if can_retry {
            // 指数退避 + 抖动：base * 2^n，封顶 30s
            let backoff_ms = (200_i64 * (1 << job.attempts.min(8) as u32)).min(30_000);
            let jitter = rand_jitter(backoff_ms);
            sqlx::query(
                "UPDATE jobs SET status = 'pending', error = $2, due_at = now() + ($3 || ' milliseconds')::interval, locked_by = NULL, locked_at = NULL WHERE id = $1",
            )
            .bind(job_id)
            .bind(&message)
            .bind(backoff_ms + jitter)
            .execute(&self.pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
            crate::types::FailOutcome::Rescheduled
        } else if retryable {
            sqlx::query("UPDATE jobs SET status = 'dead', error = $2, finished_at = now(), locked_by = NULL, locked_at = NULL WHERE id = $1")
                .bind(job_id)
                .bind(&message)
                .execute(&self.pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
            crate::types::FailOutcome::Dead
        } else {
            sqlx::query("UPDATE jobs SET status = 'failed', error = $2, finished_at = now(), locked_by = NULL, locked_at = NULL WHERE id = $1")
                .bind(job_id)
                .bind(&message)
                .execute(&self.pool)
                .await
                .map_err(|e| JobError::Retryable(e.to_string()))?;
            crate::types::FailOutcome::Failed
        };

        self.emit(
            job_id,
            "error",
            &format!(
                "任务失败（{}）: {message}",
                if retryable { "可重试" } else { "永久" }
            ),
            Some(serde_json::json!({ "retryable": retryable, "outcome": format!("{outcome:?}") })),
        )
        .await
        .ok();
        Ok(outcome)
    }

    /// 更新进度（n/total 等）。
    pub async fn progress(
        &self,
        job_id: Uuid,
        progress: serde_json::Value,
    ) -> Result<(), JobError> {
        sqlx::query("UPDATE jobs SET progress = $2 WHERE id = $1")
            .bind(job_id)
            .bind(sqlx::types::Json(progress))
            .execute(&self.pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
        Ok(())
    }

    /// 追加事件。
    pub async fn emit(
        &self,
        job_id: Uuid,
        level: &str,
        message: &str,
        data: Option<serde_json::Value>,
    ) -> Result<(), JobError> {
        sqlx::query(
            "INSERT INTO job_events (job_id, level, message, data) VALUES ($1, $2, $3, $4)",
        )
        .bind(job_id)
        .bind(level)
        .bind(message)
        .bind(data.map(sqlx::types::Json))
        .execute(&self.pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))?;
        Ok(())
    }

    /// 单查。
    pub async fn get(&self, job_id: Uuid) -> Result<Option<Job>, JobError> {
        sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))
    }

    /// 列表（kind/status 过滤 + 游标分页）。
    pub async fn list(
        &self,
        kinds: &[String],
        statuses: &[JobStatus],
        cursor: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<Job>, JobError> {
        let statuses: Vec<String> = statuses.iter().map(|s| s.to_string()).collect();
        sqlx::query_as::<_, Job>(
            "SELECT * FROM jobs
             WHERE ($1::text[] IS NULL OR kind = ANY($1))
               AND ($2::text[] IS NULL OR status::text = ANY($2))
               AND ($3::timestamptz IS NULL OR created_at < $3)
             ORDER BY created_at DESC
             LIMIT $4",
        )
        .bind(kords_opt(kinds))
        .bind(statuses_opt(&statuses))
        .bind(cursor)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))
    }

    /// 事件时间线。
    pub async fn events(
        &self,
        job_id: Uuid,
        after_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<JobEvent>, JobError> {
        sqlx::query_as::<_, JobEvent>(
            "SELECT * FROM job_events WHERE job_id = $1 AND ($2::bigint IS NULL OR id > $2) ORDER BY id LIMIT $3",
        )
        .bind(job_id)
        .bind(after_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| JobError::Retryable(e.to_string()))
    }

    /// 死任务复活（人工重跑）。
    pub async fn revive(&self, job_id: Uuid) -> Result<(), JobError> {
        sqlx::query("UPDATE jobs SET status = 'pending', attempts = 0, error = NULL, finished_at = NULL, due_at = now(), locked_by = NULL, locked_at = NULL WHERE id = $1 AND status IN ('dead','failed')")
            .bind(job_id)
            .execute(&self.pool)
            .await
            .map_err(|e| JobError::Retryable(e.to_string()))?;
        self.emit(job_id, "info", "人工复活重跑", None).await.ok();
        Ok(())
    }
}

fn kords_opt(v: &[String]) -> Option<&[String]> {
    if v.is_empty() { None } else { Some(v) }
}
fn statuses_opt(v: &[String]) -> Option<&[String]> {
    if v.is_empty() { None } else { Some(v) }
}

/// 全抖动：[0, base)
fn rand_jitter(base_ms: i64) -> i64 {
    // 无需密码学随机；用纳秒时钟做廉价抖动源
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as i64)
        .unwrap_or(0);
    if base_ms <= 0 { 0 } else { ns % base_ms }
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            JobStatus::Pending => "pending",
            JobStatus::Running => "running",
            JobStatus::Succeeded => "succeeded",
            JobStatus::Failed => "failed",
            JobStatus::Dead => "dead",
        };
        f.write_str(s)
    }
}

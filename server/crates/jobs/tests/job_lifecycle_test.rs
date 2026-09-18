//! testcontainers 集成测试：job 全生命周期（入队→执行→成功/重试/回收/dead→复活）。
//! Phase 1 出口标准的验证本体。

// 复用 storage crate 的测试基建（容器启动逻辑一致）
mod support;

use engram_jobs::types::{FailOutcome, JobError, JobStatus, JobTemplate};
use engram_jobs::{JobQueue, Runner, RunnerConfig};
use std::collections::HashMap;
use std::time::Duration;

async fn setup() -> (support::TestPg, JobQueue, sqlx::PgPool, String) {
    let container = support::start_pgvector().await.expect("启动容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    (container, JobQueue::new(pool.clone()), pool, url)
}

#[tokio::test]
async fn job_lifecycle_success() {
    let (_c, queue, _pool, _url) = setup().await;

    // 入队
    let job = queue
        .enqueue(JobTemplate::new("echo").with_payload(serde_json::json!({"v": 1})))
        .await
        .unwrap();
    assert_eq!(job.status, JobStatus::Pending);
    assert_eq!(job.attempts, 0);

    // 抢占
    let claimed = queue.claim("w-test", 10).await.unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, job.id);
    assert_eq!(claimed[0].status, JobStatus::Running);
    assert_eq!(claimed[0].attempts, 1);

    // 成功
    queue
        .complete(job.id, Some(serde_json::json!({"ok": true})))
        .await
        .unwrap();
    let done = queue.get(job.id).await.unwrap().unwrap();
    assert_eq!(done.status, JobStatus::Succeeded);
    assert!(done.finished_at.is_some());

    // 事件时间线完整：入队→抢占（无事件）→成功
    let events = queue.events(job.id, None, 100).await.unwrap();
    assert!(events.iter().any(|e| e.message.contains("入队")));
    assert!(events.iter().any(|e| e.message.contains("成功")));
}

#[tokio::test]
async fn job_retry_backoff_then_dead() {
    let (_c, queue, _pool, _url) = setup().await;

    let job = queue
        .enqueue(JobTemplate::new("flaky").with_max_attempts_for_test(2))
        .await
        .unwrap();

    // 第一次失败（可重试）→ 重排
    queue.claim("w1", 10).await.unwrap();
    let outcome = queue
        .fail(job.id, &JobError::Retryable("LLM 429".into()))
        .await
        .unwrap();
    assert_eq!(outcome, FailOutcome::Rescheduled);
    let j = queue.get(job.id).await.unwrap().unwrap();
    assert_eq!(j.status, JobStatus::Pending);
    assert_eq!(j.attempts, 1);

    // 第二次失败（可重试，但 attempts=2 已达 max）→ dead
    // 等退避窗口过去（base 400ms + 抖动 ≤ 800ms）
    tokio::time::sleep(Duration::from_millis(900)).await;
    queue.claim("w2", 10).await.unwrap();
    let outcome = queue
        .fail(job.id, &JobError::Retryable("LLM 429 again".into()))
        .await
        .unwrap();
    assert_eq!(outcome, FailOutcome::Dead);
    let j = queue.get(job.id).await.unwrap().unwrap();
    assert_eq!(j.status, JobStatus::Dead);

    // 永久错误即使有重试余额也直接 failed
    let job2 = queue.enqueue(JobTemplate::new("bad-param")).await.unwrap();
    queue.claim("w3", 10).await.unwrap();
    let outcome = queue
        .fail(job2.id, &JobError::Permanent("参数非法".into()))
        .await
        .unwrap();
    assert_eq!(outcome, FailOutcome::Failed);
    assert_eq!(
        queue.get(job2.id).await.unwrap().unwrap().status,
        JobStatus::Failed
    );

    // 复活 dead
    queue.revive(job.id).await.unwrap();
    let j = queue.get(job.id).await.unwrap().unwrap();
    assert_eq!(j.status, JobStatus::Pending);
    assert_eq!(j.attempts, 0);
}

#[tokio::test]
async fn idempotency_key_dedupes() {
    let (_c, queue, _pool, _url) = setup().await;

    let t = JobTemplate::new("ingest").with_idempotency_key("doc-abc-ingest");
    let first = queue.enqueue(t.clone()).await.unwrap();
    let second = queue.enqueue(t).await.unwrap();
    assert_eq!(first.id, second.id, "同幂等键应返回既有任务");

    // 终态后同键新任务：当前语义仍复用（调用方如需强制重跑应换键或 revive）
    queue.complete(first.id, None).await.unwrap();
    let third = queue
        .enqueue(JobTemplate::new("ingest").with_idempotency_key("doc-abc-ingest"))
        .await
        .unwrap();
    assert_eq!(third.id, first.id);
}

#[tokio::test]
async fn orphan_reap_returns_stuck_jobs() {
    let (_c, queue, _pool, _url) = setup().await;

    // visibility_timeout 极短的任务
    let job = queue
        .enqueue(
            JobTemplate::new("stuck")
                .with_payload(serde_json::json!({}))
                .with_visibility_timeout_for_test(1),
        )
        .await
        .unwrap();
    queue.claim("crashed-worker", 10).await.unwrap();
    assert_eq!(
        queue.get(job.id).await.unwrap().unwrap().status,
        JobStatus::Running
    );

    // 等 visibility timeout 过期
    tokio::time::sleep(Duration::from_millis(1300)).await;
    let reaped = queue.reap_orphans().await.unwrap();
    assert_eq!(reaped, 1);
    let j = queue.get(job.id).await.unwrap().unwrap();
    assert_eq!(j.status, JobStatus::Pending, "僵尸应被回收重排");
    assert_eq!(j.attempts, 1, "回收不吞尝试次数");
}

#[tokio::test]
async fn runner_executes_registered_handler() {
    let (_c, queue, pool, _url) = setup().await;

    let runner = Runner::new(
        pool,
        RunnerConfig {
            worker_id: "w-runner".into(),
            concurrency: 2,
            poll_interval: Duration::from_millis(50),
            batch_size: 5,
            reap_interval: Duration::from_secs(3600),
            per_kind_concurrency: HashMap::new(),
        },
    )
    .register("double", |ctx| async move {
        let v = ctx
            .job
            .payload
            .0
            .get("x")
            .and_then(|x| x.as_i64())
            .unwrap_or(0);
        ctx.emit("计算完成", None).await.ok();
        Ok(serde_json::json!({ "doubled": v * 2 }))
    });

    let handle = runner.start();
    let job = queue
        .enqueue(JobTemplate::new("double").with_payload(serde_json::json!({"x": 21})))
        .await
        .unwrap();

    // 轮询等待执行完成
    let mut done = None;
    for _ in 0..100 {
        if let Some(j) = queue.get(job.id).await.unwrap()
            && j.status == JobStatus::Succeeded
        {
            done = Some(j);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    handle
        .shutdown_and_wait(std::time::Duration::from_secs(5))
        .await;

    let done = done.expect("任务应在 runner 内执行成功");
    assert_eq!(done.progress.unwrap().0["doubled"], 42);
}

// ---- 测试辅助：模板参数注入（仅测试可见的扩展） ----
trait TestExt {
    fn with_max_attempts_for_test(self, n: i32) -> Self;
    fn with_visibility_timeout_for_test(self, s: i32) -> Self;
}
impl TestExt for JobTemplate {
    fn with_max_attempts_for_test(mut self, n: i32) -> Self {
        self.max_attempts = n;
        self
    }
    fn with_visibility_timeout_for_test(mut self, s: i32) -> Self {
        self.visibility_timeout_s = s;
        self
    }
}

/// R12 行为级：per-kind 专属池真的限流——slow kind 配 cap=1 时并发峰值=1，
/// 未配置的 fast kind 不受影响可并发（>1），证明分池只作用于配置的 kind。
#[tokio::test]
async fn per_kind_concurrency_actually_caps_running_jobs() {
    let (_c, queue, pool, _url) = setup().await;

    // 每类两个原子：cur（当前并发，可回落）+ peak（历史峰值，fetch_max 只增不减）
    let mk_pair = || {
        (
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        )
    };
    let (slow_cur, slow_peak) = mk_pair();
    let (fast_cur, fast_peak) = mk_pair();

    let mk_handler = |cur: std::sync::Arc<std::sync::atomic::AtomicUsize>,
                      peak: std::sync::Arc<std::sync::atomic::AtomicUsize>| {
        move |_ctx: engram_jobs::JobContext| {
            let cur = cur.clone();
            let peak = peak.clone();
            async move {
                let now = cur.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                peak.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(250)).await;
                cur.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                Ok(serde_json::json!({}))
            }
        }
    };

    let runner = Runner::new(
        pool,
        RunnerConfig {
            worker_id: "w-perkind".into(),
            concurrency: 4,
            poll_interval: Duration::from_millis(30),
            batch_size: 10,
            reap_interval: Duration::from_secs(3600),
            per_kind_concurrency: HashMap::from([("slow".to_string(), 1)]),
        },
    )
    .register("slow", mk_handler(slow_cur.clone(), slow_peak.clone()))
    .register("fast", mk_handler(fast_cur.clone(), fast_peak.clone()));

    let handle = runner.start();
    for _ in 0..3 {
        queue.enqueue(JobTemplate::new("slow")).await.unwrap();
        queue.enqueue(JobTemplate::new("fast")).await.unwrap();
    }

    // 等全部完成（6 个 job × 250ms + 轮询裕量）
    let mut all_done = false;
    for _ in 0..200 {
        let pending = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM jobs WHERE status NOT IN ('succeeded','failed','dead','cancelled')",
        )
        .fetch_one(queue.pool())
        .await
        .unwrap_or(1);
        if pending == 0 {
            all_done = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    handle
        .shutdown_and_wait(std::time::Duration::from_secs(5))
        .await;
    assert!(all_done, "全部任务应执行完");

    let slow = slow_peak.load(std::sync::atomic::Ordering::SeqCst);
    let fast = fast_peak.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        slow, 1,
        "配置 cap=1 的 kind 并发峰值应恰为 1（实际 {slow}）"
    );
    assert!(fast >= 2, "未配置 kind 应走全局池可并发（实际峰值 {fast}）");
}

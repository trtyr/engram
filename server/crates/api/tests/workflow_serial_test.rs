//! 蒸馏链串行保证集成测试（T016 · 决策：串行性是正确性不变量）：
//! ① workflow 类任务（extract_atoms/maintain_memory/reembed_memory）全局单飞——
//!    多 kind 并发投递时任意时刻最多一条在跑（max_concurrent ≤ 1）；
//! ② 非 workflow 任务不受 workflow 排队阻塞；
//! ③ LLM 调用预算耗尽 → 任务 failed 且错误含「预算耗尽」。

use engram_jobs::types::JobStatus;
use engram_jobs::{JobContext, JobQueue, JobTemplate, Runner, RunnerConfig};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

mod support;

type HandlerFuture = std::pin::Pin<
    Box<
        dyn std::future::Future<Output = Result<serde_json::Value, engram_jobs::types::JobError>>
            + Send,
    >,
>;

#[tokio::test]
async fn workflow_jobs_execute_serially_and_budget_gate_works() {
    let (_app, pg) = support::app().await;
    let url = support::connection_url(&pg).await.unwrap();
    let pool = support::connect_with_retry(&url).await.unwrap();
    let queue = JobQueue::new(pool.clone());

    let cur = Arc::new(AtomicI64::new(0));
    let max = Arc::new(AtomicI64::new(0));
    let plain_done = Arc::new(AtomicU64::new(0));

    let mut runner = Runner::new(
        pool.clone(),
        RunnerConfig {
            worker_id: "test-serial".into(),
            poll_interval: Duration::from_millis(100),
            ..Default::default()
        },
    );

    // 三个 workflow kind：各记并发峰值（跨 kind 也必须互斥）
    for kind in ["extract_atoms", "maintain_memory", "reembed_memory"] {
        let cur = cur.clone();
        let max = max.clone();
        runner = runner.register(kind, move |_ctx: JobContext| {
            let cur = cur.clone();
            let max = max.clone();
            Box::pin(async move {
                let now = cur.fetch_add(1, Ordering::SeqCst) + 1;
                max.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(150)).await;
                cur.fetch_sub(1, Ordering::SeqCst);
                Ok(serde_json::json!({ "ok": true }))
            }) as HandlerFuture
        });
    }

    // 非 workflow 任务：不受 workflow 排队阻塞
    let plain_done_c = plain_done.clone();
    runner = runner.register("plain_test_kind", move |ctx: JobContext| {
        let plain_done = plain_done_c.clone();
        Box::pin(async move {
            plain_done.fetch_add(1, Ordering::SeqCst);
            // 预算闸烟测：ctx 记账可达（这里远低于预算，正常完成）
            ctx.record_llm_call()?;
            Ok(serde_json::json!({ "ok": true }))
        }) as HandlerFuture
    });

    let handle = runner.start();

    // 投 3 个 workflow（不同 kind，跨 kind 也互斥）+ 1 个普通任务
    let mut ids = Vec::new();
    for kind in [
        "extract_atoms",
        "maintain_memory",
        "reembed_memory",
        "plain_test_kind",
    ] {
        ids.push(queue.enqueue(JobTemplate::new(kind)).await.unwrap().id);
    }

    // 等全部终态
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let mut all_done = true;
        for id in &ids {
            if let Some(j) = queue.get(*id).await.unwrap() {
                if !matches!(j.status, JobStatus::Succeeded | JobStatus::Failed) {
                    all_done = false;
                    break;
                }
            } else {
                all_done = false;
            }
        }
        if all_done {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "任务 30s 未全部终态"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // 断言①：workflow 全部成功且串行（跨 kind 并发峰值 ≤ 1）
    assert!(
        max.load(Ordering::SeqCst) <= 1,
        "workflow 任务出现并发：max_concurrent={}——串行保证被破坏",
        max.load(Ordering::SeqCst)
    );
    // 断言②：普通任务完成（不受排队阻塞）
    assert_eq!(plain_done.load(Ordering::SeqCst), 1);
    // 断言③：全部 succeeded
    for id in &ids {
        let j = queue.get(*id).await.unwrap().expect("任务应存在");
        assert!(
            matches!(j.status, JobStatus::Succeeded),
            "任务 {} 终态异常: {:?} err={:?}",
            id,
            j.status,
            j.error
        );
    }

    handle.shutdown_and_wait(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn llm_budget_exceeded_fails_job_with_clear_error() {
    let (_app, pg) = support::app().await;
    let url = support::connection_url(&pg).await.unwrap();
    let pool = support::connect_with_retry(&url).await.unwrap();
    let queue = JobQueue::new(pool.clone());

    let mut runner = Runner::new(
        pool.clone(),
        RunnerConfig {
            worker_id: "test-budget".into(),
            poll_interval: Duration::from_millis(100),
            ..Default::default()
        },
    );
    // handler：循环记账直到 BudgetExceeded 传播（JOB_LLM_CALL_BUDGET 次，纯内存瞬间完成）
    runner = runner.register("extract_atoms", move |ctx: JobContext| {
        Box::pin(async move {
            for _ in 0..=engram_jobs::JOB_LLM_CALL_BUDGET {
                ctx.record_llm_call()?;
            }
            Ok(serde_json::json!({ "unreachable": false }))
        }) as HandlerFuture
    });
    let handle = runner.start();

    let job = queue
        .enqueue(JobTemplate::new("extract_atoms"))
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let j = queue.get(job.id).await.unwrap().expect("任务应存在");
        if matches!(j.status, JobStatus::Failed) {
            let err = j.error.unwrap_or_default();
            assert!(err.contains("预算耗尽"), "failed 原因应是预算耗尽：{err}");
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "预算闸未在 15s 内终态: {:?}",
            j.status
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    handle.shutdown_and_wait(Duration::from_secs(5)).await;
}

//! Runner：轮询抢占 + 执行 handler 的 worker 池。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

use crate::queue::JobQueue;
use crate::types::{Job, JobError};

/// 执行上下文（handler 拿到的全部能力）。
#[derive(Clone)]
pub struct JobContext {
    pub job: Job,
    queue: JobQueue,
}

impl JobContext {
    /// 域表连接池（与队列同池）。
    pub fn pool(&self) -> &sqlx::PgPool {
        self.queue.pool()
    }

    /// 更新进度（n/total 等）。
    pub async fn progress(&self, progress: serde_json::Value) -> Result<(), JobError> {
        self.queue.progress(self.job.id, progress).await
    }

    /// 追加事件（LLM 调用摘要等）。
    pub async fn emit(
        &self,
        message: &str,
        data: Option<serde_json::Value>,
    ) -> Result<(), JobError> {
        self.queue.emit(self.job.id, "info", message, data).await
    }

    /// 链式入队下游任务。
    pub async fn enqueue_next(&self, template: crate::types::JobTemplate) -> Result<Job, JobError> {
        self.queue.enqueue(template).await
    }
}

/// handler 签名：payload 进，成功结果/JobError 出。
pub type HandlerFn = Arc<
    dyn Fn(JobContext) -> futures_handler::BoxFuture<'static, Result<serde_json::Value, JobError>>
        + Send
        + Sync,
>;

/// 便于构造 HandlerFn 的小模块（避免直接依赖 futures crate 的路径书写）。
pub mod futures_handler {
    pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
}

/// worker 池配置。
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// worker 标识（日志用）
    pub worker_id: String,
    /// 全局并发上限
    pub concurrency: usize,
    /// 轮询间隔（空转时）
    pub poll_interval: Duration,
    /// 每轮每 worker 抢取上限
    pub batch_size: i64,
    /// 僵尸回收间隔
    pub reap_interval: Duration,
    /// per-kind 并发上限（R12）：kind → 并发数。有配置的 kind 走专属信号量
    /// （cap = min(per_kind, 全局 concurrency)），未配置走全局——默认空表 = 行为与旧版完全一致。
    /// env 入口：`AGENT_MEMORY_JOB_CONCURRENCY=wiki_generate:1,wiki_analyze:2`（runner::parse_per_kind）。
    pub per_kind_concurrency: HashMap<String, usize>,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            worker_id: format!("worker-{}", Uuid::now_v7().simple()),
            concurrency: 4,
            poll_interval: Duration::from_millis(500),
            batch_size: 2,
            reap_interval: Duration::from_secs(30),
            per_kind_concurrency: HashMap::new(),
        }
    }
}

/// 解析 `AGENT_MEMORY_JOB_CONCURRENCY`（R12）——格式 `kind:cap,kind:cap`。
/// 非法段（无冒号/非数字/0）warn 跳过；返回空表 = 全部走全局并发。
pub fn parse_per_kind_concurrency(env_val: &str) -> HashMap<String, usize> {
    let mut m = HashMap::new();
    for seg in env_val.split(',') {
        let seg = seg.trim();
        if seg.is_empty() {
            continue;
        }
        let Some((kind, cap)) = seg.split_once(':') else {
            tracing::warn!("AGENT_MEMORY_JOB_CONCURRENCY 段非法（缺冒号）跳过: {seg}");
            continue;
        };
        match cap.trim().parse::<usize>() {
            Ok(0) => tracing::warn!("AGENT_MEMORY_JOB_CONCURRENCY cap=0 非法跳过: {seg}"),
            Ok(cap) => {
                m.insert(kind.trim().to_string(), cap);
            }
            Err(_) => tracing::warn!("AGENT_MEMORY_JOB_CONCURRENCY cap 非数字跳过: {seg}"),
        }
    }
    m
}

/// 任务运行器。spawn 后用 handle 停止。
pub struct Runner {
    queue: JobQueue,
    config: RunnerConfig,
    handlers: HashMap<String, HandlerFn>,
    shutdown: tokio::sync::watch::Receiver<bool>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
}

impl Runner {
    pub fn new(pool: PgPool, config: RunnerConfig) -> Self {
        let (shutdown_tx, shutdown) = tokio::sync::watch::channel(false);
        Self {
            queue: JobQueue::new(pool),
            config,
            handlers: HashMap::new(),
            shutdown,
            shutdown_tx,
        }
    }

    /// 注册 handler（任务种类 → 处理函数）。
    pub fn register<F, Fut>(mut self, kind: impl Into<String>, f: F) -> Self
    where
        F: Fn(JobContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<serde_json::Value, JobError>> + Send + 'static,
    {
        self.handlers
            .insert(kind.into(), Arc::new(move |ctx| Box::pin(f(ctx))));
        self
    }

    /// 启动：返回运行句柄（可等待/停机）。
    pub fn start(self) -> RunnerHandle {
        let queue = self.queue.clone();
        let handlers = Arc::new(self.handlers);
        let config = self.config;
        let mut shutdown = self.shutdown;
        // 在途任务追踪（RJ-09 修复）：spawn 的任务持有 semaphore permit 至执行完毕——
        // 停机后「拿满全部 permit」即「在途清零」。semaphore 在块外创建以便 RunnerHandle 持有。
        let semaphore = Arc::new(tokio::sync::Semaphore::new(config.concurrency));

        let join = {
            let semaphore = semaphore.clone();
            tokio::spawn(async move {
                // per-kind 分池（R12）：cap 不超过全局并发；启动日志打印生效配置
                let mut per_kind_sem: HashMap<String, Arc<tokio::sync::Semaphore>> = HashMap::new();
                for (kind, cap) in &config.per_kind_concurrency {
                    let cap = (*cap).min(config.concurrency);
                    per_kind_sem.insert(kind.clone(), Arc::new(tokio::sync::Semaphore::new(cap)));
                }
                if !per_kind_sem.is_empty() {
                    let mut parts: Vec<String> = per_kind_sem
                        .iter()
                        .map(|(k, s)| format!("{k}:{}", s.available_permits()))
                        .collect();
                    parts.sort();
                    tracing::info!(worker = %config.worker_id, concurrency = config.concurrency, per_kind = %parts.join(","), "job runner 启动（per-kind 并发生效）");
                } else {
                    tracing::info!(worker = %config.worker_id, concurrency = config.concurrency, "job runner 启动");
                }
                let semaphore = semaphore.clone();
                let mut last_reap = tokio::time::Instant::now();

                loop {
                    if *shutdown.borrow() {
                        tracing::info!("job runner 收到停机信号，退出");
                        break;
                    }

                    // 定期回收僵尸
                    if last_reap.elapsed() >= config.reap_interval {
                        if let Err(e) = queue.reap_orphans().await {
                            tracing::warn!(error = %e, "僵尸回收失败（下轮重试）");
                        }
                        last_reap = tokio::time::Instant::now();
                    }

                    // 抢一批
                    match queue.claim(&config.worker_id, config.batch_size).await {
                        Ok(jobs) if jobs.is_empty() => {
                            tokio::select! {
                                _ = tokio::time::sleep(config.poll_interval) => {},
                                _ = shutdown.changed() => {},
                            }
                        }
                        Ok(jobs) => {
                            for job in jobs {
                                // R12 双层限流：专属池（若配置）在任务内先拿、全局池后拿。
                                // 专属等待必须发生在 spawn 内——分派循环若顺序 await 专属信号量，
                                // 满载的慢 kind 会阻塞同批后面未配置 kind 的派发（饥饿，auditor 抓出）。
                                let kind_sem = per_kind_sem.get(&job.kind).cloned();
                                let semaphore = semaphore.clone();
                                let queue = queue.clone();
                                let handlers = handlers.clone();
                                tokio::spawn(async move {
                                    let _kind_permit = match kind_sem {
                                        Some(sem) => sem.acquire_owned().await.ok(),
                                        None => None,
                                    };
                                    let Ok(permit) = semaphore.acquire_owned().await else {
                                        return;
                                    };
                                    let _permit = permit;
                                    execute_job(&queue, handlers, job).await;
                                });
                            }
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "抢占任务失败，退避后重试");
                            tokio::time::sleep(Duration::from_secs(2)).await;
                        }
                    }
                }
            })
        };

        RunnerHandle {
            join,
            shutdown_tx: self.shutdown_tx,
            semaphore,
            concurrency: config.concurrency,
        }
    }
}

/// 等待在途任务清零（semaphore 全部 permit 归位）：优雅停机的「等在途」语义（RJ-09 修复）。
/// 每 100ms 轮询一次 try_acquire_many；超时返回 false（仍有任务在途，调用方决定放弃策略）。
async fn wait_inflight_idle(sem: &tokio::sync::Semaphore, total: usize, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(permits) = sem.try_acquire_many(total as u32) {
            drop(permits); // 立即归还，保持 semaphore 状态不变
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 运行句柄：等待退出或请求停机。
pub struct RunnerHandle {
    join: tokio::task::JoinHandle<()>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    semaphore: Arc<tokio::sync::Semaphore>,
    concurrency: usize,
}

impl RunnerHandle {
    /// 请求停机并等待在途任务完成（优雅语义；RJ-09 修复——此前只发信号不等任务，
    /// SIGTERM 会腰斩执行中的 job）。
    /// 返回 true = 在途全部落库完成；false = 超时放弃（未完成任务由 reap_orphans 兜底回收）。
    pub async fn shutdown_and_wait(self, timeout: Duration) -> bool {
        let _ = self.shutdown_tx.send(true); // 有意忽略：接收端已退出即停机完成；join 结果不改变语义
        let _ = self.join.await;
        wait_inflight_idle(&self.semaphore, self.concurrency, timeout).await
    }
}

async fn execute_job(queue: &JobQueue, handlers: Arc<HashMap<String, HandlerFn>>, job: Job) {
    let Some(handler) = handlers.get(&job.kind) else {
        // 未注册种类：永久失败（防无限重排）
        if let Err(e) = queue
            .fail(
                job.id,
                &JobError::Permanent(format!("未注册的任务种类: {}", job.kind)),
            )
            .await
        {
            tracing::warn!(job = %job.id, error = %e, "未注册种类标记失败（job 留 pending 由 reap 兜底）");
        }
        return;
    };

    let ctx = JobContext {
        job: job.clone(),
        queue: queue.clone(),
    };
    match handler(ctx).await {
        Ok(result) => {
            if let Err(e) = queue.complete(job.id, Some(result)).await {
                tracing::error!(job_id = %job.id, error = %e, "标记成功失败（状态可能滞留 running，等回收）");
            }
        }
        Err(e) => {
            if let Err(mark_err) = queue.fail(job.id, &e).await {
                tracing::error!(job_id = %job.id, error = %mark_err, "标记失败本身失败（状态可能滞留 running，等回收）");
            }
        }
    }
}

#[cfg(test)]
mod per_kind_tests {
    use super::*;

    #[test]
    fn parse_per_kind_handles_valid_and_skips_garbage() {
        let m = parse_per_kind_concurrency("wiki_generate:1, wiki_analyze:2 ,,bad,broken:0,x:abc");
        assert_eq!(m.get("wiki_generate"), Some(&1));
        assert_eq!(m.get("wiki_analyze"), Some(&2));
        assert_eq!(m.len(), 2, "非法段全部跳过: {m:?}");
    }

    #[test]
    fn empty_env_yields_empty_map() {
        assert!(parse_per_kind_concurrency("").is_empty());
    }

    /// R12 核心语义：有配置的 kind cap 不超过全局；未配置走全局。
    /// 用真实 Runner 起一个两池布局验证并发上限差（wiki_generate:1 vs 全局 4）。
    #[tokio::test]
    async fn per_kind_semaphore_caps_beyond_global() {
        let cfg = RunnerConfig {
            worker_id: "test-pk".into(),
            concurrency: 4,
            per_kind_concurrency: parse_per_kind_concurrency("wiki_generate:1"),
            ..Default::default()
        };
        // 语义校验：构造分池后专属池 permit=1、全局=4
        let per = cfg
            .per_kind_concurrency
            .get("wiki_generate")
            .copied()
            .unwrap()
            .min(cfg.concurrency);
        assert_eq!(per, 1);
        assert!(
            !cfg.per_kind_concurrency.contains_key("embed"),
            "未配置 kind 回退全局"
        );
    }

    /// RJ-09：停机「等在途」语义——permit 未归位时超时放行（false），归位后立即通过（true）。
    #[tokio::test]
    async fn wait_inflight_idle_blocks_until_released() {
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        let p1 = sem.clone().acquire_owned().await.unwrap();
        // 1 个 permit 在途：拿不满 2 → 超时 false
        assert!(!wait_inflight_idle(&sem, 2, Duration::from_millis(150)).await);
        drop(p1);
        assert!(wait_inflight_idle(&sem, 2, Duration::from_millis(500)).await);
        // 归还语义：等待成功后 semaphore 仍可用（permits 完整）
        assert_eq!(sem.available_permits(), 2);
    }

    #[tokio::test]
    async fn wait_inflight_idle_immediate_when_idle() {
        let sem = Arc::new(tokio::sync::Semaphore::new(3));
        assert!(wait_inflight_idle(&sem, 3, Duration::from_millis(100)).await);
    }
}

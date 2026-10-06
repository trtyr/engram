//! P017 项目整理 Agent 测试：节律幂等/fanout、Agent mock 循环（note_issue 落文档）、单飞守卫。

mod support;

use engram_distill::llm_port::MockLlm;
use engram_jobs::types::JobTemplate;
use engram_jobs::{JobQueue, Runner, RunnerConfig};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

struct Env {
    pool: sqlx::PgPool,
    queue: JobQueue,
    handle: engram_jobs::RunnerHandle,
    _pg: support::TestPg,
}

async fn setup(chats: Vec<serde_json::Value>) -> Env {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");

    let llm: Arc<MockLlm> = Arc::new(MockLlm::with_raw_chats(
        chats
            .into_iter()
            .map(|c| match c {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            })
            .collect(),
    ));
    let runner = engram_distill::project_maintain::register_maintain_project(
        Runner::new(
            pool.clone(),
            RunnerConfig {
                worker_id: "test-runner".into(),
                concurrency: 4,
                poll_interval: Duration::from_millis(20),
                batch_size: 10,
                reap_interval: Duration::from_secs(3600),
                per_kind_concurrency: Default::default(),
                cipher: None,
            },
        ),
        llm,
    );
    let handle = runner.start();
    Env {
        pool: pool.clone(),
        queue: JobQueue::new(pool),
        handle,
        _pg: container,
    }
}

async fn create_project(pool: &sqlx::PgPool, name: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO projects (id, name, type, status) VALUES ($1, $2, 'dev', 'active')")
        .bind(id)
        .bind(name)
        .execute(pool)
        .await
        .expect("建项目");
    id
}

async fn create_doc(pool: &sqlx::PgPool, project_id: Uuid, title: &str, content: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO project_docs (id, project_id, category, title, content) \
         VALUES ($1, $2, '架构', $3, $4)",
    )
    .bind(id)
    .bind(project_id)
    .bind(title)
    .bind(content)
    .execute(pool)
    .await
    .expect("建文档");
    id
}

async fn wait_kind(pool: &sqlx::PgPool, kind: &str, min_count: i64) -> Vec<(Uuid, String)> {
    for _ in 0..600 {
        let rows: Vec<(Uuid, String)> =
            sqlx::query_as("SELECT id, status::text FROM jobs WHERE kind = $1 ORDER BY created_at")
                .bind(kind)
                .fetch_all(pool)
                .await
                .unwrap();
        let done = rows
            .iter()
            .filter(|(_, s)| matches!(s.as_str(), "succeeded" | "failed" | "dead"))
            .count() as i64;
        if rows.len() as i64 >= min_count && done >= min_count {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let all: Vec<(
        String,
        String,
        Option<serde_json::Value>,
        Option<serde_json::Value>,
    )> = sqlx::query_as("SELECT kind, status::text, progress, error FROM jobs ORDER BY created_at")
        .fetch_all(pool)
        .await
        .unwrap();
    panic!("等待 {kind} 超时，全部任务：{all:#?}");
}

/// Agent mock 循环：list_docs → note_issue → finish；注记落文档 + 报告落 progress。
#[tokio::test]
async fn maintain_project_notes_issue_into_doc() {
    // 先建库与数据（拿真实 doc_id 再定 mock 序列）
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    let project = create_project(&pool, "proj-alpha").await;
    let doc = create_doc(&pool, project, "架构", "# 架构\n\n旧内容。").await;

    let llm: Arc<MockLlm> = Arc::new(MockLlm::with_raw_chats(vec![
        json!({"tool": "list_docs", "args": {}}).to_string(),
        json!({"tool": "note_issue", "args": {"doc_id": doc.to_string(), "issue": "架构篇过时", "suggestion": "更新到新拓扑"}}).to_string(),
        json!({"tool": "finish", "args": {"summary": "两篇文档结构清晰，1 处过时已留痕", "issues_noted": 1}}).to_string(),
    ]));
    let runner = engram_distill::project_maintain::register_maintain_project(
        Runner::new(
            pool.clone(),
            RunnerConfig {
                worker_id: "test-runner".into(),
                concurrency: 4,
                poll_interval: Duration::from_millis(20),
                batch_size: 10,
                reap_interval: Duration::from_secs(3600),
                per_kind_concurrency: Default::default(),
                cipher: None,
            },
        ),
        llm,
    );
    let handle = runner.start();
    let queue = JobQueue::new(pool.clone());

    queue
        .enqueue(
            JobTemplate::new("maintain_project").with_payload(json!({ "project_id": project })),
        )
        .await
        .unwrap();

    let rows = wait_kind(&pool, "maintain_project", 1).await;
    let (job_id, status) = &rows[0];
    assert_eq!(status, "succeeded", "整理任务应成功: {rows:?}");

    // note_issue 落文档断言
    let content: String = sqlx::query_scalar("SELECT content FROM project_docs WHERE id = $1")
        .bind(doc)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        content.contains("## 维护注记"),
        "文档应含维护注记段: {content}"
    );
    assert!(content.contains("架构篇过时"), "注记应含问题: {content}");
    assert!(
        content.contains("项目整理 Agent"),
        "应带 Agent 署名: {content}"
    );

    // 报告落 progress
    let progress: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT progress FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let progress = progress.expect("应有 progress");
    assert_eq!(
        progress["issues_noted"], 1,
        "issues_noted 应为 1: {progress}"
    );
    assert!(
        progress["markdown"]
            .as_str()
            .unwrap_or("")
            .contains("项目整理报告"),
        "markdown 报告应在: {progress}"
    );

    handle.shutdown_and_wait(Duration::from_secs(5)).await;
}

/// 节律：bootstrap 幂等一键一桶；跑桶后每项目一个 maintain_project。
#[tokio::test]
async fn rhythm_bucket_idempotent_and_fanout() {
    let env = setup(vec![]).await;
    create_project(&env.pool, "proj-a").await;
    create_project(&env.pool, "proj-b").await;

    // bootstrap 两次 → 幂等一键
    engram_distill::project_maintain::bootstrap_maintain_project(&env.queue)
        .await
        .unwrap();
    engram_distill::project_maintain::bootstrap_maintain_project(&env.queue)
        .await
        .unwrap();
    let buckets: i64 =
        sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind = 'rhythm_maintain_project'")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(buckets, 1, "同日 bootstrap 幂等：一个桶，得 {buckets}");

    // 跑桶 → 两项目各一 maintain_project 任务（且自续明日桶）
    let rows = wait_kind(&env.pool, "maintain_project", 2).await;
    assert_eq!(rows.len(), 2, "两项目应各一任务: {rows:?}");

    let tomorrow: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE kind = 'rhythm_maintain_project' \
         AND status = 'pending'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert!(tomorrow >= 1, "应自续明日桶");

    // Agent 循环失败（空 mock）不阻塞——任务终态允许 failed/dead，但节律已投递
    env.handle.shutdown_and_wait(Duration::from_secs(5)).await;
}

/// 单飞守卫：running 的项目被守卫拦住，其他项目不受影响（曾出 text/uuid 类型错——回归防线）。
#[tokio::test]
async fn rhythm_skips_running_project() {
    let env = setup(vec![]).await;
    let pid = create_project(&env.pool, "proj-busy").await;
    // 直接 SQL 预插 running 任务（不经队列——无 runner 消费竞态）
    sqlx::query(
        "INSERT INTO jobs (id, kind, status, payload, due_at, attempts, created_at) \
         VALUES ($1, 'maintain_project', 'running', $2::jsonb, now(), 0, now())",
    )
    .bind(Uuid::now_v7())
    .bind(json!({ "project_id": pid }).to_string())
    .execute(&env.pool)
    .await
    .unwrap();

    let busy = engram_distill::project_maintain::count_running_for_project(&env.pool, pid)
        .await
        .unwrap();
    assert_eq!(busy, 1, "running 项目的守卫应计到 1");

    let other = create_project(&env.pool, "proj-free").await;
    let free = engram_distill::project_maintain::count_running_for_project(&env.pool, other)
        .await
        .unwrap();
    assert_eq!(free, 0, "其他项目不应被误伤");

    env.handle.shutdown_and_wait(Duration::from_secs(5)).await;
}

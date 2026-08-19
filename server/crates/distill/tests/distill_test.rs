//! 蒸馏链 mock e2e：进程内 Runner + MockLlm。
//! Phase 2 出口 mock 部分：解析重试 / 仲裁三分支 / 画像版本化 / 引用链。

mod support;

use agent_memory_distill::llm_port::MockLlm;
use agent_memory_distill::register_handlers;
use agent_memory_jobs::types::{JobStatus, JobTemplate};
use agent_memory_jobs::{JobQueue, Runner, RunnerConfig};
use pgvector::Vector;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

struct Env {
    pool: sqlx::PgPool,
    queue: JobQueue,
    handle: agent_memory_jobs::RunnerHandle,
}

async fn setup(chats: Vec<serde_json::Value>) -> Env {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");
    std::mem::forget(container);

    let llm: Arc<MockLlm> = Arc::new(MockLlm::with_raw_chats(
        chats
            .into_iter()
            .map(|c| match c {
                serde_json::Value::String(s) => s, // 原始文本（可非法）
                other => other.to_string(),
            })
            .collect(),
    ));
    let runner = register_handlers(
        Runner::new(
            pool.clone(),
            RunnerConfig {
                worker_id: "test-runner".into(),
                concurrency: 4,
                poll_interval: Duration::from_millis(20),
                batch_size: 10,
                reap_interval: Duration::from_secs(3600),
            },
        ),
        llm,
    );
    let handle = runner.start();
    Env {
        pool: pool.clone(),
        queue: JobQueue::new(pool),
        handle,
    }
}

async fn wait_done(queue: &JobQueue, kind: &str) -> agent_memory_jobs::Job {
    for _ in 0..300 {
        let jobs = queue
            .list(&[kind.to_string()], &[], None, 50)
            .await
            .unwrap();
        if let Some(j) = jobs.iter().find(|j| {
            matches!(
                j.status,
                JobStatus::Succeeded | JobStatus::Failed | JobStatus::Dead
            )
        }) {
            return j.clone();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("job {kind} 超时未完成");
}

fn session(turns: &[(&str, &str)]) -> serde_json::Value {
    json!(
        turns
            .iter()
            .map(|(s, t)| json!({"speaker": s, "text": t}))
            .collect::<Vec<_>>()
    )
}

fn emb(seed: usize) -> Vector {
    Vector::from(
        (0..1024)
            .map(|i| ((seed * 31 + i) % 17) as f32 / 17.0)
            .collect::<Vec<f32>>(),
    )
}

/// L0 → extract（解析重试）→ arbitrate（兜底转正）→ organize（空动作收尾）。
/// 验证：会话 done、候选 active、embedding/tsv/source_refs 齐全。
#[tokio::test]
async fn extract_with_retry_and_full_refs() {
    let env = setup(vec![
        serde_json::Value::String("抱歉这不是 JSON".into()), // 第一次：非法 → 重试
        json!({"atoms": [
            {"kind": "fact", "content": "用户用 Mac 开发", "confidence": 0.9, "turn_refs": [1]},
            {"kind": "preference", "content": "用户喜欢暗色主题", "confidence": 0.5, "turn_refs": [1]},
        ]}),
        // arbitrate：占位 id 不命中 → 兜底转正
        json!({"verdicts": [{"candidate_id": "00000000-0000-0000-0000-000000000000", "disposition": "duplicate"}]}),
        // organize：空动作（链收尾）
        json!({"actions": []}),
    ])
    .await;

    let sid = Uuid::now_v7();
    sqlx::query("INSERT INTO raw_sessions (id, agent, content) VALUES ($1, 'pi', $2)")
        .bind(sid)
        .bind(session(&[("user", "我平时用 Mac 写代码，主题用暗色")]))
        .execute(&env.pool)
        .await
        .unwrap();

    env.queue
        .enqueue(JobTemplate::new("extract_atoms"))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "extract_atoms").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "解析重试后应成功: {:?}",
        j.error
    );
    let j2 = wait_done(&env.queue, "arbitrate_atoms").await;
    assert_eq!(
        j2.status,
        JobStatus::Succeeded,
        "占位裁决走兜底: {:?}",
        j2.error
    );
    let j3 = wait_done(&env.queue, "organize_scenarios").await;
    assert_eq!(j3.status, JobStatus::Succeeded);

    // 会话 done
    let st: String = sqlx::query_scalar("SELECT distill_status FROM raw_sessions WHERE id = $1")
        .bind(sid)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(st, "done");

    // 两条候选：active、低置信标 needs_review、embedding+tsv+source_refs
    let rows: Vec<(String, String, bool, bool, bool, serde_json::Value)> = sqlx::query_as(
        "SELECT content, status, needs_review, embedding IS NOT NULL, tsv IS NOT NULL, source_refs \
         FROM atoms ORDER BY created_at",
    )
    .fetch_all(&env.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    let (mac, dark) = (&rows[0], &rows[1]);
    assert_eq!(mac.1, "active");
    assert!(!mac.2, "高置信不需人审");
    assert!(mac.3 && mac.4, "embedding 与 tsv 都应生成");
    assert!(
        mac.5.to_string().contains(&sid.to_string()),
        "source_refs 指向 L0: {}",
        mac.5
    );
    assert!(dark.2, "confidence 0.5 应标 needs_review");

    env.handle.shutdown();
    env.handle.join().await;
}

/// 仲裁三分支（真实 id）+ organize 归组 + persona 版本化：一条龙。
#[tokio::test]
async fn arbitrate_branches_organize_and_persona_history() {
    // 三条候选（真实 id）
    let c_new = Uuid::now_v7();
    let c_dup = Uuid::now_v7();
    let c_con = Uuid::now_v7();
    // 既有：dup 靶子 + con 靶子
    let t_dup = Uuid::now_v7();
    let t_con = Uuid::now_v7();

    let env = setup(vec![
        // arbitrate：三分支
        json!({"verdicts": [
            {"candidate_id": c_new.to_string(), "disposition": "new"},
            {"candidate_id": c_dup.to_string(), "disposition": "duplicate", "target_id": t_dup.to_string()},
            {"candidate_id": c_con.to_string(), "disposition": "contradicts", "target_id": t_con.to_string()},
        ]}),
        // organize：建场景收编 new + con
        json!({"actions": [
            {"action": "create", "topic": "居住地", "summary": "用户移居深圳", "body": "用户已从广州搬到深圳定居。", "atom_ids": [c_new.to_string(), c_con.to_string()]},
        ]}),
        // persona v1
        json!({"aspects": [
            {"aspect": "identity", "content": "用户现居深圳。"},
        ]}),
        // persona v2（手动再跑）
        json!({"aspects": [
            {"aspect": "identity", "content": "用户现居深圳，在广州生活过多年。"},
        ]}),
    ])
    .await;

    for (id, content, seed) in [
        (c_new, "用户会写 Rust", 1),
        (c_dup, "用户偏好简短回复", 2),
        (c_con, "用户现在定居深圳", 3),
    ] {
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, status, confidence, embedding, tsv) \
             VALUES ($1, 'fact', $2, 'candidate', 0.9, $3, to_tsvector('simple', $4))",
        )
        .bind(id)
        .bind(content)
        .bind(emb(seed))
        .bind(agent_memory_search::tokenize::tsv_text(content))
        .execute(&env.pool)
        .await
        .unwrap();
    }
    for (id, content, seed) in [(t_dup, "用户喜欢简短的回答", 4), (t_con, "用户住在广州", 5)]
    {
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, status, confidence, embedding, tsv, hit_count) \
             VALUES ($1, 'fact', $2, 'active', 0.9, $3, to_tsvector('simple', $4), 2)",
        )
        .bind(id)
        .bind(content)
        .bind(emb(seed))
        .bind(agent_memory_search::tokenize::tsv_text(content))
        .execute(&env.pool)
        .await
        .unwrap();
    }

    // 入队 arbitrate（payload 指定候选）
    env.queue
        .enqueue(
            JobTemplate::new("arbitrate_atoms")
                .with_payload(json!({"candidate_ids": [c_new, c_dup, c_con]})),
        )
        .await
        .unwrap();

    for kind in ["arbitrate_atoms", "organize_scenarios", "distill_persona"] {
        let j = wait_done(&env.queue, kind).await;
        assert_eq!(j.status, JobStatus::Succeeded, "{kind} 失败: {:?}", j.error);
    }

    // 三分支断言
    // new → active
    let (s,): (String,) = sqlx::query_as("SELECT status FROM atoms WHERE id = $1")
        .bind(c_new)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(s, "active");
    // duplicate → 候选删除 + 靶子 hit_count+1
    let cnt: i64 = sqlx::query_scalar("SELECT count(*) FROM atoms WHERE id = $1")
        .bind(c_dup)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(cnt, 0, "duplicate 候选应删除");
    let hits: i32 = sqlx::query_scalar("SELECT hit_count FROM atoms WHERE id = $1")
        .bind(t_dup)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(hits, 3, "靶子 hit_count 2→3");
    // contradicts → 旧 superseded + 链到新
    let (old_status, sup_by): (String, Option<Uuid>) =
        sqlx::query_as("SELECT status, superseded_by FROM atoms WHERE id = $1")
            .bind(t_con)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(old_status, "superseded");
    assert_eq!(sup_by, Some(c_con));

    // organize：场景 + 归组
    let (topic, refs, sid): (String, serde_json::Value, Uuid) =
        sqlx::query_as("SELECT topic, atom_refs, id FROM scenarios")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(topic, "居住地");
    assert_eq!(refs.as_array().map(|a| a.len()), Some(2));
    let grouped: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM atoms WHERE scenario_id IS NOT NULL AND status = 'active'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(grouped, 2, "new+con 应归组");

    // persona v1
    let (v1, ver): (String, i32) = sqlx::query_as(
        "SELECT content, version FROM persona_aspects WHERE aspect = 'identity' ORDER BY version DESC LIMIT 1",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert!(v1.contains("深圳"));
    assert_eq!(ver, 1);

    // 手动再跑 persona → v2 + history + evidence
    env.queue
        .enqueue(JobTemplate::new("distill_persona").with_payload(json!({"scenario_ids": [sid]})))
        .await
        .unwrap();
    wait_done(&env.queue, "distill_persona").await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let history: Vec<(i32, String)> = sqlx::query_as(
        "SELECT version, content FROM persona_aspects WHERE aspect = 'identity' ORDER BY version",
    )
    .fetch_all(&env.pool)
    .await
    .unwrap();
    assert_eq!(history.len(), 2, "两个版本: {history:?}");
    assert!(history[1].1.contains("广州"));
    let evidence: serde_json::Value = sqlx::query_scalar(
        "SELECT evidence_refs FROM persona_aspects WHERE aspect = 'identity' AND version = 2",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert!(
        evidence.to_string().contains(&sid.to_string()),
        "evidence 指向场景: {evidence}"
    );

    env.handle.shutdown();
    env.handle.join().await;
}

/// 防抖：同窗口两次触发复用同一任务；到期执行取走全部 pending 会话。
#[tokio::test]
async fn debounce_bucket_shares_job() {
    let env = setup(vec![
        json!({"atoms": []}),
        json!({"verdicts": []}),
        json!({"actions": []}),
    ])
    .await;

    let j1 = agent_memory_distill::trigger_auto_extract(&env.queue, 30)
        .await
        .unwrap();
    let j2 = agent_memory_distill::trigger_auto_extract(&env.queue, 30)
        .await
        .unwrap();
    assert_eq!(j1.id, j2.id, "同 30s 窗口应复用任务");
    assert!(j1.due_at > chrono::Utc::now(), "防抖应延迟执行");

    // 立即手动入队另一个 extract（不同键）→ 两个任务并存
    let j3 = env
        .queue
        .enqueue(JobTemplate::new("extract_atoms").with_payload(json!({"reason": "manual"})))
        .await
        .unwrap();
    assert_ne!(j1.id, j3.id);

    env.handle.shutdown();
    env.handle.join().await;
}

//! P012-T004：新旧 organize 对齐评测（ignored——真 LLM，需 ENGRAM_EVAL_KEY）。
//!
//! 跑法：
//! ```bash
//! ENGRAM_EVAL_KEY=sk-or-... cargo test -p engram-distill \
//!   --test eval_align -- --ignored --nocapture
//! ```
//!
//! 设计：同一批 40 条生产真实原子（fixtures/eval_atoms.json，全部拆散 scenario_id=NULL）
//! 注入两个独立 TestPg，分别以旧单发路径与 agentic 循环组织（同一 OpenRouter 模型），
//! 产出对照：场景数/成员归属率/topic 清单/LLM 调用数/耗时。
//! 注：OpenRouter 无 embedding 端点——场景检索走 tsv 腿，embedding 留空（生产部署后
//! 含向量腿复测）。key 只走 env，不落代码/日志。

mod support;

use serde_json::json;
use std::time::Instant;
use support::{connect_with_retry, connection_url, start_pgvector};

const MODEL: &str = "qwen/qwen3.8-27b:free";

struct RunOutcome {
    scenario_count: i64,
    assigned_atoms: i64,
    topics: Vec<String>,
    llm_calls: i64,
    elapsed: std::time::Duration,
    status: String,
}

async fn run_one(agentic: bool, key: &str) -> (sqlx::PgPool, RunOutcome, support::TestPg) {
    let container = start_pgvector().await.expect("容器");
    let url = connection_url(&container).await.unwrap();
    let pool = connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");

    // 注入真实原子（全部拆散）
    let raw = std::fs::read_to_string("tests/fixtures/eval_atoms.json").expect("fixtures");
    let atoms: serde_json::Value = serde_json::from_str(&raw).expect("fixtures json");
    for a in atoms.as_array().unwrap() {
        let id: uuid::Uuid = a["id"].as_str().unwrap().parse().unwrap();
        let kind = a["kind"].as_str().unwrap();
        let content = a["content"].as_str().unwrap();
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, status, confidence, tsv) \
             VALUES ($1, $2, $3, 'active', 0.9, to_tsvector('simple', $3))",
        )
        .bind(id)
        .bind(kind)
        .bind(content)
        .execute(&pool)
        .await
        .unwrap();
    }

    // LLM provider：OpenRouter chat（key 走 env → cipher 加密落库）
    // 0022 单行制：model_id + capability；OpenRouter 无 embedding——不插 embedding 行，
    // embed resolve 失败由 refresh/scenarios_search 的降级容忍兜底（检索走 tsv 腿）。
    let cipher =
        engram_llm::crypto::KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap();
    let enc = cipher
        .encrypt(key)
        .expect("encrypt key");
    let enc_hex: String = enc.iter().map(|b| format!("{b:02x}")).collect();
    sqlx::query(
        "INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, model_id, capability, is_default) \
         VALUES ($1, 'eval-openrouter', 'https://openrouter.ai/api/v1', decode($2,'hex'), $3, 'chat', TRUE)",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(&enc_hex)
    .bind(MODEL)
    .execute(&pool)
    .await
    .unwrap();

    // 开关
    engram_storage::repo::settings::put_json(
        &pool,
        "organize_agentic",
        &json!({"enabled": agentic}),
    )
    .await
    .unwrap();

    // 跑 organize（runner 全链：converge→主组织→persona 链；persona/chat 全走同 provider）
    let llm = engram_distill::gateway_llm(pool.clone(), cipher);
    let runner = engram_distill::register_handlers(
        engram_jobs::Runner::new(
            pool.clone(),
            engram_jobs::RunnerConfig {
                worker_id: "eval".into(),
                concurrency: 2,
                poll_interval: std::time::Duration::from_millis(50),
                batch_size: 10,
                reap_interval: std::time::Duration::from_secs(3600),
                per_kind_concurrency: Default::default(),
                cipher: None,
            },
        ),
        llm,
    );
    let handle = runner.start();
    let queue = engram_jobs::JobQueue::new(pool.clone());

    let started = Instant::now();
    queue
        .enqueue(engram_jobs::JobTemplate::new("organize_scenarios").with_payload(json!({})))
        .await
        .unwrap();
    // 轮询终态（含链式 persona；上限 10 分钟）
    let mut status = "timeout".into();
    let mut task_error: Option<String> = None;
    for _ in 0..1200 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let done: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT status::text, error FROM jobs WHERE kind = 'organize_scenarios' \
             AND status IN ('succeeded','failed','dead') ORDER BY created_at DESC LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        // persona 链完成后再取数：organize succeeded 后再等 3s 稳态
        if let Some((s, e)) = done {
            if s == "failed" || s == "dead" {
                status = s.clone();
                task_error = e;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            status = s;
            break;
        }
    }
    if let Some(e) = &task_error {
        println!("任务错误（诊断）: {e}");
    }
    let elapsed = started.elapsed();
    handle.shutdown_and_wait(std::time::Duration::from_secs(5)).await;

    // 采集指标
    let scenario_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM scenarios WHERE retired_at IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap_or(0);
    let assigned_atoms: i64 =
        sqlx::query_scalar("SELECT count(*) FROM atoms WHERE scenario_id IS NOT NULL")
            .fetch_one(&pool)
            .await
            .unwrap_or(0);
    let topics: Vec<String> = sqlx::query_scalar(
        "SELECT topic FROM scenarios WHERE retired_at IS NULL ORDER BY created_at",
    )
    .fetch_all(&pool)
    .await
    .unwrap_or_default();
    let llm_calls: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM logs WHERE target = 'job.organize_scenarios' \\
         AND message = 'LLM 调用'",
    )
    .fetch_one(&pool)
    .await
    .unwrap_or(0);

    (
        pool,
        RunOutcome {
            scenario_count,
            assigned_atoms,
            topics,
            llm_calls,
            elapsed,
            status,
        },
        container,
    )
}

#[tokio::test]
#[ignore = "真 LLM 评测：ENGRAM_EVAL_KEY=... cargo test --test eval_align -- --ignored"]
async fn eval_old_vs_agentic() {
    let key = std::env::var("ENGRAM_EVAL_KEY").expect("ENGRAM_EVAL_KEY 未设置");

    // T004 评测注：免费模型限速严（~20 req/min）——agentic 先跑（步数多更怕限流），
    // 旧版后跑；顺序效应在报告中注明。
    println!("\n===== agentic 循环（organize_agentic=true，先跑）=====");
    let (_p_new, new_out, _c2) = run_one(true, &key).await;
    println!(
        "status={} 场景数={} 归属原子={}/40 LLM调用={} 耗时={:.1}s\ntopics: {:?}",
        new_out.status,
        new_out.scenario_count,
        new_out.assigned_atoms,
        new_out.llm_calls,
        new_out.elapsed.as_secs_f32(),
        new_out.topics
    );

    println!("\n===== 旧单发路径（organize_agentic=false，后跑）=====");
    let (_p_old, old_out, _c1) = run_one(false, &key).await;
    println!(
        "status={} 场景数={} 归属原子={}/40 LLM调用={} 耗时={:.1}s\ntopics: {:?}",
        new_out.status,
        new_out.scenario_count,
        new_out.assigned_atoms,
        new_out.llm_calls,
        new_out.elapsed.as_secs_f32(),
        new_out.topics
    );

    // 对照表（markdown，人工评审用）
    println!(
        "\n| 指标 | 旧单发 | agentic |\n|---|---|---|\n| 状态 | {} | {} |\n| 场景数 | {} | {} |\n| 归属原子 | {}/40 | {}/40 |\n| LLM 调用 | {} | {} |\n| 耗时 | {:.1}s | {:.1}s |",
        old_out.status,
        new_out.status,
        old_out.scenario_count,
        new_out.scenario_count,
        old_out.assigned_atoms,
        new_out.assigned_atoms,
        old_out.llm_calls,
        new_out.llm_calls,
        old_out.elapsed.as_secs_f32(),
        new_out.elapsed.as_secs_f32(),
    );
}

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
    _pg: support::TestPg,
}

async fn setup(chats: Vec<serde_json::Value>) -> Env {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");

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
        _pg: container,
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
    // duplicate → 候选归档保留（B6：不再物理删除）+ superseded_by 溯源 + 靶子 hit_count+1
    let (dup_status, dup_sup): (String, Option<Uuid>) =
        sqlx::query_as("SELECT status, superseded_by FROM atoms WHERE id = $1")
            .bind(c_dup)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(dup_status, "archived", "duplicate 候选应归档保留（可审计）");
    assert_eq!(dup_sup, Some(t_dup), "归档候选 superseded_by 指向既有条");
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
    // 轮询 DB 等 v2 落库（wait_done 可能匹配到第一次的终态 job，不可靠——既有 flaky 根因）
    let mut history: Vec<(i32, String)> = Vec::new();
    for _ in 0..100 {
        history = sqlx::query_as(
            "SELECT version, content FROM persona_aspects WHERE aspect = 'identity' ORDER BY version",
        )
        .fetch_all(&env.pool)
        .await
        .unwrap();
        if history.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
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

/// B3：分面级证据链——不同分面 evidence_refs 只含各自依据的场景，并打通到 L0 会话。
#[tokio::test]
async fn persona_evidence_per_aspect() {
    let s_home = Uuid::now_v7(); // S1：居住
    let s_skill = Uuid::now_v7(); // S2：技能
    let a_home = Uuid::now_v7(); // S1 的原子
    let a_skill = Uuid::now_v7(); // S2 的原子
    let sess_home = Uuid::now_v7(); // L0：居住信息来源会话
    let sess_skill = Uuid::now_v7(); // L0：技能信息来源会话

    let env = setup(vec![
        // persona：两个分面各标注自己的依据场景
        json!({"aspects": [
            {"aspect": "identity", "content": "用户现居深圳。", "evidence_scenarios": ["S1"]},
            {"aspect": "skills", "content": "用户会写 Rust。", "evidence_scenarios": ["S2"]},
        ]}),
    ])
    .await;

    for (sid, topic, aid, content, sess) in [
        (s_home, "居住地", a_home, "用户现居深圳", sess_home),
        (s_skill, "技能栈", a_skill, "用户会写 Rust", sess_skill),
    ] {
        sqlx::query(
            "INSERT INTO scenarios (id, topic, summary, body, atom_refs) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(sid)
        .bind(topic)
        .bind(content)
        .bind(content)
        .bind(sqlx::types::Json(vec![aid]))
        .execute(&env.pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO atoms (id, kind, content, status, confidence, source_refs) \
             VALUES ($1, 'fact', $2, 'active', 0.9, $3)",
        )
        .bind(aid)
        .bind(content)
        .bind(sqlx::types::Json(vec![
            serde_json::json!({"session_id": sess.to_string()}),
        ]))
        .execute(&env.pool)
        .await
        .unwrap();
    }

    env.queue
        .enqueue(
            JobTemplate::new("distill_persona")
                .with_payload(json!({"scenario_ids": [s_home, s_skill]})),
        )
        .await
        .unwrap();
    let j = wait_done(&env.queue, "distill_persona").await;
    assert_eq!(j.status, JobStatus::Succeeded, "{:?}", j.error);

    let rows: Vec<(String, serde_json::Value)> =
        sqlx::query_as("SELECT aspect, evidence_refs FROM persona_aspects")
            .fetch_all(&env.pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 2, "两个分面: {rows:?}");

    let ev = |aspect: &str| -> serde_json::Value {
        rows.iter()
            .find(|(a, _)| a == aspect)
            .map(|(_, e)| e.clone())
            .unwrap()
    };
    let identity = ev("identity");
    let skills = ev("skills");

    // 分面级证据：各含各的场景，不串
    let id_scen: Vec<String> = identity["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let sk_scen: Vec<String> = skills["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(id_scen, vec![s_home.to_string()], "identity 只依据居住场景");
    assert_eq!(sk_scen, vec![s_skill.to_string()], "skills 只依据技能场景");

    // L3→L2→L1→L0 全链：atoms + sessions 各归各
    assert_eq!(identity["sessions"].as_array().unwrap().len(), 1);
    assert!(
        identity["sessions"]
            .to_string()
            .contains(&sess_home.to_string())
    );
    assert!(
        skills["sessions"]
            .to_string()
            .contains(&sess_skill.to_string())
    );

    // prompt 版本落 v2
    let pv: String =
        sqlx::query_scalar("SELECT prompt_version FROM persona_aspects WHERE aspect = 'identity'")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(pv, "2", "persona prompt 升版 v2");

    env.handle.shutdown();
    env.handle.join().await;
}
#[tokio::test]
async fn arbitrate_null_embedding_falls_back_to_fts() {
    let c_no_emb = Uuid::now_v7(); // 候选：embedding NULL
    let target = Uuid::now_v7(); // 既有 active：与候选文本共享关键词（FTS 可命中），带向量

    let env = setup(vec![
        // arbitrate：对无嵌入候选给出 duplicate 判定（证明它进了 LLM 仲裁而非直通转正）
        json!({"verdicts": [
            {"candidate_id": c_no_emb.to_string(), "disposition": "duplicate", "target_id": target.to_string()},
        ]}),
        // organize：空动作收尾（无转正 → 不入队，本条仅为队列兜底）
        json!({"actions": []}),
    ])
    .await;

    // 候选无嵌入（模拟 embed 失败/上游异常的产物）
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, status, confidence, embedding, tsv) \
         VALUES ($1, 'fact', '用户喜欢简洁的中文回复', 'candidate', 0.9, NULL, to_tsvector('simple', $2))",
    )
    .bind(c_no_emb)
    .bind(agent_memory_search::tokenize::tsv_text("用户喜欢简洁的中文回复"))
    .execute(&env.pool)
    .await
    .unwrap();

    // 既有 active：语义相近（关键词重叠），有嵌入
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, status, confidence, embedding, tsv, hit_count) \
         VALUES ($1, 'preference', '用户喜欢简洁中文回答', 'active', 0.9, $2, to_tsvector('simple', $3), 1)",
    )
    .bind(target)
    .bind(emb(9))
    .bind(agent_memory_search::tokenize::tsv_text("用户喜欢简洁中文回答"))
    .execute(&env.pool)
    .await
    .unwrap();

    env.queue
        .enqueue(
            JobTemplate::new("arbitrate_atoms").with_payload(json!({"candidate_ids": [c_no_emb]})),
        )
        .await
        .unwrap();
    let j = wait_done(&env.queue, "arbitrate_atoms").await;
    assert_eq!(j.status, JobStatus::Succeeded, "{:?}", j.error);

    // B6 语义：duplicate → 归档 + superseded_by（而非物理删除）——同时证明候选
    // 真的经过 LLM 仲裁（旧代码里无嵌入候选会直通 active）
    let (status, sup_by): (String, Option<Uuid>) =
        sqlx::query_as("SELECT status, superseded_by FROM atoms WHERE id = $1")
            .bind(c_no_emb)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(
        status, "archived",
        "无嵌入候选应被仲裁（duplicate → archived）"
    );
    assert_eq!(sup_by, Some(target));
    let hits: i32 = sqlx::query_scalar("SELECT hit_count FROM atoms WHERE id = $1")
        .bind(target)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(hits, 2, "靶子 hit_count 1→2");

    env.handle.shutdown();
    env.handle.join().await;
}

/// 实体档案自动生成：consolidate 对高密度滞后实体 LLM 聚合摘要；
/// 低密度（<3 原子）与摘要新鲜的实体跳过；LLM 失败仅告警不拖垮主链。
#[tokio::test]
async fn consolidate_generates_entity_portraits() {
    // mock 队列：近重复合并无候选（原子无嵌入 → 不调 LLM）；张三档案一份；
    // 李四（LLM 故障路径）由独立用例覆盖——本用例只验证生成与跳过逻辑
    let env = setup(vec![
        json!({"summary": "张三是用户的同事，负责后端；近期与用户协作 Engram 索引层重写。"}),
    ])
    .await;

    // 张三：3 条原子、空摘要 → 应生成档案
    let zhang: Uuid = sqlx::query_scalar(
        "INSERT INTO entities (id, name, kind, summary) VALUES ($1, '张三', 'person', '') RETURNING id")
        .bind(Uuid::now_v7())
        .fetch_one(&env.pool).await.unwrap();
    for i in 0..3 {
        let aid = Uuid::now_v7();
        sqlx::query("INSERT INTO atoms (id, kind, content, confidence, status, source_refs, tsv) \
                     VALUES ($1, 'fact', $2, 0.9, 'active', '[]'::jsonb, to_tsvector('simple', $3))")
            .bind(aid)
            .bind(format!("关于张三的事实 {i}"))
            .bind(format!("zhangsan {i}"))
            .execute(&env.pool).await.unwrap();
        sqlx::query("INSERT INTO atom_entities (atom_id, entity_id) VALUES ($1, $2)")
            .bind(aid)
            .bind(zhang)
            .execute(&env.pool)
            .await
            .unwrap();
    }
    // Engram：2 条原子（低于 3 密度阈值）→ 跳过
    let eng: Uuid = sqlx::query_scalar(
        "INSERT INTO entities (id, name, kind, summary) VALUES ($1, 'Engram', 'project', '') RETURNING id")
        .bind(Uuid::now_v7())
        .fetch_one(&env.pool).await.unwrap();
    for i in 0..2 {
        let aid = Uuid::now_v7();
        sqlx::query("INSERT INTO atoms (id, kind, content, confidence, status, source_refs, tsv) \
                     VALUES ($1, 'fact', $2, 0.9, 'active', '[]'::jsonb, to_tsvector('simple', $3))")
            .bind(aid)
            .bind(format!("Engram 项目事实 {i}"))
            .bind(format!("engram {i}"))
            .execute(&env.pool).await.unwrap();
        sqlx::query("INSERT INTO atom_entities (atom_id, entity_id) VALUES ($1, $2)")
            .bind(aid)
            .bind(eng)
            .execute(&env.pool)
            .await
            .unwrap();
    }
    // 王五：摘要新鲜（updated_at 晚于所有原子）→ 跳过
    let wang: Uuid = sqlx::query_scalar(
        "INSERT INTO entities (id, name, kind, summary, updated_at) \
         VALUES ($1, '王五', 'person', '已有新鲜档案', now()) RETURNING id",
    )
    .bind(Uuid::now_v7())
    .fetch_one(&env.pool)
    .await
    .unwrap();
    let aid = Uuid::now_v7();
    sqlx::query("INSERT INTO atoms (id, kind, content, confidence, status, source_refs, tsv) \
                 VALUES ($1, 'fact', '王五旧事实', 0.9, 'active', '[]'::jsonb, to_tsvector('simple', $2))")
        .bind(aid).bind("wangwu old")
        .execute(&env.pool).await.unwrap();
    sqlx::query("INSERT INTO atom_entities (atom_id, entity_id) VALUES ($1, $2)")
        .bind(aid)
        .bind(wang)
        .execute(&env.pool)
        .await
        .unwrap();
    // 王五原子数只有 1（低于阈值，双保险跳过）——再补两条使密度=3，验证「新鲜摘要」才是跳过原因
    for i in 0..2 {
        let a2 = Uuid::now_v7();
        sqlx::query("INSERT INTO atoms (id, kind, content, confidence, status, source_refs, tsv, created_at) \
                     VALUES ($1, 'fact', $2, 0.9, 'active', '[]'::jsonb, to_tsvector('simple', $3), now() - interval '1 day')")
            .bind(a2)
            .bind(format!("王五更旧事实 {i}"))
            .bind(format!("wangwu {i}"))
            .execute(&env.pool).await.unwrap();
        sqlx::query("INSERT INTO atom_entities (atom_id, entity_id) VALUES ($1, $2)")
            .bind(a2)
            .bind(wang)
            .execute(&env.pool)
            .await
            .unwrap();
    }

    env.queue
        .enqueue(JobTemplate::new("consolidate"))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "consolidate").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "consolidate 应成功: {:?}",
        j.error
    );

    // 张三：档案已生成；Engram/王五：仍是原值
    let zhang_summary: String = sqlx::query_scalar("SELECT summary FROM entities WHERE id = $1")
        .bind(zhang)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert!(
        zhang_summary.contains("张三"),
        "张三档案应已生成：{zhang_summary}"
    );
    let eng_summary: String = sqlx::query_scalar("SELECT summary FROM entities WHERE id = $1")
        .bind(eng)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(eng_summary, "", "低密度实体应跳过");
    let wang_summary: String = sqlx::query_scalar("SELECT summary FROM entities WHERE id = $1")
        .bind(wang)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(wang_summary, "已有新鲜档案", "摘要新鲜应跳过");

    env.handle.shutdown();
    env.handle.join().await;
}

/// 实体抽取挂链（社交记忆）：atoms 带 entities 字段 → 原子、实体、关联三表齐落；
/// 同名同类活体只建一次；无实体的原子不产生关联。
#[tokio::test]
async fn extract_creates_and_links_entities() {
    let env = setup(vec![
        json!({"atoms": [
            {"kind": "fact", "content": "张三建议用户用 Rust 重写索引层", "confidence": 0.9, "turn_refs": [1],
             "entities": [{"name": "张三", "kind": "person"}, {"name": "Engram", "kind": "project"}]},
            {"kind": "fact", "content": "张三的生日是 3 月 5 日", "confidence": 0.85, "turn_refs": [1],
             "entities": [{"name": "张三", "kind": "person"}]},
            {"kind": "preference", "content": "用户喜欢暗色主题", "confidence": 0.9, "turn_refs": [1]}
        ]}),
        // arbitrate：占位 id 不命中 → 兜底转正
        json!({"verdicts": [{"candidate_id": "00000000-0000-0000-0000-000000000000", "disposition": "duplicate"}]}),
        json!({"actions": []}),
    ])
    .await;

    let sid = Uuid::now_v7();
    sqlx::query("INSERT INTO raw_sessions (id, agent, content) VALUES ($1, 'pi', $2)")
        .bind(sid)
        .bind(session(&[(
            "user",
            "和张三聊了 Engram 重构，顺便他提到生日是 3 月 5 日；我说我喜欢暗色主题",
        )]))
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
        "extract 应成功: {:?}",
        j.error
    );
    wait_done(&env.queue, "arbitrate_atoms").await;

    let atom_n: i64 = sqlx::query_scalar("SELECT count(*) FROM atoms")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(atom_n, 3);

    // 同名同类活体只建一次：张三 1 行 + Engram 1 行
    let zhang_n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM entities WHERE name = '张三' AND kind = 'person' AND merged_into IS NULL")
        .fetch_one(&env.pool).await.unwrap();
    assert_eq!(zhang_n, 1, "同名同类实体应复用单行");
    let entity_n: i64 = sqlx::query_scalar("SELECT count(*) FROM entities")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(entity_n, 2);

    // 关联：原子1→(张三,Engram) + 原子2→张三 = 3 条；原子3 无实体零关联
    let link_n: i64 = sqlx::query_scalar("SELECT count(*) FROM atom_entities")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(link_n, 3);

    // 张三密度 = 2（两条原子挂他）
    let zhang_density: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM atom_entities ae JOIN entities e ON e.id = ae.entity_id \
         WHERE e.name = '张三'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(zhang_density, 2);

    env.handle.shutdown();
    env.handle.join().await;
}

/// 记忆域重嵌：NULL 向量的原子（active）与场景批量补嵌；archived 原子不动。
#[tokio::test]
async fn reembed_memory_fills_missing_vectors() {
    // 无 chat 调用——纯 embed 路径
    let env = setup(vec![]).await;

    for (i, status) in ["active", "active", "archived"].iter().enumerate() {
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, tsv) \
             VALUES ($1, 'fact', $2, 0.9, $3, '[]'::jsonb, to_tsvector('simple', $4))",
        )
        .bind(Uuid::now_v7())
        .bind(format!("记忆事实 {i}"))
        .bind(status)
        .bind(format!("fact {i}"))
        .execute(&env.pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO scenarios (id, topic, summary, body, atom_refs, tsv) \
         VALUES ($1, '主题', '场景摘要', '正文', '[]'::jsonb, to_tsvector('simple', $2))",
    )
    .bind(Uuid::now_v7())
    .bind("scene")
    .execute(&env.pool)
    .await
    .unwrap();

    let miss: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM atoms WHERE embedding IS NULL), \
                (SELECT count(*) FROM scenarios WHERE embedding IS NULL)",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(miss, (3, 1));

    env.queue
        .enqueue(JobTemplate::new("reembed_memory"))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "reembed_memory").await;
    assert_eq!(j.status, JobStatus::Succeeded, "重嵌应成功: {:?}", j.error);

    // active 原子 2 条 + 场景 1 条补齐；archived 不动
    let after: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM atoms WHERE embedding IS NULL), \
                (SELECT count(*) FROM scenarios WHERE embedding IS NULL)",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(after, (1, 0), "archived 原子应保持无向量，其余补齐");

    env.handle.shutdown();
    env.handle.join().await;
}

/// 议题二：extract 把 LLM 解析出的相对时间（以 prompt 日期锚换算）落入 occurred_at/valid_until。
#[tokio::test]
async fn extract_carries_event_time() {
    let env = setup(vec![
        json!({"atoms": [
            {"kind": "event", "content": "用户与张三去环淀山湖骑行", "confidence": 0.9, "turn_refs": [1],
             "occurred_at": "2026-09-02", "valid_until": "2026-09-02T23:59:59Z",
             "entities": [{"name": "淀山湖", "kind": "place"}]},
            {"kind": "fact", "content": "用户偏好早上六点半出发", "confidence": 0.9, "turn_refs": [1]}
        ]}),
        json!({"verdicts": []}),
        json!({"actions": []}),
    ])
    .await;

    let sid = Uuid::now_v7();
    sqlx::query("INSERT INTO raw_sessions (id, agent, content) VALUES ($1, 'pi', $2)")
        .bind(sid)
        .bind(session(&[(
            "user",
            "下周三和张三去淀山湖骑行，早上六点半出发",
        )]))
        .execute(&env.pool)
        .await
        .unwrap();

    env.queue
        .enqueue(JobTemplate::new("extract_atoms"))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "extract_atoms").await;

    let row: (
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        "SELECT occurred_at, valid_until FROM atoms WHERE content LIKE '%淀山湖%' LIMIT 1",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(
        row.0.map(|d| d.date_naive().to_string()),
        Some("2026-09-02".into()),
        "date-only 应解析为当日零点 UTC"
    );
    assert!(row.1.is_some(), "valid_until 应落入");
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "蒸馏应成功：{}",
        j.error.unwrap_or_default()
    );
}

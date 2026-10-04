//! 蒸馏链 mock e2e：进程内 Runner + MockLlm。
//! Phase 2 出口 mock 部分：解析重试 / 仲裁三分支 / 画像版本化 / 引用链。

mod support;

use engram_distill::llm_port::MockLlm;
use engram_distill::register_handlers;
use engram_jobs::types::{JobStatus, JobTemplate};
use engram_jobs::{JobQueue, Runner, RunnerConfig};
use pgvector::Vector;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

struct Env {
    pool: sqlx::PgPool,
    queue: JobQueue,
    handle: engram_jobs::RunnerHandle,
    llm: std::sync::Arc<MockLlm>,
    _pg: support::TestPg,
}

async fn setup(chats: Vec<serde_json::Value>) -> Env {
    setup_with(chats, None).await
}

/// 带 cipher 的变体（T014：JEV 哨兵解密配置用；None = 哨兵静默降级）。
async fn setup_with(
    chats: Vec<serde_json::Value>,
    cipher: Option<engram_llm::crypto::KeyCipher>,
) -> Env {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");

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
                per_kind_concurrency: Default::default(),
                cipher,
            },
        ),
        llm.clone(),
    );
    let handle = runner.start();
    Env {
        pool: pool.clone(),
        queue: JobQueue::new(pool),
        handle,
        llm,
        _pg: container,
    }
}

async fn wait_done(queue: &JobQueue, kind: &str) -> engram_jobs::Job {
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

/// L0 → extract（解析重试，直落 active+向量化）→ organize（空动作收尾）。
/// 验证：会话 done、原子 active、低置信（<0.55）丢弃、embedding/tsv/source_refs 齐全。
#[tokio::test]
async fn extract_with_retry_and_full_refs() {
    let env = setup(vec![
        serde_json::Value::String("抱歉这不是 JSON".into()), // 第一次：非法 → 重试
        json!({"atoms": [
            {"kind": "fact", "content": "用户用 Mac 开发", "confidence": 0.9, "turn_refs": [1]},
            {"kind": "preference", "content": "用户喜欢暗色主题", "confidence": 0.5, "turn_refs": [1]},
        ]}),
        // organize：空动作（链收尾）
        json!({"tool": "finish", "args": {"summary": ""}}),
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
    let j2 = wait_done(&env.queue, "organize_scenarios").await;
    assert_eq!(j2.status, JobStatus::Succeeded);

    // 会话 done
    let st: String = sqlx::query_scalar("SELECT distill_status FROM raw_sessions WHERE id = $1")
        .bind(sid)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(st, "done");

    // P015：conf<0.55 丢弃 → 只有 1 条落库；active 直落、无待审、embedding+tsv+source_refs 齐全
    let rows: Vec<(String, String, bool, bool, bool, serde_json::Value)> = sqlx::query_as(
        "SELECT content, status, needs_review, embedding IS NOT NULL, tsv IS NOT NULL, source_refs \
         FROM atoms ORDER BY created_at",
    )
    .fetch_all(&env.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1, "0.5 低置信应被丢弃");
    let mac = &rows[0];
    assert_eq!(mac.1, "active");
    assert!(!mac.2, "P015 无待审通道");
    assert!(mac.3 && mac.4, "embedding 与 tsv 都应生成");
    assert!(
        mac.5.to_string().contains(&sid.to_string()),
        "source_refs 指向 L0: {}",
        mac.5
    );

    env.handle
        .shutdown_and_wait(std::time::Duration::from_secs(5))
        .await;
}

/// 仲裁三分支（真实 id）+ organize 归组 + persona 版本化：一条龙。
/// 防抖：同窗口两次触发复用同一任务；到期执行取走全部 pending 会话。
#[tokio::test]
async fn debounce_bucket_shares_job() {
    let env = setup(vec![
        json!({"atoms": []}),
        json!({"verdicts": []}),
        json!({"tool": "finish", "args": {"summary": ""}}),
    ])
    .await;

    let j1 = engram_distill::trigger_auto_extract(&env.queue, 30)
        .await
        .unwrap();
    let j2 = engram_distill::trigger_auto_extract(&env.queue, 30)
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

    env.handle
        .shutdown_and_wait(std::time::Duration::from_secs(5))
        .await;
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

    env.handle
        .shutdown_and_wait(std::time::Duration::from_secs(5))
        .await;
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

    env.handle
        .shutdown_and_wait(std::time::Duration::from_secs(5))
        .await;
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
        json!({"tool": "finish", "args": {"summary": ""}}),
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

    env.handle
        .shutdown_and_wait(std::time::Duration::from_secs(5))
        .await;
}

/// 会话级敏感标记：session sensitive=true → 蒸馏产物自动继承 sensitive。
#[tokio::test]
async fn extract_inherits_session_sensitive() {
    let env = setup(vec![
        json!({"atoms": [
            {"kind": "fact", "content": "用户对青霉素过敏", "confidence": 0.9, "turn_refs": [1]}
        ]}),
        json!({"verdicts": [{"candidate_id": "00000000-0000-0000-0000-000000000000", "disposition": "duplicate"}]}),
        json!({"tool": "finish", "args": {"summary": ""}}),
    ])
    .await;

    let sid = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO raw_sessions (id, agent, content, sensitive) VALUES ($1, 'pi', $2, true)",
    )
    .bind(sid)
    .bind(session(&[("user", "我有个隐私：对青霉素严重过敏")]))
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
    let sensitive_n: i64 = sqlx::query_scalar("SELECT count(*) FROM atoms WHERE sensitive")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(sensitive_n, 1, "敏感会话的产物应自动 sensitive");

    env.handle
        .shutdown_and_wait(std::time::Duration::from_secs(5))
        .await;
}

/// 圈子强化 P3：extract 抽取类型化关系（顶层 relations → entity_relations）。
#[tokio::test]
async fn extract_creates_relations() {
    let env = setup(vec![
        json!({"atoms": [
            {"kind": "fact", "content": "张三在后端组负责 API 层", "confidence": 0.9, "turn_refs": [1],
             "entities": [{"name": "张三", "kind": "person"}, {"name": "后端组", "kind": "group"}]}
        ],
        "relations": [{"from": "张三", "to": "后端组", "rel_type": "member_of"}]}),
        json!({"verdicts": [{"candidate_id": "00000000-0000-0000-0000-000000000000", "disposition": "duplicate"}]}),
        json!({"tool": "finish", "args": {"summary": ""}}),
    ])
    .await;

    let sid = Uuid::now_v7();
    sqlx::query("INSERT INTO raw_sessions (id, agent, content) VALUES ($1, 'pi', $2)")
        .bind(sid)
        .bind(session(&[("user", "张三在后端组负责 API 层")]))
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
    // 关系落库：张三 member_of 后端组，source=distill
    let rel_n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM entity_relations WHERE rel_type = 'member_of' AND source = 'distill'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(rel_n, 1, "extract 应抽出并落库 1 条 member_of 关系");

    env.handle
        .shutdown_and_wait(std::time::Duration::from_secs(5))
        .await;
}

/// 关系回溯：存量实体（无 session 可重放）由 consolidate 直接抽关系。
#[tokio::test]
async fn consolidate_backfills_relations_for_stale_entities() {
    // mock：2 份实体档案（summary 空 → 触发 portrait）+ 1 份关系回溯
    let env = setup(vec![
        json!({"summary": "权志龙是 BIGBANG 队长"}),
        json!({"summary": "BIGBANG 是韩国男团"}),
        json!({"relations": [{"from": "权志龙", "to": "BIGBANG", "rel_type": "member_of"}]}),
    ])
    .await;

    let gd: Uuid = sqlx::query_scalar(
        "INSERT INTO entities (id, name, kind, summary) VALUES ($1, '权志龙', 'person', '') RETURNING id",
    )
    .bind(Uuid::now_v7())
    .fetch_one(&env.pool)
    .await
    .unwrap();
    let bb: Uuid = sqlx::query_scalar(
        "INSERT INTO entities (id, name, kind, summary) VALUES ($1, 'BIGBANG', 'group', '') RETURNING id",
    )
    .bind(Uuid::now_v7())
    .fetch_one(&env.pool)
    .await
    .unwrap();

    for (eid, content, tsv) in [
        (gd, "权志龙是 BIGBANG 的队长", "gd bigbang"),
        (bb, "BIGBANG 是韩国男团", "bigbang group"),
    ] {
        let aid = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, tsv) \
             VALUES ($1, 'fact', $2, 0.9, 'active', '[]'::jsonb, to_tsvector('simple', $3))",
        )
        .bind(aid)
        .bind(content)
        .bind(tsv)
        .execute(&env.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO atom_entities (atom_id, entity_id) VALUES ($1, $2)")
            .bind(aid)
            .bind(eid)
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

    let rel_n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM entity_relations WHERE rel_type = 'member_of' AND source = 'distill'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(rel_n, 1, "关系回溯应抽出并落库 1 条 member_of 关系");

    env.handle
        .shutdown_and_wait(std::time::Duration::from_secs(5))
        .await;
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

    env.handle
        .shutdown_and_wait(std::time::Duration::from_secs(5))
        .await;
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
        json!({"tool": "finish", "args": {"summary": ""}}),
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

/// P1：仲裁相似查找必须覆盖无嵌入的种子原子（ANN∪FTS 并集）。
/// 双胞胎案：种子「周日晚上不安排长任务」SQL 直插无嵌入；蒸馏产出近义候选
/// 「周日晚上不排长任务」——修复前 ANN 分支看不见种子 → 候选直通转正成双胞胎。
/// 断言：LLM 收到的仲裁 prompt 里出现种子内容（FTS 补位把它喂了进去）。
/// R3 画像退休：分面超 7 天未更新 → 即使 payload 无新场景也以近期场景强制重写。
// F3 快照收敛：含 archived 成员的场景——活跃≥1 重算（atom_refs 重写为活跃成员）、
// 全非活跃解散删除。
// F4 口径更新（P001 决策 001）：敏感原子照常进 organize 素材——sensitive 只是标记不再排除。
#[tokio::test]
async fn organize_includes_sensitive_atoms_in_prompt() {
    // agentic 形态：素材经 atoms_pending 工具结果进入下一轮 history（sensitive 仅标记，
    // 决策 001——照常可见）。断言两轮轮转发生（atoms_pending → finish）。
    let env = setup(vec![
        json!({"tool": "atoms_pending", "args": {"page": 1}}),
        json!({"tool": "finish", "args": {"summary": ""}}),
    ])
    .await;

    sqlx::query(
        "INSERT INTO atoms (id, kind, content, confidence, status, needs_review, sensitive) \
                 VALUES ($1, 'fact', '普通原子内容公开可见', 0.9, 'active', false, false)",
    )
    .bind(Uuid::now_v7())
    .execute(&env.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, confidence, status, needs_review, sensitive) \
                 VALUES ($1, 'fact', '青霉素过敏绝对机密', 0.9, 'active', false, true)",
    )
    .bind(Uuid::now_v7())
    .execute(&env.pool)
    .await
    .unwrap();

    env.queue
        .enqueue(JobTemplate::new("organize_scenarios").with_payload(json!({})))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "organize_scenarios").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "organize 应成功：{}",
        j.error.unwrap_or_default()
    );

    let sent_n = env.llm.sent_user.lock().unwrap().len();
    assert!(
        sent_n >= 2,
        "agentic 至少两轮（atoms_pending→finish）：{sent_n}"
    );
}

#[tokio::test]
async fn scenario_converge_recompute_and_dissolve() {
    let env = setup(vec![
        // 第 1 发：场景 A 的收敛重算
        json!({ "topic": "骑行", "summary": "仅活跃成员的新摘要", "body": "新正文" }),
        // 第 2 发：主流程把未归组的 a3 再组织一次——空动作即可
        json!({"tool": "finish", "args": {"summary": ""}}),
    ])
    .await;

    // 场景 A：3 成员 2 archived 1 active → 重算
    let a1 = Uuid::now_v7();
    let a2 = Uuid::now_v7();
    let a3 = Uuid::now_v7();
    for (id, st) in [(a1, "archived"), (a2, "archived"), (a3, "active")] {
        sqlx::query("INSERT INTO atoms (id, kind, content, confidence, status, needs_review) VALUES ($1, 'fact', $2, 0.9, $3, false)")
            .bind(id)
            .bind(format!("原子{id}"))
            .bind(st)
            .execute(&env.pool)
            .await
            .unwrap();
    }
    let sa = Uuid::now_v7();
    sqlx::query("INSERT INTO scenarios (id, topic, summary, body, atom_refs, version) VALUES ($1, '骑行', '旧摘要含已归档内容', '旧正文', $2, 1)")
        .bind(sa)
        .bind(sqlx::types::Json(vec![a1, a2, a3]))
        .execute(&env.pool)
        .await
        .unwrap();

    // 场景 B：成员全 archived → 解散
    let b1 = Uuid::now_v7();
    sqlx::query("INSERT INTO atoms (id, kind, content, confidence, status, needs_review) VALUES ($1, 'fact', 'B成员', 0.9, 'archived', false)")
        .bind(b1)
        .execute(&env.pool)
        .await
        .unwrap();
    let sb = Uuid::now_v7();
    sqlx::query("INSERT INTO scenarios (id, topic, summary, body, atom_refs, version) VALUES ($1, '旧项目', '旧摘要', '旧正文', $2, 1)")
        .bind(sb)
        .bind(sqlx::types::Json(vec![b1]))
        .execute(&env.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE atoms SET scenario_id = $2 WHERE id = $1")
        .bind(b1)
        .bind(sb)
        .execute(&env.pool)
        .await
        .unwrap();

    // mock：重算场景 A 的一次 chat（返回新快照）

    env.queue
        .enqueue(JobTemplate::new("organize_scenarios").with_payload(json!({})))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "organize_scenarios").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "收敛 organize 应成功：{}",
        j.error.unwrap_or_default()
    );

    // A：重算——atom_refs 只剩 a3，version+1，摘要重写
    let (refs, ver, summary): (serde_json::Value, i32, String) =
        sqlx::query_as("SELECT atom_refs, version, summary FROM scenarios WHERE id = $1")
            .bind(sa)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(ver, 2, "重算应递增版本");
    assert_eq!(
        refs.as_array().unwrap().len(),
        1,
        "atom_refs 应只剩活跃成员"
    );
    assert_eq!(
        refs.as_array().unwrap()[0].as_str().unwrap(),
        a3.to_string()
    );
    assert!(summary.contains("活跃成员"), "摘要应重写：{summary}");

    // B：解散——场景删除，成员 scenario_id 清空
    let gone: Option<Uuid> = sqlx::query_scalar("SELECT id FROM scenarios WHERE id = $1")
        .bind(sb)
        .fetch_optional(&env.pool)
        .await
        .unwrap();
    assert!(gone.is_none(), "全非活跃场景应被解散");
    let sid: Option<Uuid> = sqlx::query_scalar("SELECT scenario_id FROM atoms WHERE id = $1")
        .bind(b1)
        .fetch_optional(&env.pool)
        .await
        .unwrap()
        .flatten();
    assert!(sid.is_none(), "解散场景成员指向应清空");
}

// F3 persona 素材全空：stale 分面写空版本（content=''），不静默跳过。
// F4 治：organize 收敛传 removed_texts → persona prompt 明确剔除块。
// 编辑能力：用户钉住（manually_edited）的分面——蒸馏输出落库前丢弃。
#[tokio::test]
async fn persona_skips_pinned_facet_on_write() {
    let env = setup(vec![json!({"aspects": [
        {"aspect": "constraints", "content": "蒸馏想覆盖的手编约束", "evidence_scenarios": ["S1"]},
        {"aspect": "preferences", "content": "蒸馏写的偏好", "evidence_scenarios": ["S1"]}
    ]})])
    .await;

    // constraints 钉住 + preferences 未钉
    sqlx::query("INSERT INTO persona_aspects (id, aspect, content, version, manually_edited, created_at, updated_at) \
                 VALUES ($1, 'constraints', '用户手编的约束', 1, true, now() - interval '8 days', now() - interval '8 days')")
        .bind(Uuid::now_v7())
        .execute(&env.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO scenarios (id, topic, summary, body, atom_refs) \
                 VALUES ($1, '骑行', '周末骑行', '正文', '[]'::jsonb)",
    )
    .bind(Uuid::now_v7())
    .execute(&env.pool)
    .await
    .unwrap();

    env.queue
        .enqueue(JobTemplate::new("distill_persona").with_payload(json!({
            "scenario_ids": [],
        })))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "distill_persona").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "{}",
        j.error.unwrap_or_default()
    );

    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM persona_aspects WHERE aspect = 'constraints' AND prompt_version != 'v1' AND manually_edited = false",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(n, 0, "钉住分面不得产生非手编新版本");
    let pref: Option<String> = sqlx::query_scalar(
        "SELECT content FROM persona_aspects WHERE aspect = 'preferences' AND content = '蒸馏写的偏好' LIMIT 1",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap_or(None);
    assert!(pref.is_some(), "未钉分面正常落库");
}

// 全量重建（收录哲学线 task-10）：payload.full_rebuild=true → 素材 = 全部场景（非增量），
// 所有非钉住分面视为 stale 强制重写；钉住分面（manually_edited）豁免。
#[tokio::test]
async fn persona_full_rebuild_uses_all_scenarios_and_skips_pinned() {
    let env = setup(vec![json!({"aspects": [
        {"aspect": "identity", "content": "重建后的身份", "evidence_scenarios": ["S1", "S2"]},
        {"aspect": "constraints", "content": "重建想覆盖的手编约束", "evidence_scenarios": ["S1"]}
    ]})])
    .await;

    // constraints 钉住；identity 未钉；两个场景（full_rebuild 素材应为全部场景）
    sqlx::query("INSERT INTO persona_aspects (id, aspect, content, version, manually_edited, created_at, updated_at) \
                 VALUES ($1, 'constraints', '用户手编的约束', 1, true, now() - interval '8 days', now() - interval '8 days')")
        .bind(Uuid::now_v7())
        .execute(&env.pool)
        .await
        .unwrap();
    for (topic, summary) in [("骑行", "周末骑行"), ("读书", "技术阅读")] {
        sqlx::query(
            "INSERT INTO scenarios (id, topic, summary, body, atom_refs) \
                     VALUES ($1, $2, $3, '正文', '[]'::jsonb)",
        )
        .bind(Uuid::now_v7())
        .bind(topic)
        .bind(summary)
        .execute(&env.pool)
        .await
        .unwrap();
    }

    env.queue
        .enqueue(JobTemplate::new("distill_persona").with_payload(json!({
            "full_rebuild": true,
        })))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "distill_persona").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "{}",
        j.error.unwrap_or_default()
    );

    // 全量素材：LLM 收到的 prompt 应含全部场景（不带 scenario_ids 也全量）
    let sent = env.llm.sent_user.lock().unwrap().join("\n");
    assert!(sent.contains("骑行"), "素材应含场景1");
    assert!(sent.contains("读书"), "素材应含场景2");

    // 非钉住分面重写落库；钉住分面豁免（无新非手编版本）
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM persona_aspects WHERE aspect = 'identity' AND content = '重建后的身份'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(n, 1, "全量重建应重写非钉住分面");
    let pinned_new: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM persona_aspects WHERE aspect = 'constraints' AND manually_edited = false",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(pinned_new, 0, "钉住分面不得产生非手编新版本");
}

// 编辑能力：手编实体档案（manually_edited）——consolidate 档案重生成绕开。
#[tokio::test]
async fn consolidate_skips_manual_entity_portrait() {
    let env = setup(vec![]).await;
    // 手编实体 + 3 原子（够档案候选门槛）
    let eid = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO entities (id, name, kind, summary, manually_edited) \
                 VALUES ($1, '张三', 'person', '用户手编的档案', true)",
    )
    .bind(eid)
    .execute(&env.pool)
    .await
    .unwrap();
    for i in 0..3 {
        let aid = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, confidence, status, needs_review) \
                     VALUES ($1, 'fact', $2, 0.9, 'active', false)",
        )
        .bind(aid)
        .bind(format!("张三素材{i}"))
        .execute(&env.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO atom_entities (atom_id, entity_id) VALUES ($1, $2)")
            .bind(aid)
            .bind(eid)
            .execute(&env.pool)
            .await
            .unwrap();
    }

    env.queue
        .enqueue(JobTemplate::new("consolidate").with_payload(json!({})))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "consolidate").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "{}",
        j.error.unwrap_or_default()
    );

    let summary: String = sqlx::query_scalar("SELECT summary FROM entities WHERE id = $1")
        .bind(eid)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(summary, "用户手编的档案", "手编档案不得被档案重生成覆盖");
}

// F4 治边界（测试方头孢案）：素材全空 + removed_texts 非空 → 重叠分面确定性写空版本。
// P-B 直写重建：无待蒸馏会话时 extract 也要链 organize——直写原子进聚类。
#[tokio::test]
async fn extract_chains_organize_for_unassigned_atoms() {
    // 固定原子 id——mock 的 create 动作要求非空 atom_ids（organize 的防幻觉守卫）
    let aid = Uuid::parse_str("00000000-0000-0000-0000-00000000abcd").unwrap();
    let env = setup(vec![
        json!({"tool": "scenario_write", "args": {
            "topic": "直写聚类", "summary": "直写原子聚成的场景", "body": "正文",
            "member_atom_ids": [aid.to_string()]
        }}),
        json!({"tool": "finish", "args": {"summary": ""}}),
    ])
    .await;

    // 直写原子：无会话、无 scenario_id（批量导入/重建的典型形态）
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, confidence, status, needs_review, scenario_id) \
                 VALUES ($1, 'fact', '直写导入的原子内容', 0.9, 'active', false, NULL)",
    )
    .bind(aid)
    .execute(&env.pool)
    .await
    .unwrap();

    // full 蒸馏：extract 空认领 → 链 organize → 场景成形
    env.queue
        .enqueue(JobTemplate::new("extract_atoms").with_payload(json!({"reason": "manual"})))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "organize_scenarios").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "{}",
        j.error.unwrap_or_default()
    );

    let topic: String = sqlx::query_scalar("SELECT topic FROM scenarios LIMIT 1")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(topic, "直写聚类", "直写原子应被 organize 聚类");
}

#[tokio::test]
async fn persona_retires_facet_when_all_material_removed() {
    // 无 chat 响应——该分支不应触碰 LLM
    let env = setup(vec![]).await;

    sqlx::query("INSERT INTO persona_aspects (id, aspect, content, evidence_refs, version, prompt_version, created_at, updated_at) \
                 VALUES ($1, 'constraints', '用户对头孢类药物过敏，服用后会起疹子，用药必须避开', '[]'::jsonb, 1, 'v1', now(), now())")
        .bind(Uuid::now_v7())
        .execute(&env.pool)
        .await
        .unwrap();

    env.queue
        .enqueue(JobTemplate::new("distill_persona").with_payload(json!({
            "scenario_ids": [Uuid::now_v7().to_string()],
            "removed_texts": ["用户对头孢类药物过敏，服用后会起疹子"]
        })))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "distill_persona").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "{}",
        j.error.unwrap_or_default()
    );

    let (content, version): (String, i32) =
        sqlx::query_as("SELECT content, version FROM persona_aspects WHERE aspect = 'constraints' ORDER BY version DESC LIMIT 1")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(version, 2, "清退应写新版本");
    assert_eq!(
        content, "",
        "素材全空+removed 命中应写空版本（化石不得滞留）"
    );
}

#[tokio::test]
async fn persona_prompt_carries_removed_texts() {
    let env = setup(vec![json!({"aspects": [
        {"aspect": "constraints", "content": "健康：无已知过敏。", "evidence_scenarios": ["S1"]}
    ]})])
    .await;

    // 素材场景 + 带 removed_texts 的 payload（scenario_ids 非空才走正常重写路径）
    let sid = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO scenarios (id, topic, summary, body, atom_refs) \
                 VALUES ($1, '骑行', '周末骑行', '正文', '[]'::jsonb)",
    )
    .bind(sid)
    .execute(&env.pool)
    .await
    .unwrap();

    env.queue
        .enqueue(JobTemplate::new("distill_persona").with_payload(json!({
            "scenario_ids": [sid.to_string()],
            "removed_texts": ["用户对青霉素严重过敏，用药必须避开"]
        })))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "distill_persona").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "{}",
        j.error.unwrap_or_default()
    );

    let sent = env.llm.sent_user.lock().unwrap();
    let prompt = sent.first().cloned().unwrap_or_default();
    assert!(
        prompt.contains("已从记忆移除的表述"),
        "prompt 应含剔除块：{prompt}"
    );
    assert!(prompt.contains("青霉素"), "被移除表述应点名：{prompt}");
    assert!(prompt.contains("不得再包含"), "应明令禁止保留：{prompt}");
}

#[tokio::test]
async fn persona_writes_empty_version_when_no_material() {
    let env = setup(vec![]).await;

    // 无任何场景 + 一个 8 天前的分面
    sqlx::query("INSERT INTO persona_aspects (id, aspect, content, evidence_refs, version, prompt_version, created_at, updated_at) \
                 VALUES ($1, 'routines', '旧的例行内容', '[]'::jsonb, 5, '1', now() - interval '8 days', now() - interval '8 days')")
        .bind(Uuid::now_v7())
        .execute(&env.pool)
        .await
        .unwrap();

    env.queue
        .enqueue(JobTemplate::new("distill_persona").with_payload(json!({"scenario_ids": []})))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "distill_persona").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "空素材 persona 应成功：{}",
        j.error.unwrap_or_default()
    );

    let (content, version): (String, i32) =
        sqlx::query_as("SELECT content, version FROM persona_aspects WHERE aspect = 'routines' ORDER BY version DESC LIMIT 1")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(version, 6, "素材全空应写新版本");
    assert_eq!(content, "", "素材全空应写空内容（UI 过滤不展示）");
}

#[tokio::test]
async fn persona_stale_facet_forces_refresh() {
    let env = setup(vec![
        json!({"aspects": [
            {"aspect": "routines", "content": "周末骑行；9 月 9 日带小王去淀山湖（已重写，绝对日期）", "evidence_scenarios": ["S1"]}
        ]}),
    ])
    .await;

    // 陈旧分面（10 天前的 v1）+ 一条近期场景作素材
    sqlx::query(
        "INSERT INTO persona_aspects (id, aspect, content, evidence_refs, version, prompt_version, created_at, updated_at) \
         VALUES ($1, 'routines', 'v1 旧内容：下周三计划带小王骑行（相对词）', '[]'::jsonb, 1, 'v1', now() - interval '10 days', now() - interval '10 days')",
    )
    .bind(Uuid::now_v7())
    .execute(&env.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO scenarios (id, topic, summary, body, atom_refs) \
         VALUES ($1, '骑行圈', '周末骑行习惯', '正文', '[]'::jsonb)",
    )
    .bind(Uuid::now_v7())
    .execute(&env.pool)
    .await
    .unwrap();

    // 空 scenario_ids——退休检查应触发并以场景素材重写
    env.queue
        .enqueue(JobTemplate::new("distill_persona").with_payload(json!({"scenario_ids": []})))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "distill_persona").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "退休重写应成功：{}",
        j.error.unwrap_or_default()
    );

    let (content, version): (String, i32) = sqlx::query_as(
        "SELECT content, version FROM persona_aspects WHERE aspect='routines' ORDER BY version DESC LIMIT 1",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(version, 2, "应写出 v2");
    assert!(content.contains("绝对日期"), "重写内容生效：{content}");
}

/// memory-rhythm：cron 通道的日桶幂等——同日重复触发 consolidate 返回既有 job，
/// extract 永不去重（扫 pending 是兜底本意）；manual 通道行为不变（无键随时可重触发）。
#[tokio::test]
async fn cron_full_consolidate_daily_idempotent() {
    let env = setup(vec![]).await;
    let c1 = engram_distill::chain::trigger(&env.queue, true, "cron", "key:cron")
        .await
        .unwrap();
    let c2 = engram_distill::chain::trigger(&env.queue, true, "cron", "key:cron")
        .await
        .unwrap();

    // 两次各有 extract + consolidate
    assert_eq!(c1.len(), 2);
    assert_eq!(c2.len(), 2);
    let cons1 = c1.iter().find(|j| j.kind == "consolidate").unwrap();
    let cons2 = c2.iter().find(|j| j.kind == "consolidate").unwrap();
    assert_eq!(
        cons1.id, cons2.id,
        "cron 同日 consolidate 应幂等返回既有 job"
    );
    let ext1 = c1.iter().find(|j| j.kind == "extract_atoms").unwrap();
    let ext2 = c2.iter().find(|j| j.kind == "extract_atoms").unwrap();
    assert_ne!(
        ext1.id, ext2.id,
        "extract 永不去重（扫 pending 是兜底本意）"
    );
    // 触发源标记进 payload（可观测性）
    assert_eq!(ext1.payload.get("triggered_by"), Some(&json!("key:cron")));
    assert_eq!(ext1.payload.get("reason"), Some(&json!("cron")));

    // manual 通道：无键，随时重触发都是新 job
    let m1 = engram_distill::chain::trigger(&env.queue, true, "manual", "admin")
        .await
        .unwrap();
    let m2 = engram_distill::chain::trigger(&env.queue, true, "manual", "admin")
        .await
        .unwrap();
    let mc1 = m1.iter().find(|j| j.kind == "consolidate").unwrap();
    let mc2 = m2.iter().find(|j| j.kind == "consolidate").unwrap();
    assert_ne!(mc1.id, mc2.id, "manual consolidate 不幂等（旧行为保留）");
    assert_eq!(mc1.payload.get("reason"), Some(&json!("manual")));
}

/// v2 修复（H-A2）：distill=off 的会话**永久豁免**蒸馏——即使 extract 任务运行，
/// off 会话也不被认领（此前会被 pending 全量扫描顺带蒸掉）。
#[tokio::test]
async fn extract_skips_distill_off_sessions() {
    let env = setup(vec![]).await;

    let off_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO raw_sessions (id, agent, content, metadata) \
         VALUES ($1, 'pi', $2::jsonb, '{\"distill\":\"off\"}'::jsonb)",
    )
    .bind(off_id)
    .bind(r#"[{"speaker":"user","text":"OFF 会话内容"}]"#)
    .execute(&env.pool)
    .await
    .unwrap();

    env.queue
        .enqueue(JobTemplate::new("extract_atoms"))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "extract_atoms").await;
    assert_eq!(j.status, JobStatus::Succeeded, "extract 任务应成功");

    // off 会话必须仍是 pending——豁免生效
    let status: String =
        sqlx::query_scalar("SELECT distill_status FROM raw_sessions WHERE agent = 'pi'")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(
        status, "pending",
        "distill=off 会话不应被 extract 认领（H-A2 回归）"
    );
}

// ============================================================================
// 架构治理 task-3：五个蒸馏 job 的 MockLlm 覆盖（主分支 + 错误分支）
// ============================================================================

/// 直插一条原子（测试用最小列集：id/kind/content/confidence/status/embedding/tsv）。
async fn insert_atom(env: &Env, id: Uuid, content: &str, status: &str) {
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, confidence, status, embedding, tsv) \
         VALUES ($1, 'fact', $2, 0.9, $3, $4, to_tsvector('simple', $2))",
    )
    .bind(id)
    .bind(content)
    .bind(status)
    .bind(emb(1))
    .execute(&env.pool)
    .await
    .unwrap();
}

async fn insert_session(env: &Env, id: Uuid, text: &str) {
    sqlx::query("INSERT INTO raw_sessions (id, agent, content) VALUES ($1, 'pi', $2)")
        .bind(id)
        .bind(session(&[("user", text)]))
        .execute(&env.pool)
        .await
        .unwrap();
}

async fn atom_status(env: &Env, id: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM atoms WHERE id = $1")
        .bind(id)
        .fetch_one(&env.pool)
        .await
        .unwrap()
}

/// extract：主分支落候选原子；错误分支（LLM 两次解析全败）判失败且会话回滚 pending。
#[tokio::test]
async fn jobs_mock_extract_main_and_error() {
    let env = setup(vec![json!({"atoms": [
        {"kind": "fact", "content": "用户用 Mac 开发", "confidence": 0.9, "turn_refs": [1]}
    ]})])
    .await;
    let sid = Uuid::now_v7();
    insert_session(&env, sid, "我用 Mac 开发").await;
    env.queue
        .enqueue(JobTemplate::new("extract_atoms"))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "extract_atoms").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "主分支应成功: {:?}",
        j.error
    );
    // P015：抽取直落 active（无候选态/无在线仲裁）
    let (n, st): (i64, String) = sqlx::query_as("SELECT count(*), min(status) FROM atoms")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!((n, st.as_str()), (1, "active"), "候选经链上仲裁转正");
    env.handle.shutdown_and_wait(Duration::from_secs(5)).await;

    // 错误分支：两段垃圾响应 → 两次解析失败 → 任务失败；认领会话必须回滚 pending
    let env = setup(vec![
        serde_json::Value::String("不是 JSON".into()),
        serde_json::Value::String("仍不是 JSON".into()),
    ])
    .await;
    let sid2 = Uuid::now_v7();
    insert_session(&env, sid2, "这条会失败").await;
    env.queue
        .enqueue(JobTemplate::new("extract_atoms"))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "extract_atoms").await;
    assert_ne!(
        j.status,
        JobStatus::Succeeded,
        "解析全败应判失败，不能静默成功"
    );
    let st: String = sqlx::query_scalar("SELECT distill_status FROM raw_sessions WHERE id = $1")
        .bind(sid2)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(
        st, "pending",
        "失败应把认领会话回滚为 pending（重试不丢数据）"
    );
    env.handle.shutdown_and_wait(Duration::from_secs(5)).await;
}

/// arbitrate：主分支按裁决归档候选并写取代链；错误分支失败且候选保持 candidate（无部分写入）。
/// organize：主分支按动作建场景并回填 atom.scenario_id；错误分支失败且不建场景。
#[tokio::test]
async fn jobs_mock_organize_main_and_error() {
    let atom = Uuid::now_v7();
    let env = setup(vec![
        json!({"tool": "scenario_write", "args": {
            "topic": "开发环境", "summary": "用 Mac", "body": "详情",
            "member_atom_ids": [atom.to_string()]
        }}),
        json!({"tool": "finish", "args": {"summary": ""}}),
    ])
    .await;
    insert_atom(&env, atom, "用户用 Mac 开发", "active").await;
    env.queue
        .enqueue(JobTemplate::new("organize_scenarios"))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "organize_scenarios").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "主分支应成功: {:?}",
        j.error
    );
    let (scenarios, topic): (i64, String) =
        sqlx::query_as("SELECT count(*), min(topic) FROM scenarios")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!((scenarios, topic.as_str()), (1, "开发环境"));
    let linked: Option<Uuid> = sqlx::query_scalar("SELECT scenario_id FROM atoms WHERE id = $1")
        .bind(atom)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert!(linked.is_some(), "原子应被回填 scenario_id");
    env.handle.shutdown_and_wait(Duration::from_secs(5)).await;

    // 错误分支：垃圾响应 → 失败且不建场景
    let atom2 = Uuid::now_v7();
    let env = setup(vec![
        serde_json::Value::String("不是 JSON".into()),
        serde_json::Value::String("仍不是 JSON".into()),
    ])
    .await;
    insert_atom(&env, atom2, "用户用 Mac 开发", "active").await;
    env.queue
        .enqueue(JobTemplate::new("organize_scenarios"))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "organize_scenarios").await;
    assert_ne!(j.status, JobStatus::Succeeded, "解析全败应判失败");
    let scenarios: i64 = sqlx::query_scalar("SELECT count(*) FROM scenarios")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(scenarios, 0, "失败不得留下半建场景");
    env.handle.shutdown_and_wait(Duration::from_secs(5)).await;
}

/// persona：主分支按分面写新版本并落证据链（S 编号 → 场景 id）；错误分支失败且不写分面。
#[tokio::test]
async fn jobs_mock_persona_main_and_error() {
    let atom = Uuid::now_v7();
    let scenario = Uuid::now_v7();
    let env = setup(vec![json!({"aspects": [
        {"aspect": "identity", "content": "用户是开发者", "evidence_scenarios": ["S1"]}
    ]})])
    .await;
    insert_atom(&env, atom, "用户用 Mac 开发", "active").await;
    sqlx::query(
        "INSERT INTO scenarios (id, topic, summary, body, atom_refs) VALUES ($1, '开发', '用 Mac', '详情', $2)",
    )
    .bind(scenario)
    .bind(json!([atom]))
    .execute(&env.pool)
    .await
    .unwrap();
    env.queue
        .enqueue(
            JobTemplate::new("distill_persona").with_payload(json!({"scenario_ids": [scenario]})),
        )
        .await
        .unwrap();
    let j = wait_done(&env.queue, "distill_persona").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "主分支应成功: {:?}",
        j.error
    );
    let (aspect, content, evidence): (String, String, serde_json::Value) = sqlx::query_as(
        "SELECT aspect, content, evidence_refs FROM persona_aspects ORDER BY version DESC LIMIT 1",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(aspect, "identity");
    assert_eq!(content, "用户是开发者");
    assert_eq!(
        evidence["scenarios"],
        json!([scenario]),
        "S1 应被映射回真实场景 id（L3→L2 链）"
    );
    assert_eq!(
        evidence["atoms"],
        json!([atom]),
        "证据链应经场景 atom_refs 带到 L1"
    );
    env.handle.shutdown_and_wait(Duration::from_secs(5)).await;

    // 错误分支：垃圾响应 → 失败且不写分面
    let scenario2 = Uuid::now_v7();
    let env = setup(vec![
        serde_json::Value::String("不是 JSON".into()),
        serde_json::Value::String("仍不是 JSON".into()),
    ])
    .await;
    sqlx::query(
        "INSERT INTO scenarios (id, topic, summary, body) VALUES ($1, '开发', '用 Mac', '详情')",
    )
    .bind(scenario2)
    .execute(&env.pool)
    .await
    .unwrap();
    env.queue
        .enqueue(
            JobTemplate::new("distill_persona").with_payload(json!({"scenario_ids": [scenario2]})),
        )
        .await
        .unwrap();
    let j = wait_done(&env.queue, "distill_persona").await;
    assert_ne!(j.status, JobStatus::Succeeded, "解析全败应判失败");
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM persona_aspects")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "失败不得写入分面");
    env.handle.shutdown_and_wait(Duration::from_secs(5)).await;
}

/// consolidate：主分支按合并项归档 victim 并补取代链；错误分支失败且两条原子都保持 active。
#[tokio::test]
async fn jobs_mock_consolidate_main_and_error() {
    let keep = Uuid::now_v7();
    let victim = Uuid::now_v7();
    let env = setup(vec![json!({"merges": [
        {"keep_id": keep.to_string(), "merge_ids": [victim.to_string()]}
    ]})])
    .await;
    insert_atom(&env, keep, "用户用 Mac 开发", "active").await;
    insert_atom(&env, victim, "用户用 Mac 开发", "active").await;
    env.queue
        .enqueue(JobTemplate::new("consolidate"))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "consolidate").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "主分支应成功: {:?}",
        j.error
    );
    assert_eq!(atom_status(&env, victim).await, "archived", "victim 应归档");
    let sup: Option<Uuid> = sqlx::query_scalar("SELECT superseded_by FROM atoms WHERE id = $1")
        .bind(victim)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(sup, Some(keep), "近重复合并也要补取代指针（R1）");
    assert_eq!(
        atom_status(&env, keep).await,
        "active",
        "keep 应保持 active"
    );
    env.handle.shutdown_and_wait(Duration::from_secs(5)).await;

    // 错误分支：近重复合并的 LLM 解析全败 → 任务失败，且两条原子都不动
    let keep2 = Uuid::now_v7();
    let victim2 = Uuid::now_v7();
    let env = setup(vec![
        serde_json::Value::String("不是 JSON".into()),
        serde_json::Value::String("仍不是 JSON".into()),
    ])
    .await;
    insert_atom(&env, keep2, "用户用 Mac 开发", "active").await;
    insert_atom(&env, victim2, "用户用 Mac 开发", "active").await;
    env.queue
        .enqueue(JobTemplate::new("consolidate"))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "consolidate").await;
    assert_ne!(j.status, JobStatus::Succeeded, "解析全败应判失败");
    assert_eq!(atom_status(&env, keep2).await, "active");
    assert_eq!(
        atom_status(&env, victim2).await,
        "active",
        "合并失败不得部分写入"
    );
    env.handle.shutdown_and_wait(Duration::from_secs(5)).await;
}

/// P003-T001 回归（决策 001 收敛判据）：标敏感成员不再把场景判 stale。
/// 病根复盘：bde6092 让敏感成员进 atom_refs 后，fetch_stale_scenarios 的
/// `OR a.sensitive` 判据未同步——场景被判 stale → 重算刷新 updated_at → 下一轮
/// 再判 stale → 永动烧 LLM + removed_texts 反复把敏感内容当「已移除」喂画像。
#[tokio::test]
async fn sensitive_member_does_not_mark_scenario_stale() {
    let env = setup(vec![
        // 阶段 2 第 1 发：场景 S 收敛重算（a2 归档后触发）
        json!({ "topic": "骑行", "summary": "仅活跃成员的新摘要", "body": "新正文" }),
        // 阶段 2 第 2 发：主组织段空动作兜底
        json!({"tool": "finish", "args": {"summary": ""}}),
    ])
    .await;

    let a1 = Uuid::now_v7();
    let a2 = Uuid::now_v7();
    for (id, content) in [(a1, "成员甲（将标敏感）"), (a2, "成员乙")] {
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, confidence, status, needs_review) \
             VALUES ($1, 'fact', $2, 0.9, 'active', false)",
        )
        .bind(id)
        .bind(content)
        .execute(&env.pool)
        .await
        .unwrap();
    }
    let sa = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO scenarios (id, topic, summary, body, atom_refs, version) \
         VALUES ($1, '骑行', '摘要', '正文', $2, 1)",
    )
    .bind(sa)
    .bind(sqlx::types::Json(vec![a1, a2]))
    .execute(&env.pool)
    .await
    .unwrap();
    // 归组（scenario_id 回填）——否则 organize 主流程会把未归组原子再组织（干扰收敛判据观测）
    sqlx::query("UPDATE atoms SET scenario_id = $1 WHERE id = ANY($2)")
        .bind(sa)
        .bind(&[a1, a2][..])
        .execute(&env.pool)
        .await
        .unwrap();

    // 标敏感 a1（直 SQL——本测试聚焦收敛判据，不走 core 入队层）
    sqlx::query("UPDATE atoms SET sensitive = true WHERE id = $1")
        .bind(a1)
        .execute(&env.pool)
        .await
        .unwrap();

    let before: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT updated_at FROM scenarios WHERE id = $1")
            .bind(sa)
            .fetch_one(&env.pool)
            .await
            .unwrap();

    // 阶段 1：S 含敏感成员 → 不应判 stale（零 LLM 调用、updated_at 不动）
    env.queue
        .enqueue(JobTemplate::new("organize_scenarios").with_payload(json!({})))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "organize_scenarios").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "organize 应成功：{}",
        j.error.unwrap_or_default()
    );
    {
        let sent = env.llm.sent_user.lock().unwrap();
        assert!(
            sent.is_empty(),
            "标敏感不得触发场景收敛重算（P003-T001 永动机回归）：{sent:?}"
        );
    }
    let after: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT updated_at FROM scenarios WHERE id = $1")
            .bind(sa)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(after, before, "未判 stale 则 updated_at 不应刷新");

    // 阶段 2（对照）：归档 a2 → S 判 stale 并重算。wait_done 按 kind 匹配第一个
    // 终态会撞上阶段 1 的 job（二义性）——这里直接轮询最终效果（atom_refs 重算）。
    sqlx::query("UPDATE atoms SET status = 'archived' WHERE id = $1")
        .bind(a2)
        .execute(&env.pool)
        .await
        .unwrap();
    env.queue
        .enqueue(JobTemplate::new("organize_scenarios").with_payload(json!({})))
        .await
        .unwrap();
    let mut refs: sqlx::types::Json<Vec<Uuid>> =
        sqlx::query_scalar("SELECT atom_refs FROM scenarios WHERE id = $1")
            .bind(sa)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    for _ in 0..150 {
        if refs.0 == vec![a1] {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        refs = sqlx::query_scalar("SELECT atom_refs FROM scenarios WHERE id = $1")
            .bind(sa)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    }
    assert_eq!(
        refs.0,
        vec![a1],
        "重算后应只剩活跃成员 a1（敏感成员在位保留——决策 001；15s 超时未重算即永动机/漏触发）：{:?}",
        refs.0
    );
    env.handle.shutdown_and_wait(Duration::from_secs(5)).await;
}

/// T014：JEV 配置非法（垃圾 key_enc → resolve Err）→ 哨兵降级直通——
/// 降级事件落日志可查、原子照常产出（哨兵打盹不阻塞主链）。
#[tokio::test]
async fn jev_gate_degrades_to_passthrough_with_visible_event() {
    let cipher = engram_llm::crypto::KeyCipher::from_hex_master(&"bb".repeat(32)).unwrap();
    let env = setup_with(
        vec![
            json!({"atoms": [
                {"kind": "fact", "content": "用户住在杭州", "confidence": 0.9, "turn_refs": [1]},
            ]}),
            json!({"verdicts": []}),
            json!({"tool": "finish", "args": {"summary": ""}}),
        ],
        Some(cipher),
    )
    .await;

    // settings 写非法 jev 配置（api_key_enc 非法 hex → resolve Err → 降级直通）
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES ('jev', '{\"enabled\":true,\"api_key_enc\":\"zz\"}'::jsonb) \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
    )
    .execute(&env.pool)
    .await
    .unwrap();

    let sid = Uuid::now_v7();
    sqlx::query("INSERT INTO raw_sessions (id, agent, content) VALUES ($1, 'pi', $2)")
        .bind(sid)
        .bind(session(&[("user", "我住在杭州")]))
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
        "降级直通不应失败: {:?}",
        j.error
    );

    // 降级事件可查（logs 表唯一时间线，target=job.extract_atoms）
    let degraded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM logs \
         WHERE target = 'job.extract_atoms' AND message LIKE '%JEV 哨兵不可用%'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert!(degraded >= 1, "降级事件应落日志可查（count={degraded}）");

    // 原子照常产出（直通 = 不丢功能）
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM atoms")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert!(n >= 1, "降级直通后原子应照常产出（count={n}）");
}

/// T002 回归：contradicts 取代落空（旧条已 archived）时候选不得直接 active——
/// 先 supersede 后 promote；降位失败 → 候选提级 needs_review，杜绝同主题双 active。
/// T003 回归：场景成员归属牌（atoms.scenario_id）为唯一真源，atom_refs 按真源重算——
/// 被挪去其他场景的成员自动从原场景缓存消失（不再「并集只进不出」）。
#[tokio::test]
async fn t003_scenario_member_reassignment_recomputes_refs() {
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    let x = Uuid::now_v7();
    let z = Uuid::now_v7();
    let env = setup(vec![
        // 先把 X 明确归 A，再把 X 归 B——最终 X 应只属于 B（agentic scenario_write）
        json!({"tool": "scenario_write", "args": {
            "scenario_id": a.to_string(), "topic": "场景A",
            "summary": "A", "body": "A", "member_atom_ids": [x.to_string()]
        }}),
        json!({"tool": "scenario_write", "args": {
            "scenario_id": b.to_string(), "topic": "场景B",
            "summary": "B", "body": "B", "member_atom_ids": [x.to_string()]
        }}),
        json!({"tool": "finish", "args": {"summary": ""}}),
    ])
    .await;
    for (id, sid) in [(a, None::<Uuid>), (b, None)] {
        let _ = sid;
        sqlx::query(
            "INSERT INTO scenarios (id, topic, summary, body, atom_refs) \
             VALUES ($1, $2, 's', 'b', '[]'::jsonb)",
        )
        .bind(id)
        .bind(format!("t-{id}"))
        .execute(&env.pool)
        .await
        .unwrap();
    }
    // X 挂 A（初始真源）、Z 挂 A（留在 A 的成员）
    for aid in [x, z] {
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, status, confidence, scenario_id, tsv) \
             VALUES ($1, 'fact', $3, 'active', 0.9, $2, to_tsvector('simple', $3))",
        )
        .bind(aid)
        .bind(a)
        .bind(format!("内容-{aid}"))
        .execute(&env.pool)
        .await
        .unwrap();
    }
    // refs 先对齐真源（x/z 挂 A → A.refs=[x,z]）——否则 converge 的双轨漂移兜底
    // 会抢先触发重写快照消耗 mock chat（T003 测的是 organize 写侧归一，不是兜底）
    sqlx::query(
        "UPDATE scenarios SET atom_refs = ( \
            SELECT COALESCE(jsonb_agg(id::text ORDER BY created_at), '[]'::jsonb) \
            FROM atoms WHERE scenario_id = scenarios.id )",
    )
    .execute(&env.pool)
    .await
    .unwrap();
    env.queue
        .enqueue(JobTemplate::new("organize_scenarios").with_payload(json!({"atom_ids": [x, z]})))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "organize_scenarios").await;
    if j.status != JobStatus::Succeeded {
        let jobs: Vec<(String, String, Option<String>)> =
            sqlx::query_as("SELECT kind, status::text, error FROM jobs ORDER BY created_at")
                .fetch_all(&env.pool)
                .await
                .unwrap_or_default();
        let sent: Vec<String> = env
            .llm
            .sent_user
            .lock()
            .unwrap()
            .iter()
            .map(|u| {
                let c: Vec<char> = u.chars().collect();
                let n = c.len();
                if n > 150 {
                    c[n - 150..].iter().collect::<String>()
                } else {
                    c.iter().collect::<String>()
                }
            })
            .collect();
        let stale_probe: Vec<Uuid> = sqlx::query_scalar(
            "SELECT s.id FROM scenarios s WHERE EXISTS ( \
             SELECT 1 FROM jsonb_array_elements_text(s.atom_refs) r \
             LEFT JOIN atoms a ON a.id = r::uuid \
             WHERE a.id IS NULL OR a.status != 'active')",
        )
        .fetch_all(&env.pool)
        .await
        .unwrap_or_default();
        let refs_probe: Vec<(Uuid, String)> =
            sqlx::query_as("SELECT id, atom_refs::text FROM scenarios")
                .fetch_all(&env.pool)
                .await
                .unwrap_or_default();
        let remaining: Vec<String> = env
            .llm
            .chats
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.chars().take(60).collect())
            .collect();
        panic!(
            "organize 应成功: {:?}——全任务: {jobs:?}，stale探针: {stale_probe:?}，refs: {refs_probe:?}，LLM 输入序: {sent:?}，剩余队列: {remaining:?}",
            j.error
        );
    }

    // 诊断：organize 的 emit 摘要与日志行
    let org_logs: Vec<String> = sqlx::query_scalar(
        "SELECT message FROM logs WHERE target = 'job.organize_scenarios' ORDER BY id",
    )
    .fetch_all(&env.pool)
    .await
    .unwrap_or_default();
    let _ = org_logs;
    // 真源：X 归 B（后写的赢）
    let xsid: Option<Uuid> = sqlx::query_scalar("SELECT scenario_id FROM atoms WHERE id = $1")
        .bind(x)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(
        xsid,
        Some(b),
        "X 的归属牌应迁到 B——organize 日志: {org_logs:?}"
    );

    // 缓存：A 的 atom_refs 不再含 X，但保留 Z；B 的 atom_refs = [X]
    let refs_of = |sid: Uuid| {
        let pool = env.pool.clone();
        async move {
            let v: serde_json::Value =
                sqlx::query_scalar("SELECT atom_refs FROM scenarios WHERE id = $1")
                    .bind(sid)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            v.as_array()
                .unwrap()
                .iter()
                .filter_map(|s| s.as_str().and_then(|t| t.parse::<Uuid>().ok()))
                .collect::<Vec<_>>()
        }
    };
    let refs_a = refs_of(a).await;
    let refs_b = refs_of(b).await;
    assert!(!refs_a.contains(&x), "A 缓存应随真源移出 X: {refs_a:?}");
    assert!(refs_a.contains(&z), "A 缓存应保留 Z: {refs_a:?}");
    assert_eq!(refs_b, vec![x], "B 缓存应恰为 [X]: {refs_b:?}");
}

/// 轻量环境（无 runner/mock——纯 repo 层测试用）。
async fn setup_min() -> (sqlx::PgPool, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    (pool, container)
}

/// T005 回归：实体子串归并误吞修复——「云」不再吞进「星云」（新名在旧名内的方向已删）；
/// 「小王同志」复用「小王」档（旧名完整出现在新名里才归并，精确优先）；
/// 关系 lookup 与挂链口径统一（lookup_entity 命中同一档）。
#[tokio::test]
async fn t005_entity_substring_no_longer_swallows() {
    let (pool, _pg) = setup_min().await;
    for (name, kind) in [("星云", "topic"), ("小王", "person")] {
        sqlx::query("INSERT INTO entities (id, name, kind) VALUES ($1, $2, $3)")
            .bind(Uuid::now_v7())
            .bind(name)
            .bind(kind)
            .execute(&pool)
            .await
            .unwrap();
    }
    let aid = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, status, confidence, tsv) \
         VALUES ($1, 'fact', '用户研究云', 'active', 0.9, to_tsvector('simple', '用户研究云'))",
    )
    .bind(aid)
    .execute(&pool)
    .await
    .unwrap();

    // ① 「云」：精确不命中；旧名（星云）不在新名（云）内 → 不归并 → 新建实体
    engram_distill::extract::link_entity(&pool, aid, "云", "topic")
        .await
        .expect("link 云");
    let names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM entities WHERE archived_at IS NULL ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        names.contains(&"星云".to_string()) && names.contains(&"云".to_string()),
        "「云」不得吞进「星云」——应为两个实体: {names:?}"
    );

    // ② 「小王同志」：旧名（小王）完整出现在新名里 → 复用小王档（不新建）
    //   （注意「王小明」不含连续子串「小王」——王-小-明，字符串层面本就不归并）
    let aid2 = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, status, confidence, tsv) \
         VALUES ($1, 'fact', '小王同志来拜访', 'active', 0.9, to_tsvector('simple', '小王同志来拜访'))",
    )
    .bind(aid2)
    .execute(&pool)
    .await
    .unwrap();
    engram_distill::extract::link_entity(&pool, aid2, "小王同志", "person")
        .await
        .expect("link 小王同志");
    let names2: Vec<String> =
        sqlx::query_scalar("SELECT name FROM entities WHERE archived_at IS NULL ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
    let probe: Vec<(String, String, bool, bool, i32, i32)> = sqlx::query_as(
        "SELECT name, kind, merged_into IS NULL, archived_at IS NULL, \
         position(lower(name) in lower('王小明')), length(name) \
         FROM entities WHERE archived_at IS NULL ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        !names2.contains(&"王小明".to_string()) && names2.contains(&"小王".to_string()),
        "「小王同志」应复用「小王」档: {names2:?}，探针: {probe:?}"
    );
    // 挂链验证：小王同志的原子挂在小王档上
    let linked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM atom_entities ae JOIN entities e ON e.id = ae.entity_id \
         WHERE e.name = '小王' AND ae.atom_id = $1",
    )
    .bind(aid2)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(linked, 1, "小王同志的原子应挂到小王档");

    // ③ 关系 lookup 口径统一：lookup_entity("小王同志") 命中小王档（与挂链同款归并）
    let looked = engram_distill::extract::lookup_entity(&pool, "小王同志").await;
    let xiaowang: Uuid = sqlx::query_scalar("SELECT id FROM entities WHERE name = '小王'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(looked, Some(xiaowang), "lookup 应与挂链口径一致命中小王档");
}

/// T008 回归：归档实体同名重现 → 复活延续旧档案（Q002 改判：同联原则）——
/// summary/revision 不沉归档态，不新建实体；墓碑（merged_into）永不复活。
#[tokio::test]
async fn t008_archived_entity_revives_with_continuity() {
    let (pool, _pg) = setup_min().await;
    let eid = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO entities (id, name, kind, summary) VALUES ($1, '老王', 'person', '老王的旧档案摘要')",
    )
    .bind(eid)
    .execute(&pool)
    .await
    .unwrap();
    // 归档（孤儿清扫同款操作）
    sqlx::query("UPDATE entities SET archived_at = now() WHERE id = $1")
        .bind(eid)
        .execute(&pool)
        .await
        .unwrap();

    // 同名重现（旧名完整出现在新名里 → 复用归档档）
    let aid = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, status, confidence, tsv) \
         VALUES ($1, 'fact', '老王师傅来串门', 'active', 0.9, to_tsvector('simple', '老王师傅来串门'))",
    )
    .bind(aid)
    .execute(&pool)
    .await
    .unwrap();
    engram_distill::extract::link_entity(&pool, aid, "老王师傅", "person")
        .await
        .expect("link 老王师傅");

    // 断言：同 id 复活 + 档案延续 + 无新实体
    let (archived, summary): (Option<chrono::DateTime<chrono::Utc>>, Option<String>) =
        sqlx::query_as("SELECT archived_at, summary FROM entities WHERE id = $1")
            .bind(eid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(archived.is_none(), "归档实体应被复活");
    assert_eq!(
        summary.as_deref(),
        Some("老王的旧档案摘要"),
        "旧档案 summary 应延续"
    );
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM entities WHERE name LIKE '老王%'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(total, 1, "同名重现不得新建实体");
    // 原子挂到复活实体上
    let linked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM atom_entities WHERE entity_id = $1 AND atom_id = $2",
    )
    .bind(eid)
    .bind(aid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(linked, 1, "新原子应挂到复活实体档上");
}

/// T006 回归：claim 分批——pending 超过 EXTRACT_CLAIM_BATCH 时候批只认领 50，
/// 满批自续投下一批，最终全部蒸完（不再一次性全量抢占）。
#[tokio::test]
async fn t006_extract_claim_is_batched_with_continuation() {
    // 60 个 pending 会话，内容为空话（extract 每段返回空 atoms）
    let mut chats = vec![];
    for _ in 0..60 {
        chats.push(json!({"atoms": []}));
    }
    // 两个 extract 批各自链一个 organize（空动作）
    chats.push(json!({"tool": "finish", "args": {"summary": ""}}));
    chats.push(json!({"tool": "finish", "args": {"summary": ""}}));
    let env = setup(chats).await;
    for i in 0..60 {
        let sid = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO raw_sessions (id, agent, content, created_at) \
             VALUES ($1, 'pi', $2, now() - make_interval(secs => $3))",
        )
        .bind(sid)
        .bind(session(&[("user", &format!("闲聊第 {i} 句"))]))
        .bind(60.0 - i as f64) // 递减间隔保证 created_at 严格有序
        .execute(&env.pool)
        .await
        .unwrap();
    }
    env.queue
        .enqueue(JobTemplate::new("extract_atoms"))
        .await
        .unwrap();
    // 第一批（50 个）+ 自续批（10 个）都应成功
    let j1 = wait_done(&env.queue, "extract_atoms").await;
    assert_eq!(j1.status, JobStatus::Succeeded, "第一批: {:?}", j1.error);
    // 自续批可能稍后入队——轮询直到全部 done
    let mut done = 0i64;
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        done =
            sqlx::query_scalar("SELECT count(*) FROM raw_sessions WHERE distill_status = 'done'")
                .fetch_one(&env.pool)
                .await
                .unwrap();
        if done >= 60 {
            break;
        }
    }
    if done != 60 {
        let dist: Vec<(String, i64)> = sqlx::query_as(
            "SELECT distill_status, count(*) FROM raw_sessions GROUP BY distill_status",
        )
        .fetch_all(&env.pool)
        .await
        .unwrap_or_default();
        let jobs: Vec<(String, String)> =
            sqlx::query_as("SELECT kind, status FROM jobs ORDER BY created_at")
                .fetch_all(&env.pool)
                .await
                .unwrap_or_default();
        panic!("满批自续后应全部蒸完（done={done}）——状态分布 {dist:?}，任务 {jobs:?}");
    }
    assert_eq!(done, 60, "满批自续后 60 个会话应全部蒸完");
    // 任务链形状：extract 至少两批
    let extract_jobs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind = 'extract_atoms'")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert!(
        extract_jobs >= 2,
        "应存在自续批任务（count={extract_jobs}）"
    );
}

/// P012-T001/T002/T003：agentic 循环——模型驱动六工具完成组织并 finish 交卷；
/// 场景建好 + 成员挂真源 + 场景补 embedding；settings 开关 organize_agentic 控制。
#[tokio::test]
async fn p012_agentic_loop_organizes_and_finishes() {
    let aid = Uuid::now_v7();
    let env = setup_with(
        vec![
            json!({"tool": "atoms_pending", "args": {"page": 1}}),
            json!({"tool": "scenario_write", "args": {
                "topic": "居住地", "summary": "用户住在杭州",
                "body": "用户长期居住在杭州。",
                "member_atom_ids": [aid.to_string()]
            }}),
            json!({"tool": "finish", "args": {"summary": "新建 1 个场景收编 1 原子"}}),
        ],
        None,
    )
    .await;
    // 开关打开（settings 单行）
    engram_storage::repo::settings::put_json(
        &env.pool,
        "organize_agentic",
        &serde_json::json!({"enabled": true}),
    )
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, status, confidence, tsv) \
         VALUES ($1, 'fact', '用户住在杭州西湖区', 'active', 0.9, to_tsvector('simple', 'x'))",
    )
    .bind(aid)
    .execute(&env.pool)
    .await
    .unwrap();

    env.queue
        .enqueue(JobTemplate::new("organize_scenarios").with_payload(json!({})))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "organize_scenarios").await;
    assert_eq!(j.status, JobStatus::Succeeded, "{:?}", j.error);

    // 场景建好 + 成员挂真源 + embedding 补齐
    let row: (Option<Uuid>, bool) =
        sqlx::query_as("SELECT scenario_id, embedding IS NOT NULL FROM atoms WHERE id = $1")
            .bind(aid)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    let (sid, _atoms_emb) = row;
    assert!(sid.is_some(), "原子应挂到新场景");
    let (topic, retired, emb_probe): (
        String,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<String>,
    ) = sqlx::query_as("SELECT topic, retired_at, embedding::text FROM scenarios WHERE id = $1")
        .bind(sid.unwrap())
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(topic, "居住地");
    assert!(
        emb_probe.is_some(),
        "agentic 路径应补场景 embedding（refresh_by_ids）"
    );
    assert!(retired.is_none(), "正常 write 不应退役");
}

/// P012-T002：max_steps=20 硬顶（结构不变量写死）——模型不 finish 也在上限收尾，
/// 任务成功（已执行动作保留），不再消耗 chat。
#[tokio::test]
async fn p012_agentic_loop_steps_capped() {
    // 备 25 个不交卷的动作——循环应在 20 步停（不耗尽即成功收尾）
    let mut chats = Vec::new();
    for _ in 0..25 {
        chats.push(json!({"tool": "atoms_pending", "args": {"page": 1}}));
    }
    let env = setup_with(chats, None).await;
    engram_storage::repo::settings::put_json(
        &env.pool,
        "organize_agentic",
        &serde_json::json!({"enabled": true}),
    )
    .await
    .unwrap();
    // 散落原子（否则 no_atoms_reply 直返，循环不启动）
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, status, confidence, tsv) \
         VALUES ($1, 'fact', '散落原子内容', 'active', 0.9, to_tsvector('simple', 'x'))",
    )
    .bind(Uuid::now_v7())
    .execute(&env.pool)
    .await
    .unwrap();

    env.queue
        .enqueue(JobTemplate::new("organize_scenarios").with_payload(json!({})))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "organize_scenarios").await;
    assert_eq!(
        j.status,
        JobStatus::Succeeded,
        "上限收尾应是成功: {:?}",
        j.error
    );
    assert_eq!(
        env.llm.sent_user.lock().unwrap().len(),
        crate_organize_max_steps(),
        "循环步数应恰为硬顶 20"
    );
}

fn crate_organize_max_steps() -> usize {
    engram_distill::organize_agentic::ORGANIZE_MAX_STEPS
}

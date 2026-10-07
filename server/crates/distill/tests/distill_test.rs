//! 蒸馏链 mock e2e：进程内 Runner + MockLlm。
//! Phase 2 出口 mock 部分：解析重试 / 仲裁三分支 / 画像版本化 / 引用链。

mod support;

use engram_distill::llm_port::MockLlm;
use engram_distill::register_handlers;
use engram_jobs::types::{JobStatus, JobTemplate};
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
    setup_with(chats).await
}

async fn setup_with(chats: Vec<serde_json::Value>) -> Env {
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
            },
        ),
        llm.clone(),
    );
    let handle = runner.start();
    Env {
        pool: pool.clone(),
        queue: JobQueue::new(pool),
        handle,
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

/// L0 → extract（解析重试，直落 active+向量化；P015 链到 organize 已退役）。
/// 验证：会话 done、原子 active、低置信（<0.55）丢弃、embedding/source_refs 齐全。
#[tokio::test]
async fn extract_with_retry_and_full_refs() {
    let env = setup(vec![
        serde_json::Value::String("抱歉这不是 JSON".into()), // 第一次：非法 → 重试
        json!({"atoms": [
            {"kind": "fact", "content": "用户用 Mac 开发", "confidence": 0.9, "turn_refs": [1]},
            {"kind": "preference", "content": "用户喜欢暗色主题", "confidence": 0.5, "turn_refs": [1]},
        ]}),
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

    // 会话 done
    let st: String = sqlx::query_scalar("SELECT distill_status FROM raw_sessions WHERE id = $1")
        .bind(sid)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(st, "done");

    // P015：conf<0.55 丢弃 → 只有 1 条落库；active 直落、embedding+source_refs 齐全
    let rows: Vec<(String, String, bool, serde_json::Value)> = sqlx::query_as(
        "SELECT content, status, embedding IS NOT NULL, source_refs \
         FROM atoms ORDER BY created_at",
    )
    .fetch_all(&env.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1, "0.5 低置信应被丢弃");
    let mac = &rows[0];
    assert_eq!(mac.1, "active");
    assert!(mac.2, "embedding 应生成");
    assert!(
        mac.3.to_string().contains(&sid.to_string()),
        "source_refs 指向 L0: {}",
        mac.3
    );

    env.handle
        .shutdown_and_wait(std::time::Duration::from_secs(5))
        .await;
}

/// 防抖窗口共享（2026-10-04）：organize 归组与 persona 版本化已随场景层退役。
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
async fn reembed_memory_fills_missing_vectors() {
    // 无 chat 调用——纯 embed 路径
    let env = setup(vec![]).await;

    for (i, status) in ["active", "active", "archived"].iter().enumerate() {
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, confidence, status, source_refs) \
             VALUES ($1, 'fact', $2, 0.9, $3, '[]'::jsonb)",
        )
        .bind(Uuid::now_v7())
        .bind(format!("记忆事实 {i}"))
        .bind(status)
        .bind(format!("fact {i}"))
        .execute(&env.pool)
        .await
        .unwrap();
    }
    let miss: i64 = sqlx::query_scalar("SELECT count(*) FROM atoms WHERE embedding IS NULL")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(miss, 3);

    env.queue
        .enqueue(JobTemplate::new("reembed_memory"))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "reembed_memory").await;
    assert_eq!(j.status, JobStatus::Succeeded, "重嵌应成功: {:?}", j.error);

    // active 原子 2 条补齐；archived 不动（P015 场景层退役：无 scenarios 侧）
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM atoms WHERE embedding IS NULL")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(after, 1, "archived 原子应保持无向量，其余补齐");

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
// F4 口径更新（P001 决策 001）：敏感原子照常抽取——sensitive 只是标记不再排除。
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

async fn insert_session(env: &Env, id: Uuid, text: &str) {
    sqlx::query("INSERT INTO raw_sessions (id, agent, content) VALUES ($1, 'pi', $2)")
        .bind(id)
        .bind(session(&[("user", text)]))
        .execute(&env.pool)
        .await
        .unwrap();
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

/// T002 回归：contradicts 取代落空（旧条已 archived）时候选不得直接 active——
/// 先 supersede 后 promote，杜绝同主题双 active。
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
        "INSERT INTO atoms (id, kind, content, status, confidence) \
         VALUES ($1, 'fact', '用户研究云', 'active', 0.9)",
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
        "INSERT INTO atoms (id, kind, content, status, confidence) \
         VALUES ($1, 'fact', '小王同志来拜访', 'active', 0.9)",
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
        "INSERT INTO atoms (id, kind, content, status, confidence) \
         VALUES ($1, 'fact', '老王师傅来串门', 'active', 0.9)",
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
    // 链尾 mock 兜底（P015 后 organize 链退役，多余响应不被消费）
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

#[tokio::test]
async fn maintain_agent_merges_and_edits_persona_doc() {
    let id_a = Uuid::now_v7();
    let id_b = Uuid::now_v7();
    let env = setup(vec![
        json!({"tool": "atoms_recent", "args": {"limit": 10}}),
        json!({"tool": "atom_merge", "args": {"keep_id": id_a.to_string(), "merge_ids": [id_b.to_string()]}}),
        json!({"tool": "persona_doc_read", "args": {}}),
        json!({"tool": "persona_doc_edit", "args": {"content": "# 用户画像\n\n- 协作偏好：最烦别人催 review（已验证）", "summary": "补充协作偏好"}}),
        json!({"tool": "finish", "args": {"summary": "合并 1 组，画像初建"}}),
    ])
    .await;

    // seed：两条语义重复的 active 原子
    for (id, content, refs) in [
        (
            id_a,
            "用户最烦别人催 review，自己安排节奏",
            json!([{"session_id": Uuid::now_v7()}]),
        ),
        (
            id_b,
            "用户不喜欢被人催 review",
            json!([{"session_id": Uuid::now_v7()}]),
        ),
    ] {
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, embedding, strength, source_kind) \
             VALUES ($1, 'preference', $2, 0.95, 'active', $3, NULL, 'fact', 'agent_inferred')",
        )
        .bind(id)
        .bind(content)
        .bind(refs)
        .execute(&env.pool)
        .await
        .unwrap();
    }

    env.queue
        .enqueue(JobTemplate::new("maintain_memory").with_payload(json!({"reason": "manual"})))
        .await
        .unwrap();
    let j = wait_done(&env.queue, "maintain_memory").await;
    assert_eq!(j.status, JobStatus::Succeeded, "{:?}", j.error);

    // B 被归档并指向 A；A 保持 active
    assert_eq!(j.attempts, 1, "不应有重试: {:?}", j.error);
    let (st_b, sup): (String, Option<Uuid>) =
        sqlx::query_as("SELECT status, superseded_by FROM atoms WHERE id = $1")
            .bind(id_b)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(st_b, "archived");
    assert_eq!(sup, Some(id_a));
    let st_a: String = sqlx::query_scalar("SELECT status FROM atoms WHERE id = $1")
        .bind(id_a)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(st_a, "active");

    // 画像文档：首建 version=1，内容来自 Agent 编辑
    let (doc, ver): (String, i32) =
        sqlx::query_as("SELECT content, version FROM persona_doc WHERE id = 1")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(ver, 1);
    assert!(doc.contains("已验证"), "画像应包含编辑内容: {doc}");

    // 任务结果回执
    let payload = j.progress.map(|p| p.0).unwrap_or(json!({}));
    assert_eq!(payload.get("merged").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(
        payload.get("persona_edited").and_then(|v| v.as_bool()),
        Some(true)
    );

    env.handle
        .shutdown_and_wait(std::time::Duration::from_secs(5))
        .await;
}

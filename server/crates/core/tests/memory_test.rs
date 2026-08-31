//! 记忆域集成测试：context_pack 的 L1 相关性（R3）。
//!
//! 验证 L1 补充在「有 query」时按语义相关排序（走 search_atoms），
//! 而不是无差别按 hit_count 排序。

mod support;

use agent_memory_core::memory::{MemoryError, MemoryService};
use agent_memory_llm::{KeyCipher, ProviderRegistry};
use agent_memory_search::tokenize::tsv_text;
use sqlx::PgPool;
use uuid::Uuid;

async fn setup() -> (PgPool, MemoryService, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");
    let registry = ProviderRegistry::new(
        pool.clone(),
        KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
    );
    let svc = MemoryService::new(pool.clone(), registry);
    (pool, svc, container)
}

async fn insert_atom(pool: &PgPool, content: &str, hit_count: i32) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, needs_review, embedding, tsv, hit_count) \
         VALUES ($1, 'fact', $2, 0.9, 'active', '[]'::jsonb, false, NULL, to_tsvector('simple', $3), $4)",
    )
    .bind(id)
    .bind(content)
    .bind(tsv_text(content))
    .bind(hit_count)
    .execute(pool)
    .await
    .expect("插入 atom");
    id
}

#[tokio::test]
async fn context_pack_l1_is_query_relevant_not_hit_count() {
    let (pool, svc, _container) = setup().await;

    // 相关 atom（hit_count 低）与不相关 atom（hit_count 高）
    let relevant = insert_atom(&pool, "用户偏好使用 Rust 语言进行系统编程", 0).await;
    let irrelevant = insert_atom(&pool, "用户喜欢在家做中式烹饪料理", 100).await;

    let pack = svc
        .context_pack(Some("Rust"), 10, 10_000, false)
        .await
        .expect("context_pack");

    let atom_ids: Vec<Uuid> = pack.atoms.iter().map(|a| a.id).collect();

    // R3：L1 应按 query 语义相关排序——相关 atom 必须在结果中
    assert!(
        atom_ids.contains(&relevant),
        "相关 atom 应在结果中，got: {atom_ids:?}"
    );

    // 不相关但 hit_count 高的 atom 不应挤到相关 atom 之前
    if let Some(pos_irr) = atom_ids.iter().position(|id| *id == irrelevant) {
        let pos_rel = atom_ids.iter().position(|id| *id == relevant).unwrap();
        assert!(
            pos_rel < pos_irr,
            "相关 atom 应排在不相关 atom 之前：rel@{pos_rel} irr@{pos_irr}"
        );
    }
}

/// B9：检索命中异步回写 hit_count（atoms + scenarios）。
#[tokio::test]
async fn search_hits_bump_hit_count() {
    let (pool, svc, _container) = setup().await;

    let a = insert_atom(&pool, "用户偏好使用 Rust 语言进行系统编程", 0).await;

    // 场景种子（含关键词，FTS 可命中）
    let sid = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO scenarios (id, topic, summary, body, tsv) VALUES \
         ($1, '开发环境', '用户偏好 Rust', '完整描述', to_tsvector('simple', $2))",
    )
    .bind(sid)
    .bind(agent_memory_search::tokenize::tsv_text(
        "开发环境 用户偏好 Rust",
    ))
    .execute(&pool)
    .await
    .unwrap();

    let _ = svc.search("Rust", &[], 10, false).await.expect("search");

    // 回写是异步的：轮询等它落地
    let mut atom_hits = 0i32;
    let mut scen_hits = 0i32;
    for _ in 0..50 {
        atom_hits = sqlx::query_scalar("SELECT hit_count FROM atoms WHERE id = $1")
            .bind(a)
            .fetch_one(&pool)
            .await
            .unwrap();
        scen_hits = sqlx::query_scalar("SELECT hit_count FROM scenarios WHERE id = $1")
            .bind(sid)
            .fetch_one(&pool)
            .await
            .unwrap();
        if atom_hits >= 1 && scen_hits >= 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(atom_hits >= 1, "atoms.hit_count 应回写（实际 {atom_hits}）");
    assert!(
        scen_hits >= 1,
        "scenarios.hit_count 应回写（实际 {scen_hits}）"
    );

    // context_pack 读路径同样计数（有 query 时 L1 走 search_atoms）
    let before = atom_hits;
    let _ = svc
        .context_pack(Some("Rust"), 10, 10_000, false)
        .await
        .unwrap();
    let mut after = before;
    for _ in 0..50 {
        after = sqlx::query_scalar("SELECT hit_count FROM atoms WHERE id = $1")
            .bind(a)
            .fetch_one(&pool)
            .await
            .unwrap();
        if after > before {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        after > before,
        "context_pack 命中应回写（{before} → {after}）"
    );
}

#[tokio::test]
async fn update_atom_can_clear_needs_review() {
    let (pool, svc, _container) = setup().await;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, needs_review, tsv) \
         VALUES ($1, 'fact', '低置信事实', 0.5, 'candidate', '[]'::jsonb, true, to_tsvector('simple', $2))",
    )
    .bind(id)
    .bind(tsv_text("低置信事实"))
    .execute(&pool)
    .await
    .unwrap();

    // 通过：清人审标记，其余不动
    let a = svc
        .update_atom(id, None, None, None, Some(false), None, None, None)
        .await
        .unwrap();
    assert!(!a.needs_review, "人审通过应清 needs_review");
    assert_eq!(a.status, "candidate");
    assert_eq!(a.content, "低置信事实");
}

/// 实体透镜进 context_pack：AI 冷启动能看到用户世界里的人与事。
/// 有 query 按 token 相关命中，无 query 按密度头部。
#[tokio::test]
async fn context_pack_carries_entity_lenses() {
    let (_pool, svc, _container) = setup().await;

    let zhang = svc
        .create_entity("张三", "person", "同事，负责后端")
        .await
        .unwrap();
    let _cook = svc.create_entity("烹饪", "topic", "").await.unwrap();

    // 有 query：token 相关——「张三」命中人物实体
    let pack = svc
        .context_pack(Some("张三"), 10, 10_000, false)
        .await
        .unwrap();
    assert!(
        pack.entities.iter().any(|e| e.id == zhang.id),
        "query 张三 → 实体透镜应含张三，实得 {:?}",
        pack.entities
            .iter()
            .map(|e| e.name.clone())
            .collect::<Vec<String>>()
    );

    // 无 query：密度头部（两实体密度同为 0 时取 updated_at 头部，非空即可）
    let pack = svc.context_pack(None, 10, 10_000, false).await.unwrap();
    assert!(!pack.entities.is_empty(), "无 query → 实体透镜应有密度头部");
}

/// 议题一 b：append_session 增量写——pending 可追加、已蒸馏拒绝、agent 可补记。
#[tokio::test]
async fn append_session_semantics() {
    let (_pool, svc, _container) = setup().await;
    let s = svc
        .write_session(
            "pi-ext",
            serde_json::json!([{"speaker":"user","text":"第一轮"}]),
            "off",
        )
        .await
        .unwrap();

    // pending 可追加，轮次合并
    let s2 = svc
        .append_session(
            s.id,
            serde_json::json!([{"speaker":"assistant","text":"收到"},{"speaker":"user","text":"继续"}]),
            None,
            "off",
        )
        .await
        .unwrap();
    assert_eq!(
        s2.content.as_array().map(|a| a.len()),
        Some(3),
        "追加后 3 轮"
    );
    assert_eq!(s2.agent, "pi-ext", "不传 agent 保持原值");

    // agent 补记（COALESCE）
    let s3 = svc
        .append_session(
            s.id,
            serde_json::json!([{"speaker":"user","text":"第四轮"}]),
            Some("pi-claude"),
            "off",
        )
        .await
        .unwrap();
    assert_eq!(s3.agent, "pi-claude", "补记 agent 应生效");

    // 已蒸馏拒绝
    sqlx::query("UPDATE raw_sessions SET distill_status = 'done' WHERE id = $1")
        .bind(s.id)
        .execute(&_pool)
        .await
        .unwrap();
    let err = svc
        .append_session(
            s.id,
            serde_json::json!([{"speaker":"user","text":"迟到"}]),
            None,
            "off",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, MemoryError::BadRequest(_)), "已蒸馏应 400");
}

/// 议题四：实体级遗忘——挂链 active 原子归档、非 active 不动、实体+墓碑消失。
#[tokio::test]
async fn forget_entity_archives_linked_atoms() {
    let (pool, svc, _container) = setup().await;
    let e = svc.create_entity("小王", "person", "").await.unwrap();
    let a1 = insert_atom(&pool, "小王 pace 偏慢", 0).await;
    let a2 = insert_atom(&pool, "小王 也要去骑行", 0).await;
    // 预置一条已归档的挂链原子（不应被二次动）
    let a3 = insert_atom(&pool, "小王 的旧记录", 0).await;
    sqlx::query("UPDATE atoms SET status = 'archived' WHERE id = $1")
        .bind(a3)
        .execute(&pool)
        .await
        .unwrap();
    for a in [a1, a2, a3] {
        svc.attach_atom(e.id, a).await.unwrap();
    }

    let n = svc.forget_entity(e.id).await.unwrap();
    assert_eq!(n, 2, "只归档 2 条 active（archived 不动）");

    for a in [a1, a2] {
        let st: String = sqlx::query_scalar("SELECT status FROM atoms WHERE id = $1")
            .bind(a)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(st, "archived");
    }
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM entities")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "实体应删除");
}

/// 议题二：直写带事件时间；correction 手动补 superseded_by 取代链。
#[tokio::test]
async fn atom_time_and_supersede_chain() {
    let (_pool, svc, _container) = setup().await;
    let old = svc
        .create_atom("fact", "张三的生日是 3 月 5 日", 0.9, None, None)
        .await
        .unwrap();
    assert!(old.occurred_at.is_none());

    let occ = "2026-09-02T00:00:00Z"
        .parse::<chrono::DateTime<chrono::Utc>>()
        .unwrap();
    let new = svc
        .create_atom(
            "correction",
            "张三的生日是 3 月 15 日",
            0.95,
            Some(occ),
            None,
        )
        .await
        .unwrap();
    assert_eq!(new.occurred_at, Some(occ), "直写应携带事件时间");

    // 手动 correction：归档旧原子并补取代链
    let archived = svc
        .update_atom(
            old.id,
            None,
            None,
            Some("archived"),
            None,
            Some(new.id),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(archived.status, "archived");
    assert_eq!(archived.superseded_by, Some(new.id), "取代链应指向新原子");
}

/// 议题三：context_pack 带人审队列（代问）；no_feedback 不刷热度。
#[tokio::test]
async fn context_pack_pending_review_and_no_feedback() {
    let (pool, svc, _container) = setup().await;
    let hot = insert_atom(&pool, "用户偏好深色主题", 50).await;

    // 造一条待人审原子
    let pr = svc
        .create_atom("fact", "待确认：张三生日 3 月 15 日", 0.3, None, None)
        .await
        .unwrap();
    assert!(pr.needs_review);

    let pack = svc
        .context_pack(Some("张三"), 10, 10_000, false)
        .await
        .unwrap();
    assert!(
        pack.pending_review.iter().any(|a| a.id == pr.id),
        "人审队列应进 context_pack 供 AI 代问"
    );

    // no_feedback=true：hit_count 不涨
    let before: i32 = sqlx::query_scalar("SELECT hit_count FROM atoms WHERE id = $1")
        .bind(hot)
        .fetch_one(&pool)
        .await
        .unwrap();
    let _ = svc
        .context_pack(Some("深色主题"), 10, 10_000, true)
        .await
        .unwrap();
    let after: i32 = sqlx::query_scalar("SELECT hit_count FROM atoms WHERE id = $1")
        .bind(hot)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after, "no_feedback 不应刷热度");
}

/// P12：并发 append 不丢更新——原子 jsonb 拼接在行级串行。
#[tokio::test]
async fn concurrent_append_keeps_all_turns() {
    let (_pool, svc, _container) = setup().await;
    let s = svc
        .write_session(
            "pi",
            serde_json::json!([{"speaker":"user","text":"第1轮"}]),
            "off",
        )
        .await
        .unwrap();

    let a = svc.clone();
    let b = svc.clone();
    let sid = s.id;
    let (r1, r2) = tokio::join!(
        a.append_session(
            sid,
            serde_json::json!([{"speaker":"user","text":"并发A"}]),
            None,
            "off"
        ),
        b.append_session(
            sid,
            serde_json::json!([{"speaker":"user","text":"并发B"}]),
            None,
            "off"
        ),
    );
    r1.unwrap();
    r2.unwrap();

    let after = svc.get_session(sid).await.unwrap();
    assert_eq!(
        after.content.as_array().map(|x| x.len()),
        Some(3),
        "两路并发 append 都应落库（原子拼接），实得 {}",
        after.content
    );
}

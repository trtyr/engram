//! 记忆域集成测试：context_pack 的 L1 相关性（R3）。
//!
//! 验证 L1 补充在「有 query」时按语义相关排序（走 search_atoms），
//! 而不是无差别按 hit_count 排序。

mod support;

use agent_memory_core::memory::MemoryService;
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
        .context_pack(Some("Rust"), 10, 10_000)
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

    let _ = svc.search("Rust", &[], 10).await.expect("search");

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
    let _ = svc.context_pack(Some("Rust"), 10, 10_000).await.unwrap();
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
        .update_atom(id, None, None, None, Some(false))
        .await
        .unwrap();
    assert!(!a.needs_review, "人审通过应清 needs_review");
    assert_eq!(a.status, "candidate");
    assert_eq!(a.content, "低置信事实");
}

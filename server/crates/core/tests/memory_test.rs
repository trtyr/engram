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
        let pos_rel = atom_ids
            .iter()
            .position(|id| *id == relevant)
            .unwrap();
        assert!(
            pos_rel < pos_irr,
            "相关 atom 应排在不相关 atom 之前：rel@{pos_rel} irr@{pos_irr}"
        );
    }
}

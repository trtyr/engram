//! R6 rerank：LLM 精排生效 / LLM 失败降级原序 / 默认关零 LLM 调用。

mod support;

use engram_core::unified::UnifiedSearch;
use engram_distill::llm_port::MockLlm;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

async fn setup() -> (support::TestPg, PgPool) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    (container, pool)
}

fn tsv_text(s: &str) -> String {
    // 与 memory 域写入同口径的简化分词（空格分隔足够 FTS 命中）
    s.to_lowercase()
}

async fn insert_atom(pool: &PgPool, content: &str) -> uuid::Uuid {
    let id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, needs_review, embedding, tsv, hit_count) \
         VALUES ($1, 'fact', $2, 0.9, 'active', '[]'::jsonb, false, NULL, to_tsvector('simple', $3), 0)",
    )
    .bind(id)
    .bind(content)
    .bind(tsv_text(content))
    .execute(pool)
    .await
    .unwrap();
    id
}

fn mk_search(pool: PgPool, mock: Arc<MockLlm>) -> UnifiedSearch {
    let cipher = engram_llm::KeyCipher::from_hex_master(&"00".repeat(32)).unwrap();
    let registry = engram_llm::ProviderRegistry::new(pool.clone(), cipher);
    UnifiedSearch::new(pool, registry, "/tmp/unified-rerank-test", mock)
}

#[tokio::test]
async fn rerank_true_reorders_by_llm() {
    let (_c, pool) = setup().await;
    let a = insert_atom(&pool, "apple banana cherry").await; // RRF 首位（更早命中序）
    let b = insert_atom(&pool, "banana cherry date").await;

    // Mock：order [1, 0]——LLM 认为第二候选更相关
    let mock = Arc::new(MockLlm::with_chats(vec![
        serde_json::json!({"order": [1, 0]}),
    ]));
    let svc = mk_search(pool.clone(), Arc::clone(&mock));

    let hits = svc.search("banana", 5, true).await.unwrap();
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert_eq!(hits[0].id, b, "LLM 精排后 b 应在前");
    assert_eq!(hits[1].id, a);
    assert!(
        !mock.sent_user.lock().unwrap().is_empty(),
        "rerank 应发起 LLM 调用"
    );
}

#[tokio::test]
async fn rerank_llm_failure_falls_back_to_original_order() {
    let (_c, pool) = setup().await;
    let a = insert_atom(&pool, "apple banana cherry").await;
    let b = insert_atom(&pool, "banana cherry date").await;

    // Mock：垃圾输出（解析败）→ 降级原序
    let mock = Arc::new(MockLlm::with_chats(vec![serde_json::json!(
        "完全不是 JSON"
    )]));
    let svc = mk_search(pool.clone(), mock);

    let hits = svc.search("banana", 5, true).await.unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, a, "降级应保持 RRF 原序");
    assert_eq!(hits[1].id, b);
}

#[tokio::test]
async fn rerank_default_off_makes_no_llm_call() {
    let (_c, pool) = setup().await;
    insert_atom(&pool, "apple banana cherry").await;

    let mock = Arc::new(MockLlm::with_chats(vec![]));
    let svc = mk_search(pool.clone(), Arc::clone(&mock));

    let hits = svc.search("banana", 5, false).await.unwrap();
    assert!(!hits.is_empty());
    assert!(
        mock.sent_user.lock().unwrap().is_empty(),
        "默认关不应有任何 LLM 调用"
    );
    let _ = Duration::ZERO;
    let _: HashMap<String, String> = HashMap::new();
}

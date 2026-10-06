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

/// MockLlm.embed 同款确定性伪向量（hash 混合，文本形态）——查询文本的向量即此。
fn mock_vec(text: &str) -> String {
    let h: usize = text.chars().map(|c| c as usize).sum();
    let dim = engram_distill::llm_port::embedding_dimensions() as usize;
    let v: Vec<f32> = (0..dim)
        .map(|i| ((h.wrapping_mul(i + 7)) % 97 + 1) as f32 / 98.0)
        .collect();
    format!(
        "[{}]",
        v.iter()
            .map(|f| f.to_string())
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// 第二候选向量：0.9 ×（查询向量）+ 0.2 × e2——dist ≈ 0.024，
/// 严格大于 a 的 0、小于天花板——a 先 b 后的序确定可断言。
fn second_vec(base: &str) -> String {
    let h: usize = text_hash(base);
    let dim = engram_distill::llm_port::embedding_dimensions() as usize;
    let v: Vec<f32> = (0..dim)
        .map(|i| {
            let base_v = ((h.wrapping_mul(i + 7)) % 97 + 1) as f32 / 98.0;
            let e2 = if i == 1 { 0.2 } else { 0.0 };
            0.9 * base_v + e2
        })
        .collect();
    format!(
        "[{}]",
        v.iter()
            .map(|f| f.to_string())
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn text_hash(text: &str) -> usize {
    text.chars().map(|c| c as usize).sum()
}

async fn insert_atom(pool: &PgPool, content: &str, vec_text: &str) -> uuid::Uuid {
    let id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, embedding, hit_count) \
         VALUES ($1, 'fact', $2, 0.9, 'active', '[]'::jsonb, false, $3::vector, 0)",
    )
    .bind(id)
    .bind(content)
    .bind(vec_text)
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
    let a = insert_atom(&pool, "apple banana cherry", &mock_vec("banana")).await; // 与查询同向量 → dist=0 稳定首位
    let b = insert_atom(&pool, "banana cherry date", &second_vec("banana")).await; // dist≈0.024 次位

    // Mock：order [0, 1]——纯向量下输入序即 [b, a]（b 伪向量距离更近），LLM 确认 b 最相关
    let mock = Arc::new(MockLlm::with_chats(vec![
        serde_json::json!({"order": [0, 1]}),
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
    let a = insert_atom(&pool, "apple banana cherry", &mock_vec("banana")).await; // dist=0 → 稳定首位
    let b = insert_atom(&pool, "banana cherry date", &second_vec("banana")).await; // dist≈0.024 次位

    // Mock：垃圾输出（解析败）→ 降级原序
    let mock = Arc::new(MockLlm::with_chats(vec![serde_json::json!(
        "完全不是 JSON"
    )]));
    let svc = mk_search(pool.clone(), mock);

    let hits = svc.search("banana", 5, true).await.unwrap();
    for h in &hits {
        eprintln!(
            "[rerank-debug] hit {} score={}",
            &h.id.to_string()[..8],
            h.score
        );
    }
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, b, "降级应保持输入原序（纯向量序：b 距离更近）");
    assert_eq!(hits[1].id, a);
}

#[tokio::test]
async fn rerank_default_off_makes_no_llm_call() {
    let (_c, pool) = setup().await;
    insert_atom(&pool, "apple banana cherry", &mock_vec("banana")).await;

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

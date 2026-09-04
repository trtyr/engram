//! 跨域统一检索集成测试（R1）。
//!
//! 验证 `/search` 背后的 UnifiedSearch 真的融合 memory + wiki 两域：
//! 三域各插一条含关键词的数据，一次查询应返回三域合并命中（带域标签）。

mod support;

use engram_core::unified::UnifiedSearch;
use engram_llm::{KeyCipher, ProviderRegistry};
use engram_search::tokenize::tsv_text;
use uuid::Uuid;

async fn setup() -> (sqlx::PgPool, UnifiedSearch, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool)
        .await
        .expect("迁移");
    let registry = ProviderRegistry::new(
        pool.clone(),
        KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
    );
    let dir = tempfile::tempdir().unwrap();
    let svc = UnifiedSearch::new(pool.clone(), registry, dir.keep());
    (pool, svc, container)
}

#[tokio::test]
async fn unified_search_fuses_three_domains() {
    let (pool, svc, _container) = setup().await;

    // 1. memory：atom
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, needs_review, embedding, tsv) \
         VALUES ($1, 'fact', '用户偏好使用 Rust 编程', 0.9, 'active', '[]'::jsonb, false, NULL, to_tsvector('simple', $2))",
    )
    .bind(Uuid::now_v7())
    .bind(tsv_text("用户偏好使用 Rust 编程"))
    .execute(&pool)
    .await
    .unwrap();

    // 2. wiki：document + chunk
    let doc_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO wiki_documents (id, title, source_uri, sha256, status) VALUES ($1, 'Rust 文档', 'rust.md', $2, 'ready')",
    )
    .bind(doc_id)
    .bind(format!("sha-{}", Uuid::now_v7()))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO wiki_chunks (id, document_id, seq, content, embed_failed, tsv) \
         VALUES ($1, $2, 0, 'Rust 语言的内存安全特性', false, to_tsvector('simple', $3))",
    )
    .bind(Uuid::now_v7())
    .bind(doc_id)
    .bind(tsv_text("Rust 语言的内存安全特性"))
    .execute(&pool)
    .await
    .unwrap();

    // 3. wiki：page
    sqlx::query(
        "INSERT INTO wiki_pages (id, slug, title, page_type, content, frontmatter, origin, version, tsv) \
         VALUES ($1, 'rust-page', 'Rust', 'concept', 'Rust 是一门系统编程语言', '{}'::jsonb, 'llm', 1, to_tsvector('simple', $2))",
    )
    .bind(Uuid::now_v7())
    .bind(tsv_text("Rust 是一门系统编程语言"))
    .execute(&pool)
    .await
    .unwrap();

    // 4. entity：Rust 异步主题实体（名字含 Rust → token 命中）
    sqlx::query("INSERT INTO entities (id, name, kind, summary) VALUES ($1, 'Rust 异步', 'topic', '持续学习主题')")
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await
        .unwrap();

    // 统一检索
    let hits = svc.search("Rust", 20).await.expect("统一检索");

    assert!(!hits.is_empty(), "统一检索应返回命中");

    let domains: std::collections::HashSet<&str> = hits.iter().map(|h| h.domain.as_str()).collect();
    assert!(
        domains.contains("memory"),
        "应包含 memory 域命中，got: {domains:?}"
    );
    assert!(
        domains.contains("wiki"),
        "应包含 wiki 域命中，got: {domains:?}"
    );
    assert!(
        domains.contains("entity"),
        "应包含 entity 域命中（实体进统一检索），got: {domains:?}"
    );

    // 每个命中都带域标签 + 非零分数（域内 rank 归一化后）
    for h in &hits {
        assert!(!h.domain.is_empty(), "域标签非空");
        assert!(h.score > 0.0, "分数应为正（RRF 归一化）");
    }
}

//! Wiki 级联删除集成测试（R4 事务）。

mod support;

use agent_memory_wiki_engine::WikiService;
use uuid::Uuid;

async fn setup() -> (sqlx::PgPool, WikiService, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");
    let registry = agent_memory_llm::ProviderRegistry::new(
        pool.clone(),
        agent_memory_llm::KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
    );
    (
        pool.clone(),
        WikiService::new(pool, registry),
        container,
    )
}

#[tokio::test]
async fn cascade_delete_removes_source_and_downstream() {
    let (pool, svc, _container) = setup().await;

    // 直接构造：一个 source + 一个摘要页（唯一来源）+ 一个共享 concept 页
    let source_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO wiki_sources (id, sha256, raw_path, title, status) \
         VALUES ($1, $2, '/tmp/x.md', '测试源', 'ready')",
    )
    .bind(source_id)
    .bind(format!("sha-{}", Uuid::now_v7()))
    .execute(&pool)
    .await
    .unwrap();

    // 摘要页（page_type=source，唯一来源 → 整页删）
    sqlx::query(
        "INSERT INTO wiki_pages (id, slug, title, page_type, content, frontmatter, origin, version) \
         VALUES ($1, 'src-page', '摘要', 'source', '# 摘要', $2, 'llm', 1)",
    )
    .bind(Uuid::now_v7())
    .bind(sqlx::types::Json(serde_json::json!({
        "title": "摘要", "page_type": "source", "sources": [source_id.to_string()]
    })))
    .execute(&pool)
    .await
    .unwrap();

    // 共享 concept 页（引用该 source，含指向摘要页的 wikilink）
    let concept_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO wiki_pages (id, slug, title, page_type, content, frontmatter, origin, version) \
         VALUES ($1, 'concept-page', '概念', 'concept', '参见 [[src-page]]', $2, 'llm', 1)",
    )
    .bind(concept_id)
    .bind(sqlx::types::Json(serde_json::json!({
        "title": "概念", "page_type": "concept", "sources": [source_id.to_string()]
    })))
    .execute(&pool)
    .await
    .unwrap();

    // 级联删除
    let report = svc
        .delete_source_cascade(source_id)
        .await
        .expect("级联删除");

    // 摘要页整页删、共享页摘源、dead link 清理
    assert!(report.deleted_pages.contains(&"src-page".to_string()));
    assert!(report.updated_shared.contains(&"concept-page".to_string()));
    assert_eq!(report.cleaned_links, 1, "concept 页里的 [[src-page]] 应被清理");

    // 验证 DB 状态（事务提交后三表一致）
    let source_exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM wiki_sources WHERE id = $1")
            .bind(source_id)
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert!(source_exists.is_none(), "source 应被删除");

    let src_page: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM wiki_pages WHERE slug = 'src-page'")
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert!(src_page.is_none(), "摘要页应被删除");

    let concept_sources: i32 =
        sqlx::query_scalar("SELECT jsonb_array_length(frontmatter->'sources') FROM wiki_pages WHERE id = $1")
            .bind(concept_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(concept_sources, 0, "共享页 sources 应摘除该 source");
}

// ---------- W5：级联删除不留幽灵边（事务内删边 + 无据边回收 + 权重重算） ----------

#[tokio::test]
async fn w5_cascade_cleans_dangling_and_baseless_edges() {
    let (pool, wiki, _pg) = setup().await;

    let sid = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO wiki_sources (id, sha256, raw_path, title, status) VALUES ($1, $2, '/tmp/w5.md', 'W5源', 'ready')")
        .bind(sid)
        .bind(format!("sha-w5-{}", uuid::Uuid::now_v7().simple()))
        .execute(&pool)
        .await
        .unwrap();

    async fn mk_page(
        pool: &sqlx::PgPool,
        slug: &str,
        pt: &str,
        sources: serde_json::Value,
    ) {
        let id = uuid::Uuid::now_v7();
        sqlx::query(
            "INSERT INTO wiki_pages (id, slug, title, page_type, content, frontmatter, origin, tsv) \
             VALUES ($1, $2, $2, $3, $4, $5::jsonb, 'llm', NULL)",
        )
        .bind(id)
        .bind(slug)
        .bind(pt)
        .bind(format!("# {slug}\n\n内容。"))
        .bind(sources)
        .execute(pool)
        .await
        .unwrap();
    }
    // 源摘要页（独源→整删）+ 两个共享页（摘源后不再有任何关联依据）
    mk_page(&pool, "w5-src", "source", serde_json::json!({"sources": [sid.to_string()]})).await;
    mk_page(&pool, "w5-a", "concept", serde_json::json!({"sources": [sid.to_string()]})).await;
    mk_page(&pool, "w5-b", "concept", serde_json::json!({"sources": [sid.to_string()]})).await;

    // 源重叠边生成（三页两两无向双插，与真实 ingest 相同路径）
    agent_memory_wiki_engine::relevance::rebuild_weights(&pool)
        .await
        .unwrap();
    let edges_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM wiki_links WHERE from_slug LIKE 'w5-%' OR to_slug LIKE 'w5-%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(edges_before >= 4, "源重叠边应已生成: {edges_before}");

    // 级联删除
    let report = wiki.delete_source_cascade(sid).await.unwrap();
    assert_eq!(report.deleted_pages, vec!["w5-src".to_string()]);
    assert_eq!(report.updated_shared.len(), 2, "两个共享页摘源");

    // 幽灵边：不得存在任何指向 w5-src 的边
    let dangling: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM wiki_links WHERE from_slug = 'w5-src' OR to_slug = 'w5-src'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(dangling, 0, "删页后不得有悬空边");

    // 无据边：w5-a 与 w5-b 已无共享源也无内容互链 → 边必须被回收
    let baseless: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM wiki_links \
         WHERE (from_slug = 'w5-a' AND to_slug = 'w5-b') OR (from_slug = 'w5-b' AND to_slug = 'w5-a')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(baseless, 0, "摘源后无据边应被回收");
}

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
    (pool.clone(), WikiService::new(pool), container)
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

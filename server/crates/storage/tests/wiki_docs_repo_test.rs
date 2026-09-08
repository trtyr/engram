//! repo::wiki_docs 多库隔离验证（0037）：wiki_documents / wiki_chunks 的全部 SQL
//! 按 library_id 过滤/写入——含库内 sha 去重、跨库读改删互不命中、检索不串库。

mod support;

use engram_storage::repo::wiki_docs as repo;
use sqlx::PgPool;
use uuid::Uuid;

async fn setup() -> PgPool {
    let container = support::start_pgvector().await.expect("测试库");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    pool
}

async fn lib_id(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM wiki_libraries WHERE slug = $1")
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn wiki_docs_repo_scopes_by_library() {
    let pool = setup().await;
    let main = lib_id(&pool, "main").await;
    let lib_b = Uuid::new_v4();
    sqlx::query("INSERT INTO wiki_libraries (id, slug, name) VALUES ($1, 'proj-a', 'A 库')")
        .bind(lib_b)
        .execute(&pool)
        .await
        .unwrap();

    // K6：库内 sha 去重（uniq_wiki_documents_lib_sha），跨库同 sha 各自独立
    let sha = "deadbeef";
    let id_main = Uuid::new_v4();
    assert_eq!(
        repo::insert_document_sha(&pool, main, id_main, "主库文档", "u", None, "", sha)
            .await
            .unwrap(),
        Some(id_main)
    );
    let id_b = Uuid::new_v4();
    assert_eq!(
        repo::insert_document_sha(&pool, lib_b, id_b, "B 库文档", "u", None, "", sha)
            .await
            .unwrap(),
        Some(id_b)
    );
    assert_eq!(
        repo::insert_document_sha(&pool, main, Uuid::new_v4(), "重复", "u", None, "", sha)
            .await
            .unwrap(),
        None,
        "同库重复 sha 应幂等命中"
    );
    assert_eq!(
        repo::find_document_id_by_sha(&pool, main, sha)
            .await
            .unwrap(),
        id_main
    );
    assert_eq!(
        repo::find_document_id_by_sha(&pool, lib_b, sha)
            .await
            .unwrap(),
        id_b
    );

    // 读路径库隔离
    assert!(
        repo::get_document(&pool, main, id_main)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        repo::get_document(&pool, lib_b, id_main)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        repo::get_document(&pool, main, Uuid::new_v4())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        repo::get_document_status(&pool, lib_b, id_main)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        repo::list_documents(&pool, main, None, None, 50)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        repo::list_documents(&pool, main, Some("pending"), None, 50)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        repo::list_documents(&pool, main, Some("ready"), None, 50)
            .await
            .unwrap()
            .len(),
        0
    );

    // 管道状态推进 + 读源 + 抓取产物落盘：跨库更新不得命中
    assert_eq!(
        repo::update_document_status(&pool, main, id_main, "parsing")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        repo::update_document_status(&pool, lib_b, id_main, "parsing")
            .await
            .unwrap(),
        0,
        "跨库状态推进不应命中"
    );
    assert!(
        repo::get_document_source(&pool, main, id_main)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        repo::get_document_source(&pool, lib_b, id_main)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        repo::update_document_fetch_result(
            &pool,
            main,
            id_main,
            "/tmp/x.html",
            Some("text/html"),
            Some("标题")
        )
        .await
        .unwrap(),
        1
    );

    // 块写入与读隔离（重跑覆盖同 (document_id, seq)）
    let c0 = Uuid::new_v4();
    repo::insert_chunk(
        &pool,
        main,
        c0,
        id_main,
        0,
        "Rust 长期记忆",
        "rust 长期 记忆",
    )
    .await
    .unwrap();
    let c1 = Uuid::new_v4();
    repo::insert_chunk(
        &pool,
        main,
        c1,
        id_main,
        1,
        "向量检索降级 FTS",
        "向量 检索 降级",
    )
    .await
    .unwrap();
    repo::insert_chunk(
        &pool,
        lib_b,
        Uuid::new_v4(),
        id_b,
        0,
        "Rust 长期记忆",
        "rust 长期 记忆",
    )
    .await
    .unwrap();
    assert_eq!(repo::count_chunks(&pool, main, id_main).await.unwrap(), 2);
    assert_eq!(repo::count_chunks(&pool, lib_b, id_main).await.unwrap(), 0);
    assert_eq!(
        repo::list_chunks(&pool, main, id_main, 10)
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        repo::list_chunks(&pool, lib_b, id_main, 10)
            .await
            .unwrap()
            .len(),
        0
    );
    repo::insert_chunk(
        &pool,
        main,
        Uuid::new_v4(),
        id_main,
        0,
        "覆盖后内容",
        "覆盖 内容",
    )
    .await
    .unwrap();
    assert_eq!(
        repo::count_chunks(&pool, main, id_main).await.unwrap(),
        2,
        "重跑同 seq 应覆盖不新增"
    );

    // K8：缺失块 + 向量写入/失败标记（跨库写不命中）
    assert_eq!(
        repo::missing_chunks(&pool, main, id_main)
            .await
            .unwrap()
            .len(),
        2
    );
    repo::set_chunk_embedding(&pool, main, c0, vec![0.5f32; 1024])
        .await
        .unwrap();
    repo::set_chunk_failed(&pool, main, c1).await.unwrap();
    let left = repo::missing_chunks(&pool, main, id_main).await.unwrap();
    assert_eq!(left.len(), 1, "已嵌入块不再缺失，失败块仍缺");
    assert_eq!(left[0].0, c1);

    // K1 自愈：非终态卡死文档库内重置；跨库不命中
    sqlx::query("UPDATE wiki_documents SET status = 'parsing', updated_at = now() - interval '10 minutes' WHERE id = $1")
        .bind(id_main)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        repo::heal_stuck_document(&pool, main, id_main)
            .await
            .unwrap(),
        Some(id_main)
    );
    sqlx::query("UPDATE wiki_documents SET status = 'parsing', updated_at = now() - interval '10 minutes' WHERE id = $1")
        .bind(id_main)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        repo::heal_stuck_document(&pool, lib_b, id_main)
            .await
            .unwrap(),
        None,
        "跨库自愈不应命中"
    );

    // 混合检索：库内命中且不串库
    let hits_main = repo::search_chunks(&pool, main, "覆盖 & 内容", None, 10)
        .await
        .unwrap();
    assert!(!hits_main.is_empty(), "主库 FTS 应命中");
    assert!(hits_main.iter().all(|h| h.document_id == id_main));
    let hits_b = repo::search_chunks(&pool, lib_b, "rust & 记忆", None, 10)
        .await
        .unwrap();
    assert!(!hits_b.is_empty(), "B 库同内容应独立命中");
    assert!(
        hits_b.iter().all(|h| h.document_id == id_b),
        "B 库只命中 B 库文档"
    );
    assert!(
        repo::search_chunks(&pool, lib_b, "fts", None, 10)
            .await
            .unwrap()
            .is_empty(),
        "主库独有内容不串入 B 库"
    );

    // 删除：返回落盘路径 + 级联清块；跨库删除不命中
    assert_eq!(
        repo::delete_document_returning_path(&pool, lib_b, id_main)
            .await
            .unwrap(),
        None,
        "跨库删除不应命中"
    );
    assert_eq!(
        repo::delete_document_returning_path(&pool, main, id_main)
            .await
            .unwrap(),
        Some("/tmp/x.html".into())
    );
    assert!(
        repo::get_document(&pool, main, id_main)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        repo::count_chunks(&pool, main, id_main).await.unwrap(),
        0,
        "级联清块"
    );
    assert_eq!(
        repo::delete_document_quiet(&pool, lib_b, id_b)
            .await
            .unwrap(),
        1
    );
}

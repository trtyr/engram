//! wiki_promotions 集成验证（EN-59）：登记 / UNIQUE 防重 / 按项目查询 / 级联清理。
//!
//! 前置依赖真实外键：wiki_libraries（0037 迁移保证 main）+ projects + project_docs——
//! setup 里手工造最小行。project_docs 行删除时 DB 级 CASCADE 清登记（0047 外键语义）。

mod support;

use engram_storage::repo::wiki_promotions as repo;
use sqlx::PgPool;
use uuid::Uuid;

async fn setup() -> PgPool {
    let container = support::start_pgvector().await.expect("测试库");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    pool
}

/// 造最小 project + project_doc，返回 (project_id, doc_id)。
async fn mk_project_doc(pool: &PgPool, proj_name: &str) -> (Uuid, Uuid) {
    let project_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO projects (id, name, type, status, categories) \
         VALUES ($1, $2, 'dev', 'active', '[\"总览\"]'::jsonb)",
    )
    .bind(project_id)
    .bind(proj_name)
    .execute(pool)
    .await
    .unwrap();
    let doc_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO project_docs (id, project_id, category, title, content) \
         VALUES ($1, $2, '架构与实现', '测试文档', '正文内容')",
    )
    .bind(doc_id)
    .bind(project_id)
    .execute(pool)
    .await
    .unwrap();
    (project_id, doc_id)
}

async fn main_lib(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM wiki_libraries WHERE slug = 'main'")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn promote_register_insert_list_and_unique_guard() {
    let pool = setup().await;
    let lib = main_lib(&pool).await;
    let (project_id, doc_id) = mk_project_doc(&pool, "晋升源项目").await;

    // ① 首次登记成功
    let id = repo::insert(
        &pool,
        lib,
        "pass-through-principle",
        project_id,
        doc_id,
        "§机器产出",
    )
    .await
    .expect("首次登记应成功");

    // ② 同 (project, doc, page) 重复登记 → Conflict（服务层转「已晋升」友好提示）
    let dup = repo::insert(
        &pool,
        lib,
        "pass-through-principle",
        project_id,
        doc_id,
        "§机器产出",
    )
    .await;
    assert!(dup.is_err(), "重复登记应被 UNIQUE 拒绝");
    assert!(
        matches!(dup.unwrap_err(), engram_storage::StoreError::Conflict(_)),
        "冲突应映射 StoreError::Conflict"
    );

    // ③ 按项目查询：恰好 1 条且字段完整
    let rows = repo::list_by_project(&pool, project_id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id);
    assert_eq!(rows[0].page_slug, "pass-through-principle");
    assert_eq!(rows[0].anchor, "§机器产出");

    // ④ 同页多来源：另一个文档晋升到同一页 → 按页列出 2 条
    let (_p2, doc2) = mk_project_doc(&pool, "第二个来源项目").await;
    repo::insert(&pool, lib, "pass-through-principle", _p2, doc2, "§另一处")
        .await
        .unwrap();
    let by_page = repo::list_by_page(&pool, lib, "pass-through-principle")
        .await
        .unwrap();
    assert_eq!(by_page.len(), 2, "同页可承接多个来源文档的晋升");
}

#[tokio::test]
async fn doc_delete_cascades_registration() {
    let pool = setup().await;
    let lib = main_lib(&pool).await;
    let (project_id, doc_id) = mk_project_doc(&pool, "待删项目").await;
    repo::insert(&pool, lib, "some-page", project_id, doc_id, "§锚点")
        .await
        .unwrap();
    assert_eq!(
        repo::list_by_project(&pool, project_id)
            .await
            .unwrap()
            .len(),
        1
    );

    // 删 project_docs 行 → DB 级 CASCADE 清登记（0047 外键语义）
    sqlx::query("DELETE FROM project_docs WHERE id = $1")
        .bind(doc_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        repo::list_by_project(&pool, project_id)
            .await
            .unwrap()
            .is_empty(),
        "doc 删除应级联清理登记"
    );
}

#[tokio::test]
async fn delete_by_page_clears_registrations() {
    let pool = setup().await;
    let lib = main_lib(&pool).await;
    let (project_id, doc_id) = mk_project_doc(&pool, "页删除场景").await;
    repo::insert(&pool, lib, "doomed-page", project_id, doc_id, "§锚点")
        .await
        .unwrap();

    // wiki 页删除（page_slug 无 FK）→ 服务层显式清理
    let n = repo::delete_by_page(&pool, lib, "doomed-page")
        .await
        .unwrap();
    assert_eq!(n, 1);
    assert!(
        repo::list_by_page(&pool, lib, "doomed-page")
            .await
            .unwrap()
            .is_empty()
    );
}

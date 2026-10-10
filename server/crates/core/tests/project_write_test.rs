//! 项目写路径并发/一致性测试（P019-M4）：
//! upsert_file 事务化（FOR UPDATE 行锁）——并发覆盖写不丢更新、版本快照不重复；
//! related 关联语义无向——反向重复被拒。

mod support;

use engram_core::project::ProjectService;
use sqlx::PgPool;

async fn setup() -> (PgPool, ProjectService, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    let svc = ProjectService::new(pool.clone());
    (pool, svc, container)
}

#[tokio::test]
async fn concurrent_upsert_file_keeps_versions_consistent() {
    let (pool, svc, _pg) = setup().await;
    let p = svc.create_project("并发文件", "dev", None).await.unwrap();

    // 两路并发覆盖写同名文件（各写不同内容，多次交替）——各自持独立 service 实例
    let svc1 = ProjectService::new(pool.clone());
    let svc2 = ProjectService::new(pool.clone());
    let pid = p.id;
    let (r1, r2) = tokio::join!(
        async move {
            for i in 0..5 {
                svc1.upsert_file(
                    pid,
                    "report.html",
                    Some("text/html"),
                    &format!("<h1>A{i}</h1>"),
                )
                .await
                .unwrap();
            }
            "A"
        },
        async move {
            for i in 0..5 {
                svc2.upsert_file(
                    pid,
                    "report.html",
                    Some("text/html"),
                    &format!("<h1>B{i}</h1>"),
                )
                .await
                .unwrap();
            }
            "B"
        },
    );
    assert_eq!((r1, r2), ("A", "B"));

    // 主行版本 = 快照条数（无重复版本、无孤儿快照）
    let (final_version,): (i32,) = sqlx::query_as(
        "SELECT version FROM project_files WHERE project_id = $1 AND name = 'report.html'",
    )
    .bind(pid)
    .fetch_one(&pool)
    .await
    .unwrap();
    let snaps: Vec<i32> = sqlx::query_as(
        "SELECT f.version FROM project_file_versions f \
         JOIN project_files pf ON pf.id = f.file_id \
         WHERE pf.project_id = $1 AND pf.name = 'report.html' ORDER BY f.version",
    )
    .bind(pid)
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .map(|(v,): (i32,)| v)
    .collect();
    assert_eq!(
        snaps.len(),
        snaps.iter().collect::<std::collections::HashSet<_>>().len(),
        "版本快照不得重复: {snaps:?}"
    );
    assert_eq!(
        final_version as usize,
        snaps.len() + 1,
        "主行 version 应等于 快照数+1（v1 无快照）: version={final_version} snaps={snaps:?}"
    );
}

#[tokio::test]
async fn related_link_is_reverse_deduped() {
    let (_pool, svc, _pg) = setup().await;
    let a = svc.create_project("甲", "dev", None).await.unwrap();
    let b = svc.create_project("乙", "dev", None).await.unwrap();

    svc.add_link(a.id, b.id, "related", "")
        .await
        .expect("首建 related 应成功");
    let err = svc
        .add_link(b.id, a.id, "related", "")
        .await
        .expect_err("反向重复 related 应被拒");
    assert!(
        matches!(err, engram_core::project::ProjectError::Conflict(_)),
        "应报 Conflict: {err:?}"
    );

    // 有向 kind 不受影响：part_of 反向是另一条语义
    svc.add_link(b.id, a.id, "part_of", "")
        .await
        .expect("part_of 反向合法");
}

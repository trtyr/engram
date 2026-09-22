//! PG17 干净库全迁移演练（公网多Agent P001-t9，上云迁移就绪）。
//!
//! 运行方式（默认 #[ignore]，不进全量门禁）：
//! ```bash
//! docker run -d --name pg17-drill -e POSTGRES_PASSWORD=drill17 \
//!   -e POSTGRES_DB=engram -p 127.0.0.1:5433:5432 pgvector/pgvector:pg17
//! AM_PG17_URL='postgres://postgres:drill17@127.0.0.1:5433/engram' \
//!   cargo test -p engram-api --test pg17_drill_test -- --ignored --nocapture
//! ```
//! 上云时腾讯云 PG17 建库后同样的 run_migrations 会重放这套断言。

mod support;

use sqlx::Row;

#[tokio::test]
#[ignore = "PG17 演练入口：设 AM_PG17_URL 后 --ignored 运行（docker pgvector/pgvector:pg17）"]
async fn pg17_full_migrations_and_schema() {
    let url = std::env::var("AM_PG17_URL").expect("演练需设 AM_PG17_URL（PG17 干净库）");
    let pool = engram_storage::connect_pool(&engram_storage::PoolConfig::new(&url))
        .await
        .expect("连接 PG17 干净库");

    // 全部迁移（0001-0054，编译期嵌入）
    engram_storage::run_migrations(&pool)
        .await
        .expect("PG17 全量迁移应通过");

    // 迁移版本 = 最新（0058：资产域 + 位置真引用 + 项目关联）
    let version = engram_storage::current_version(&pool)
        .await
        .expect("读迁移版本");
    assert_eq!(version, Some(58), "PG17 迁移应推进到 0058");

    // 关键表抽查：各域表 + 公网加固 / 收录哲学线新表。
    // 例外：wiki_libraries 的 main 库由 0046 迁移幂等补建——干净库迁移后恰 1 行是正确行为。
    for table in [
        "admin_sessions",
        "api_keys",
        "raw_sessions",
        "atoms",
        "projects",
        "project_docs",
        "wiki_pages",
        "wiki_promotions",
        "wiki_query_log",
        "skills",
        "todos",
        "cg_projects",
        "kv_entries",
    ] {
        let n: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|e| panic!("表 {table} 应存在（PG17）：{e}"));
        assert_eq!(n, 0, "干净库 {table} 应为空");
    }
    let lib_count: i64 = sqlx::query_scalar("SELECT count(*) FROM wiki_libraries")
        .fetch_one(&pool)
        .await
        .expect("wiki_libraries 应存在");
    assert_eq!(lib_count, 1, "0046 迁移应幂等补建默认 main 库恰 1 行");

    // 0049 会话身份列 / 0050 文档乐观锁 / 0051 codegraph 上传列 / 0052 会话上下文列
    for (table, col) in [
        ("raw_sessions", "api_key_id"),
        ("raw_sessions", "client_ref"),
        ("project_docs", "version"),
        ("cg_projects", "source_kind"),
        ("cg_projects", "head"),
        ("cg_projects", "produced_at"),
        ("cg_projects", "built_with_version"),
        ("cg_projects", "last_producer"),
        ("admin_sessions", "ip"),
        ("admin_sessions", "user_agent"),
    ] {
        let ok: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
             WHERE table_name = '{table}' AND column_name = '{col}')"
        ))
        .fetch_one(&pool)
        .await
        .expect("information_schema 查询");
        assert!(ok, "列应存在：{table}.{col}");
    }

    // 0053 常识边层级模型：source 约束须已把 world_knowledge 纳入
    let src_check: String = sqlx::query_scalar(
        "SELECT pg_get_constraintdef(oid) FROM pg_constraint \
         WHERE conname = 'entity_relations_source_check'",
    )
    .fetch_one(&pool)
    .await
    .expect("0053 应重建 entity_relations_source_check");
    assert!(
        src_check.contains("world_knowledge"),
        "0053 应把 world_knowledge 纳入 source 约束：{src_check}"
    );

    // 0054 查询日志表：唯一键 (library_id, query) 与缺口索引须已建
    let qlog_idx: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = 'idx_wiki_query_log_gaps')",
    )
    .fetch_one(&pool)
    .await
    .expect("pg_indexes 查询");
    assert!(qlog_idx, "0054 应建 idx_wiki_query_log_gaps");

    let pgver: String = sqlx::query_scalar("SHOW server_version")
        .fetch_one(&pool)
        .await
        .expect("读 PG 版本");
    let row = sqlx::query("SELECT count(*) AS n FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    let applied: i64 = row.get("n");
    println!("PG17 演练完成：server_version={pgver}，迁移 {applied} 条全部通过，版本={version:?}");
}

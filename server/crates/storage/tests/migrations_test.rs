//! testcontainers 集成测试：验证全部迁移在干净 PG 上可应用、核心对象存在。

mod support;

#[tokio::test]
async fn migrations_apply_on_clean_pgvector() {
    let container = support::start_pgvector().await.expect("启动 pgvector 容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接容器");

    // 干净库跑迁移
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移执行");

    // 版本可查（当前 15 份迁移：0015 = entities 实体层）
    let version = agent_memory_storage::current_version(&pool).await.unwrap();
    assert_eq!(version, Some(21), "0001-0021 迁移应已应用");

    // pgvector 扩展真实可用
    let v: String = sqlx::query_scalar("SELECT '[1,2,3]'::vector::text")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(v, "[1,2,3]");

    // 幂等：重复执行不报错
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移幂等重放");
}

#[tokio::test]
async fn all_domain_tables_exist_with_columns() {
    let container = support::start_pgvector().await.expect("启动 pgvector 容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接容器");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移执行");

    // 全部域表存在性
    let expected_tables = [
        "jobs",
        "job_events",
        "llm_providers",
        "llm_usage",
        "api_keys",
        "admin_sessions",
        "raw_sessions",
        "atoms",
        "scenarios",
        "persona_aspects",
        "documents",
        "chunks",
        "wiki_sources",
        "wiki_pages",
        "wiki_links",
        "cg_projects",
        "settings",
        "wiki_review_items",
        "wiki_insight_dismissals",
    ];
    for table in expected_tables {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = $1)",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap_or(false);
        assert!(exists, "表 {table} 应存在");
    }

    // 抽查关键列：atoms.embedding 是 vector(1024)（D0010）
    let (udt_name, att_typmod): (String, i32) = sqlx::query_as(
        "SELECT t.typname, a.atttypmod FROM pg_attribute a \
         JOIN pg_class c ON c.oid = a.attrelid \
         JOIN pg_type t ON t.oid = a.atttypid \
         WHERE c.relname = 'atoms' AND a.attname = 'embedding'",
    )
    .fetch_one(&pool)
    .await
    .expect("atoms.embedding 列应存在");
    assert_eq!(udt_name, "vector");
    assert_eq!(att_typmod, 1024, "vector 维度应为 1024");

    // 抽查约束：jobs 状态机 CHECK 生效
    let result = sqlx::query(
        "INSERT INTO jobs (id, kind, status) VALUES (gen_random_uuid(), 'test', 'bogus')",
    )
    .execute(&pool)
    .await;
    assert!(result.is_err(), "非法 job 状态应被 CHECK 拒绝");

    // idempotency_key 唯一约束生效
    sqlx::query("INSERT INTO jobs (id, kind, idempotency_key) VALUES ($1, 'test', 'dup-key')")
        .bind(uuid::Uuid::new_v4())
        .execute(&pool)
        .await
        .unwrap();
    let dup =
        sqlx::query("INSERT INTO jobs (id, kind, idempotency_key) VALUES ($1, 'test', 'dup-key')")
            .bind(uuid::Uuid::new_v4())
            .execute(&pool)
            .await;
    assert!(dup.is_err(), "重复 idempotency_key 应被唯一约束拒绝");
}

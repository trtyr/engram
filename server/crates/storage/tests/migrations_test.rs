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

    // 版本可查（当前 25 份迁移：0025 = wiki folder 目录树层级）
    let version = agent_memory_storage::current_version(&pool).await.unwrap();
    assert_eq!(version, Some(25), "0001-0025 迁移应已应用");

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

#[tokio::test]
async fn migration_0022_splits_multi_model_provider() {
    let container = support::start_pgvector().await.expect("启动 pgvector 容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接容器");

    // 建「旧格式」llm_providers（迁移 0021 之前：models jsonb 数组）
    sqlx::query(
        "CREATE TABLE llm_providers (\
            id uuid PRIMARY KEY, name text NOT NULL UNIQUE, base_url text NOT NULL, \
            api_key_encrypted bytea NOT NULL, models jsonb NOT NULL DEFAULT '[]', \
            is_default boolean NOT NULL DEFAULT false, \
            created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now())",
    )
    .execute(&pool)
    .await
    .unwrap();

    // 旧数据：1 个 provider，2 个 model（chat + embedding）
    sqlx::query(
        "INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, models, is_default) \
         VALUES ($1, 'newapi', 'http://127.0.0.1:1', decode('ab','hex'), $2::jsonb, true)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(
        r#"[{"id": "MiniMax-M3", "capabilities": ["chat"]}, {"id": "bge-m3", "capabilities": ["embedding"]}]"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    // 执行 0022 的拆分逻辑（与 0022_provider_single_model.sql 一致）
    sqlx::query(
        "ALTER TABLE llm_providers \
            ADD COLUMN model_id text NOT NULL DEFAULT '', \
            ADD COLUMN capability text NOT NULL DEFAULT 'chat'",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TEMP TABLE _split_providers AS \
         SELECT p.name, p.base_url, p.api_key_encrypted, p.is_default, p.created_at, p.updated_at, \
            m.e->>'id' AS model_id, c.cap AS capability, \
            row_number() OVER (PARTITION BY p.id ORDER BY (c.cap = 'chat') DESC, m.e->>'id', c.cap) AS rn \
         FROM llm_providers p \
         CROSS JOIN LATERAL jsonb_array_elements(p.models) AS m(e) \
         CROSS JOIN LATERAL jsonb_array_elements_text(m.e->'capabilities') AS c(cap) \
         WHERE jsonb_array_length(p.models) > 1",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM llm_providers WHERE jsonb_array_length(models) > 1")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, model_id, capability, is_default, created_at, updated_at) \
         SELECT gen_random_uuid(), \
            CASE WHEN rn = 1 THEN name ELSE name || '-' || capability END, \
            base_url, api_key_encrypted, model_id, capability, is_default, created_at, updated_at \
         FROM _split_providers",
    )
    .execute(&pool)
    .await
    .unwrap();

    // 断言：1 provider × 2 model → 2 行（拆分前后模型数一致，无损）
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT name, model_id, capability FROM llm_providers ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 2, "1 provider 2 model 应拆成 2 行: {rows:?}");
    assert!(
        rows.iter()
            .any(|(n, m, c)| n == "newapi" && m == "MiniMax-M3" && c == "chat"),
        "chat 模型应保留原名: {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|(n, m, c)| n == "newapi-embedding" && m == "bge-m3" && c == "embedding"),
        "embedding 模型应拆成 name-capability 后缀: {rows:?}"
    );
}

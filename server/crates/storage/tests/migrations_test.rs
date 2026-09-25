//! testcontainers 集成测试：验证全部迁移在干净 PG 上可应用、核心对象存在。

mod support;

#[tokio::test]
async fn migrations_apply_on_clean_pgvector() {
    let container = support::start_pgvector().await.expect("启动 pgvector 容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接容器");

    // 干净库跑迁移
    engram_storage::run_migrations(&pool)
        .await
        .expect("迁移执行");

    // 版本可查（当前 59 份迁移：0059 = 实体同名唯一表达式索引 lower(btrim(name)),kind——EN-242 异形分档 DB 层兜底）
    let version = engram_storage::current_version(&pool).await.unwrap();
    assert_eq!(version, Some(59), "0001-0059 迁移应已应用");

    // 0056（代码图谱入口收敛）：dest_mode 列形态——NOT NULL + 落库默认 default + 二值 CHECK。
    // 历史行回填 custom 是迁移的语义保证（生产库实测见《代码图谱入口收敛 · roadmap》）；
    // 本测试锁的是「干净库上的列形态」，防止后续迁移把它改回去。
    let (not_null, default_expr): (bool, Option<String>) = sqlx::query_as(
        "SELECT a.attnotnull, pg_get_expr(d.adbin, d.adrelid) FROM pg_attribute a \
         LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
         WHERE a.attrelid = 'cg_projects'::regclass AND a.attname = 'dest_mode'",
    )
    .fetch_one(&pool)
    .await
    .expect("cg_projects.dest_mode 应存在");
    assert!(not_null, "dest_mode 应 NOT NULL");
    assert_eq!(
        default_expr.as_deref(),
        Some("'default'::text"),
        "新注册不传值时落 default"
    );
    let bad = sqlx::query(
        "INSERT INTO cg_projects (id, name, path, source_uri, dest_mode) \
         VALUES (gen_random_uuid(), 'zz-bad', '/tmp/zz', 'file:///zz', 'bogus')",
    )
    .execute(&pool)
    .await;
    assert!(bad.is_err(), "越界 dest_mode 应被 CHECK 拒绝");

    // 0057（项目场景扩类）：projects.type 值域 = 六场景；越界被拒（本测试锁干净库上的约束形态）
    let def: String = sqlx::query_scalar(
        "SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conname = 'projects_type_check'",
    )
    .fetch_one(&pool)
    .await
    .expect("projects_type_check 应存在");
    for t in ["dev", "ops", "research", "study", "life", "create"] {
        assert!(def.contains(&format!("'{t}'")), "场景值域应含 {t}：{def}");
    }
    let ok = sqlx::query(
        "INSERT INTO projects (id, name, type) VALUES (gen_random_uuid(), 'zz-ops', 'ops')",
    )
    .execute(&pool)
    .await;
    assert!(ok.is_ok(), "ops 场景应被接受");
    let bad_type = sqlx::query(
        "INSERT INTO projects (id, name, type) VALUES (gen_random_uuid(), 'zz-bogus', 'bogus')",
    )
    .execute(&pool)
    .await;
    assert!(bad_type.is_err(), "越界 type 应被 CHECK 拒绝");

    // 0058（资产域）：assets 表形态 + project_locations.asset_id 真引用（RESTRICT 而非 CASCADE）
    let def: String = sqlx::query_scalar(
        "SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conname = 'assets_kind_check'",
    )
    .fetch_one(&pool)
    .await
    .expect("assets_kind_check 应存在");
    for k in ["host", "cloud", "domain", "account", "device", "other"] {
        assert!(
            def.contains(&format!("'{k}'")),
            "资产类型值域应含 {k}：{def}"
        );
    }
    let ok_asset = sqlx::query(
        "INSERT INTO assets (id, kind, name, aliases, ip, os) \
         VALUES (gen_random_uuid(), 'host', 'zz-asset', ARRAY['zz-alias'], '10.0.0.1', 'testOS')",
    )
    .execute(&pool)
    .await;
    assert!(ok_asset.is_ok(), "合法资产应可写入");
    let dup_asset = sqlx::query(
        "INSERT INTO assets (id, kind, name) VALUES (gen_random_uuid(), 'host', 'zz-asset')",
    )
    .execute(&pool)
    .await;
    assert!(dup_asset.is_err(), "同名资产应撞唯一索引");
    let bad_kind = sqlx::query(
        "INSERT INTO assets (id, kind, name) VALUES (gen_random_uuid(), 'bogus', 'zz-bad')",
    )
    .execute(&pool)
    .await;
    assert!(bad_kind.is_err(), "越界 asset kind 应被 CHECK 拒绝");

    // 引用列：RESTRICT——删还被项目引用的资产必须显式解绑，不许静默断链
    let (fk_def, del_action): (String, String) = sqlx::query_as(
        "SELECT pg_get_constraintdef(c.oid), c.confdeltype::text FROM pg_constraint c \
          WHERE c.conrelid = 'project_locations'::regclass AND c.contype = 'f' \
            AND pg_get_constraintdef(c.oid) LIKE '%assets%'",
    )
    .fetch_one(&pool)
    .await
    .expect("project_locations → assets 外键应存在");
    assert!(fk_def.contains("asset_id"), "{fk_def}");
    assert_eq!(del_action, "r", "删除动作应为 RESTRICT（confdeltype='r'）");

    // 0058（项目关联）：project_links 形态——kind 二值 CHECK、自环被拒、同向同类唯一
    let link_def: String = sqlx::query_scalar(
        "SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conname = 'project_links_kind_check'",
    )
    .fetch_one(&pool)
    .await
    .expect("project_links_kind_check 应存在");
    for k in ["part_of", "related"] {
        assert!(
            link_def.contains(&format!("'{k}'")),
            "关联值域应含 {k}：{link_def}"
        );
    }
    sqlx::query("DELETE FROM projects WHERE name IN ('zz-link-a', 'zz-link-b')")
        .execute(&pool)
        .await
        .expect("清残留项目");
    sqlx::query(
        "INSERT INTO projects (id, name, type) \
         VALUES (gen_random_uuid(), 'zz-link-a', 'dev'), (gen_random_uuid(), 'zz-link-b', 'dev')",
    )
    .execute(&pool)
    .await
    .expect("造两条关联测试项目");
    let link_ok = sqlx::query(
        "INSERT INTO project_links (id, from_project, to_project, kind) \
         SELECT gen_random_uuid(), a.id, b.id, 'part_of' FROM projects a, projects b \
          WHERE a.name = 'zz-link-a' AND b.name = 'zz-link-b'",
    )
    .execute(&pool)
    .await;
    assert!(link_ok.is_ok(), "合法关联应可写入");
    let link_dup = sqlx::query(
        "INSERT INTO project_links (id, from_project, to_project, kind) \
         SELECT gen_random_uuid(), a.id, b.id, 'part_of' FROM projects a, projects b \
          WHERE a.name = 'zz-link-a' AND b.name = 'zz-link-b'",
    )
    .execute(&pool)
    .await;
    assert!(link_dup.is_err(), "同向同类重复应撞唯一索引");
    let link_self = sqlx::query(
        "INSERT INTO project_links (id, from_project, to_project, kind) \
         SELECT gen_random_uuid(), a.id, a.id, 'part_of' FROM projects a WHERE a.name = 'zz-link-a'",
    )
    .execute(&pool)
    .await;
    assert!(link_self.is_err(), "自环应被 CHECK 拒绝");
    let link_bad_kind = sqlx::query(
        "INSERT INTO project_links (id, from_project, to_project, kind) \
         SELECT gen_random_uuid(), a.id, b.id, 'bogus' FROM projects a, projects b \
          WHERE a.name = 'zz-link-a' AND b.name = 'zz-link-b'",
    )
    .execute(&pool)
    .await;
    assert!(link_bad_kind.is_err(), "越界 kind 应被 CHECK 拒绝");

    // pgvector 扩展真实可用
    let v: String = sqlx::query_scalar("SELECT '[1,2,3]'::vector::text")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(v, "[1,2,3]");

    // 幂等：重复执行不报错
    engram_storage::run_migrations(&pool)
        .await
        .expect("迁移幂等重放");
}

#[tokio::test]
async fn all_domain_tables_exist_with_columns() {
    let container = support::start_pgvector().await.expect("启动 pgvector 容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接容器");
    engram_storage::run_migrations(&pool)
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
        "wiki_documents",
        "wiki_chunks",
        "wiki_sources",
        "wiki_pages",
        "wiki_links",
        "cg_projects",
        "settings",
        "wiki_review_items",
        "wiki_insight_dismissals",
        "projects",
        "project_locations",
        "project_docs",
        "skills",
        "skill_revisions",
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

    // 执行 0022 的拆分逻辑（与 0022_provider_split.sql 一致）
    // 注意：TEMP TABLE 是会话级对象，PgPool 每次 execute 可能拿到不同连接——
    // 必须包在同一个事务里保证三个语句共用一条连接，否则 CI 并发下必挂 42P01。
    let mut tx = pool.begin().await.unwrap();
    sqlx::query(
        "ALTER TABLE llm_providers \
            ADD COLUMN model_id text NOT NULL DEFAULT '', \
            ADD COLUMN capability text NOT NULL DEFAULT 'chat'",
    )
    .execute(&mut *tx)
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
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("DELETE FROM llm_providers WHERE jsonb_array_length(models) > 1")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, model_id, capability, is_default, created_at, updated_at) \
         SELECT gen_random_uuid(), \
            CASE WHEN rn = 1 THEN name ELSE name || '-' || capability END, \
            base_url, api_key_encrypted, model_id, capability, is_default, created_at, updated_at \
         FROM _split_providers",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

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

// EN-57：wiki 默认库 main 由迁移保证存在——空库跑迁移后必有 main；
// 已有 main（slug 冲突）的库重复应用零影响（id/name 不动）。
#[tokio::test]
async fn default_main_library_guaranteed_and_idempotent() {
    let container = support::start_pgvector().await.expect("启动 pgvector 容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接容器");

    engram_storage::run_migrations(&pool)
        .await
        .expect("迁移执行");

    // 空库跑完全部迁移 → main 存在（0037 建名「主库」；0046 对已有行 DO NOTHING
    // 不覆盖——name 是用户可改字段，迁移不钉死）
    let (slug, name): (String, String) =
        sqlx::query_as("SELECT slug, name FROM wiki_libraries WHERE slug = 'main'")
            .fetch_one(&pool)
            .await
            .expect("迁移后应存在 main 库");
    assert_eq!(slug, "main");
    assert!(!name.is_empty(), "main 应有名字");
    let id_before: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM wiki_libraries WHERE slug = 'main'")
            .fetch_one(&pool)
            .await
            .unwrap();

    // 幂等：再次执行全部迁移（或单独重放 0046 的 INSERT）不破坏已有行
    engram_storage::run_migrations(&pool)
        .await
        .expect("幂等重放");
    sqlx::query(
        "INSERT INTO wiki_libraries (id, slug, name) \
         VALUES ('00000000-0000-4000-8000-000000000001', 'main', '个人知识库') \
         ON CONFLICT (slug) DO NOTHING",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (id_after, name_after): (uuid::Uuid, String) =
        sqlx::query_as("SELECT id, name FROM wiki_libraries WHERE slug = 'main'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(id_after, id_before, "幂等重放不应改变已有 main 的 id");
    assert_eq!(name_after, name, "幂等重放不应改变已有 main 的 name");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM wiki_libraries WHERE slug = 'main'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1,
        "main 恰好一行"
    );
}

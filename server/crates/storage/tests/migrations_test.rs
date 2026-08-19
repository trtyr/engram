//! testcontainers 集成测试：验证迁移在干净 PG 上可应用。
//! 需要 Docker daemon（CI ubuntu runner 自带；本地跑 `cargo test -p agent-memory-storage`）。

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

    // 版本可查
    let version = agent_memory_storage::current_version(&pool).await.unwrap();
    assert_eq!(version, Some(1), "0001 迁移应已应用");

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

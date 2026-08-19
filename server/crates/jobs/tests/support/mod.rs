//! jobs 测试基建：复用 pgvector 容器启动逻辑。

use agent_memory_jobs::JobQueue;
use sqlx::PgPool;
use testcontainers::core::WaitFor;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

pub async fn start_pgvector() -> anyhow::Result<ContainerAsync<GenericImage>> {
    let container = GenericImage::new("pgvector/pgvector", "pg17")
        .with_wait_for(WaitFor::message_on_stderr(
            "database system is ready to accept connections",
        ))
        .with_wait_for(WaitFor::seconds(5))
        .with_env_var("POSTGRES_USER", "postgres")
        .with_env_var("POSTGRES_PASSWORD", "postgres")
        .with_env_var("POSTGRES_DB", "agent_memory")
        .start()
        .await?;
    Ok(container)
}

pub async fn connection_url(container: &ContainerAsync<GenericImage>) -> anyhow::Result<String> {
    let port = container.get_host_port_ipv4(5432).await?;
    Ok(format!(
        "postgres://postgres:postgres@127.0.0.1:{port}/agent_memory"
    ))
}

pub async fn connect_with_retry(url: &str) -> anyhow::Result<PgPool> {
    for _ in 0..30 {
        if let Ok(pool) =
            agent_memory_storage::connect_pool(&agent_memory_storage::PoolConfig::new(url)).await
        {
            if sqlx::query("SELECT 1").execute(&pool).await.is_ok() {
                return Ok(pool);
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    Err(anyhow::anyhow!("pgvector 容器连接超时"))
}

/// 从队列拿回共享的池（已废弃：setup 直接返回 pool）。
pub fn pool_of(_queue: &JobQueue) -> PgPool {
    // JobQueue 内部持有池；测试里我们单独再连一个（同 URL）
    // 为避免复杂化，这里由调用方直接提供——见 PANIC 提示。
    panic!("使用 pool_from_url(url) 代替");
}

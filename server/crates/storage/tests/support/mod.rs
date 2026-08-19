//! 集成测试基建：pgvector 容器（仅测试用）。

use std::time::Duration;

use testcontainers::core::WaitFor;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

/// 启动一次性 pgvector/pgvector:pg17 容器（与生产 compose 同镜像）。
///
/// ready 策略：PG 官方镜像 init 时会先起临时实例再重启，
/// 故等待 ready 日志后再加缓冲秒数，配合连接层重试兜底。

/// 测试 PG 容器守卫：Drop 时后台 docker rm -f（防泄漏——曾累积 198 个僵尸容器压垮 daemon）。
pub struct TestPg {
    pub container: ContainerAsync<GenericImage>,
}

impl std::ops::Deref for TestPg {
    type Target = ContainerAsync<GenericImage>;
    fn deref(&self) -> &Self::Target {
        &self.container
    }
}

impl Drop for TestPg {
    fn drop(&mut self) {
        let id = self.container.id().to_string();
        let _ = std::process::Command::new("docker")
            .args(["rm", "-f", &id])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

pub async fn start_pgvector() -> anyhow::Result<TestPg> {
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
    Ok(TestPg { container })
}

/// 容器对应的连接 URL。
pub async fn connection_url(container: &ContainerAsync<GenericImage>) -> anyhow::Result<String> {
    let port = container.get_host_port_ipv4(5432).await?;
    Ok(format!(
        "postgres://postgres:postgres@127.0.0.1:{port}/agent_memory"
    ))
}

/// 带重试的连接（容器 init 完成有时序竞争）。
pub async fn connect_with_retry(url: &str) -> anyhow::Result<sqlx::PgPool> {
    let mut last_err = None;
    for _ in 0..30 {
        match agent_memory_storage::connect_pool(&agent_memory_storage::PoolConfig::new(url)).await
        {
            Ok(pool) => {
                // 真正确认可用：发一次 ping
                if sqlx::query("SELECT 1").execute(&pool).await.is_ok() {
                    return Ok(pool);
                }
            }
            Err(e) => last_err = Some(e),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(anyhow::anyhow!(
        "pgvector 容器连接超时: {}",
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "ping failed".into())
    ))
}

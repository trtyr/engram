//! PgPool 装配与连接配置。

use std::time::Duration;

use sqlx::postgres::{PgPool, PgPoolOptions};

/// 连接池配置。字段对应 env（见 api crate config），均有生产默认值。
#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub url: String,
    pub max_connections: u32,
    pub acquire_timeout: Duration,
}

impl PoolConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            max_connections: 10,
            acquire_timeout: Duration::from_secs(30),
        }
    }
}

/// 建立连接池。只负责连接，不跑迁移（迁移见 [`crate::run_migrations`]）。
pub async fn connect_pool(config: &PoolConfig) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(config.acquire_timeout)
        .connect(&config.url)
        .await
}

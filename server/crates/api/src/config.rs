//! 环境配置加载。全部变量带 `AGENT_MEMORY_` 前缀（compose 传入）。

use std::num::NonZeroU16;

/// 服务配置。
#[derive(Debug, Clone)]
pub struct Config {
    /// PostgreSQL 连接串（必填）。
    pub database_url: String,
    /// HTTP 监听端口（默认 8080）。
    pub port: u16,
    /// 管理员密码（Phase 1 鉴权用；本阶段仅透传警告）。
    #[allow(dead_code, reason = "Phase 1 鉴权消费")]
    pub admin_password: Option<String>,
    /// 密钥加密主密钥（Phase 1 LLM provider key 加密用）。
    #[allow(dead_code, reason = "Phase 1 密钥加密消费")]
    pub master_key: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("缺少必需环境变量 AGENT_MEMORY_DATABASE_URL")]
    MissingDatabaseUrl,
    #[error("环境变量 {name} 解析失败: {reason}")]
    Parse { name: &'static str, reason: String },
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let database_url = std::env::var("AGENT_MEMORY_DATABASE_URL")
            .map_err(|_| ConfigError::MissingDatabaseUrl)?;

        let port = match std::env::var("AGENT_MEMORY_PORT") {
            Ok(v) => v.parse::<NonZeroU16>().map_err(|e| ConfigError::Parse {
                name: "AGENT_MEMORY_PORT",
                reason: e.to_string(),
            })?,
            Err(_) => NonZeroU16::new(8080).expect("8080 非 0"),
        }
        .get();

        let admin_password = std::env::var("AGENT_MEMORY_ADMIN_PASSWORD")
            .ok()
            .filter(|s| !s.is_empty());
        let master_key = std::env::var("AGENT_MEMORY_MASTER_KEY")
            .ok()
            .filter(|s| !s.is_empty());

        if admin_password.is_none() {
            tracing::warn!("AGENT_MEMORY_ADMIN_PASSWORD 未设置：Phase 1 鉴权上线后将拒绝启动");
        }
        if master_key.is_none() {
            tracing::warn!("AGENT_MEMORY_MASTER_KEY 未设置：Phase 1 密钥加密上线后将拒绝启动");
        }

        Ok(Self {
            database_url,
            port,
            admin_password,
            master_key,
        })
    }
}

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
    /// 运行时数据目录（uploads/wiki-sources/codegraph）。
    pub data_dir: std::path::PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("缺少必需环境变量 AGENT_MEMORY_DATABASE_URL")]
    MissingDatabaseUrl,
    #[error("环境变量 {name} 解析失败: {reason}")]
    Parse { name: &'static str, reason: String },
    #[error(
        "AGENT_MEMORY_EMBEDDING_DIMENSIONS={configured} 与表列维度 {column} 不匹配——四张表 embedding 列建为 vector({column})，改维度需迁移改列并全量重嵌入（当前版本不支持自动重嵌入）。请保持 {column}，或先清空向量数据再迁移"
    )]
    EmbeddingDimensionMismatch { configured: u32, column: u32 },
}

/// 既有迁移写死的向量列维度（0005 atoms/scenarios、0006 chunks、0007 wiki_pages）。
pub const EMBEDDING_COLUMN_DIM: u32 = 1024;

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

        // R11 校验：配置维度 ≠ 既有表列维度时拒启（错误信息含迁移+重嵌入指引）。
        // 值本身由 engram_distill::llm_port::embedding_dimensions() 运行时读取——此处只做守门。
        if let Ok(v) = std::env::var("AGENT_MEMORY_EMBEDDING_DIMENSIONS") {
            let d = v.trim().parse::<u32>().map_err(|e| ConfigError::Parse {
                name: "AGENT_MEMORY_EMBEDDING_DIMENSIONS",
                reason: e.to_string(),
            })?;
            if d == 0 {
                return Err(ConfigError::Parse {
                    name: "AGENT_MEMORY_EMBEDDING_DIMENSIONS",
                    reason: "维度必须为正整数".into(),
                });
            }
            if d != EMBEDDING_COLUMN_DIM {
                return Err(ConfigError::EmbeddingDimensionMismatch {
                    configured: d,
                    column: EMBEDDING_COLUMN_DIM,
                });
            }
        }

        Ok(Self {
            database_url,
            port,
            admin_password,
            master_key,
            data_dir: std::env::var("AGENT_MEMORY_DATA_DIR")
                .unwrap_or_else(|_| "./data".into())
                .into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap()
    }

    /// edition 2024 起 set/remove_var 为 unsafe——测试持 env_lock 独占后再改（单线程测试进程内安全）。
    macro_rules! set_env {
        ($k:expr, $v:expr) => {
            // SAFETY: env_lock() 互斥期间无其他线程读 env
            unsafe { std::env::set_var($k, $v) }
        };
    }
    macro_rules! rm_env {
        ($k:expr) => {
            // SAFETY: env_lock() 互斥期间无其他线程读 env
            unsafe { std::env::remove_var($k) }
        };
    }

    #[test]
    fn default_dimensions_is_column_dim() {
        let _g = env_lock();
        rm_env!("AGENT_MEMORY_EMBEDDING_DIMENSIONS");
        set_env!("AGENT_MEMORY_DATABASE_URL", "postgres://x");
        let cfg = Config::from_env().unwrap();
        // 默认（未配置）应通过守门（无字段——值由 helper 消费）
        let _ = cfg;
    }

    #[test]
    fn mismatched_dimensions_refuse_to_start() {
        let _g = env_lock();
        set_env!("AGENT_MEMORY_DATABASE_URL", "postgres://x");
        set_env!("AGENT_MEMORY_EMBEDDING_DIMENSIONS", "768");
        let err = Config::from_env().unwrap_err();
        assert!(err.to_string().contains("不匹配"), "{err}");
        assert!(err.to_string().contains("重嵌入"), "{err}");
        rm_env!("AGENT_MEMORY_EMBEDDING_DIMENSIONS");
    }

    #[test]
    fn illegal_dimensions_parse_error() {
        let _g = env_lock();
        set_env!("AGENT_MEMORY_DATABASE_URL", "postgres://x");
        set_env!("AGENT_MEMORY_EMBEDDING_DIMENSIONS", "abc");
        assert!(matches!(
            Config::from_env().unwrap_err(),
            ConfigError::Parse { .. }
        ));
        set_env!("AGENT_MEMORY_EMBEDDING_DIMENSIONS", "0");
        assert!(matches!(
            Config::from_env().unwrap_err(),
            ConfigError::Parse { .. }
        ));
        rm_env!("AGENT_MEMORY_EMBEDDING_DIMENSIONS");
    }
}

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
}

/// 既有迁移写死的向量列维度（0005 atoms/scenarios、0006 chunks、0007 wiki_pages）。
/// 唯一口径：embedding 维度不可配（P018-Q002 删死参数，原 AGENT_MEMORY_EMBEDDING_DIMENSIONS
/// 守门与 llm_port 的 env 读取均已移除）；改维度需迁移改列 + 全量重嵌入。
pub const EMBEDDING_COLUMN_DIM: u32 = 1024;

impl Config {
    /// 不可失败（架构治理 task-5 分类 A：不可失败，保留并注明理由）。
    #[allow(clippy::expect_used)]
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

        // R11 校验已移除（P018-Q002）：embedding 维度不可配，见 EMBEDDING_COLUMN_DIM 注释。

        Ok(Self {
            database_url,
            port,
            admin_password,
            master_key,
            data_dir: engram_wiki_engine::data_root(),
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
    fn from_env_with_only_database_url() {
        let _g = env_lock();
        rm_env!("AGENT_MEMORY_EMBEDDING_DIMENSIONS");
        set_env!("AGENT_MEMORY_DATABASE_URL", "postgres://x");
        // 仅必填项即可启动（P018-Q002 后维度不可配，守门测试随死参数一起删）
        let _cfg = Config::from_env().unwrap();
    }
}

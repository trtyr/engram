//! 应用状态（依赖注入容器）。

use sqlx::PgPool;

/// 管理员密码（登录校验用）。
#[derive(Clone)]
pub struct AdminPassword(pub String);

/// 密钥加密主密钥。
#[derive(Clone)]
pub struct MasterKey(pub String);

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub admin_password: Option<AdminPassword>,
    pub master_key: Option<MasterKey>,
}

impl AppState {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            admin_password: None,
            master_key: None,
        }
    }

    pub fn with_admin_password(mut self, password: Option<String>) -> Self {
        self.admin_password = password.filter(|s| !s.is_empty()).map(AdminPassword);
        self
    }

    pub fn with_master_key(mut self, key: Option<String>) -> Self {
        self.master_key = key.filter(|s| !s.is_empty()).map(MasterKey);
        self
    }

    /// LLM 注册表（master_key 缺省时用占位密钥——仅查询用量等不涉密操作可用）。
    pub fn registry(&self) -> agent_memory_llm::ProviderRegistry {
        let hex = self
            .master_key
            .as_ref()
            .map(|m| m.0.clone())
            .unwrap_or_else(|| "00".repeat(32));
        let cipher = agent_memory_llm::KeyCipher::from_hex_master(&hex)
            .expect("主密钥格式恒合法（占位 64 hex）");
        agent_memory_llm::ProviderRegistry::new(self.pool.clone(), cipher)
    }
}

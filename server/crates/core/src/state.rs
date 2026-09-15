//! 应用状态（依赖注入容器）。api 与 mcp 两个适配器共用（Formerly api::state）。

use engram_storage::PgPool;

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
    pub data_dir: std::path::PathBuf,
}

impl AppState {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            admin_password: None,
            master_key: None,
            data_dir: "./data".into(),
        }
    }

    pub fn with_data_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.data_dir = dir.into();
        self
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
    pub fn registry(&self) -> engram_llm::ProviderRegistry {
        let hex = self
            .master_key
            .as_ref()
            .map(|m| m.0.clone())
            .unwrap_or_else(|| "00".repeat(32));
        let cipher =
            engram_llm::KeyCipher::from_hex_master(&hex).expect("主密钥格式恒合法（占位 64 hex）");
        engram_llm::ProviderRegistry::new(self.pool.clone(), cipher)
    }

    /// LLM 门面（R6 rerank 等轻量 LLM 调用用；master_key 缺省时占位密钥——解密类操作不可用）。
    pub fn llm(&self) -> engram_distill::llm_port::LlmRef {
        let hex = self
            .master_key
            .as_ref()
            .map(|m| m.0.clone())
            .unwrap_or_else(|| "00".repeat(32));
        let cipher =
            engram_llm::KeyCipher::from_hex_master(&hex).expect("主密钥格式恒合法（占位 64 hex）");
        engram_distill::gateway_llm(self.pool.clone(), cipher)
    }

    /// L10：占位主密钥检测——此状态下创建的 provider 密钥与后续真实密钥不兼容。
    pub fn is_placeholder_master_key(&self) -> bool {
        match &self.master_key {
            None => true,
            Some(m) => m.0 == "00".repeat(32),
        }
    }
}

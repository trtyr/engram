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
            // EN-47：此默认仅供测试（cwd 相对 ./data）——生产主链路必须显式 `.with_data_dir()` 注入
            // （见 api/main.rs 从 Config::from_env 注入）。不要把这里的默认改成 env 解析：
            // 测试进程通常不设 env，解析会让裸测试静默落到真数据根 ~/.engram/app 造成污染。
            // 静默漂移的病根已由 wiki-engine::data_root 的 WARN 收口解决。
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

    /// 公网加固（P001-t8）：弱熵主密钥检测——已设置但字符种类过少（如同一对字符
    /// 重复 32 次），公网部署下离线爆破代价接近于零。与占位检测互补（占位另判）。
    pub fn is_weak_master_key(&self) -> bool {
        match &self.master_key {
            None => false,
            Some(m) => m.0.chars().collect::<std::collections::HashSet<_>>().len() < 8,
        }
    }
}

#[cfg(test)]
mod weak_key_tests {
    use super::*;

    fn state_with(key: Option<&str>) -> AppState {
        AppState::new(engram_storage::PgPool::connect_lazy("postgres://x@127.0.0.1:1/x").unwrap())
            .with_master_key(key.map(str::to_string))
    }

    #[tokio::test]
    async fn weak_entropy_detected() {
        assert!(state_with(Some(&"ab".repeat(32))).is_weak_master_key());
        assert!(!state_with(Some(&"0123456789abcdef".repeat(4))).is_weak_master_key());
    }

    #[tokio::test]
    async fn placeholder_is_not_reported_as_weak() {
        // 占位（None / 00×32）由 is_placeholder_master_key 负责，弱检测不重复报
        assert!(!state_with(None).is_weak_master_key());
    }

    #[tokio::test]
    async fn random_hex_is_strong() {
        // 64 位 hex 理论上至少 10+ 种字符——构造 16 种字符各出现 4 次的强 key
        let strong: String = "0123456789abcdef".chars().flat_map(|c| [c; 4]).collect();
        assert_eq!(strong.len(), 64);
        assert!(!state_with(Some(&strong)).is_weak_master_key());
    }
}

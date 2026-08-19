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
}

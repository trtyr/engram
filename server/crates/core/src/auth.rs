//! 鉴权身份与 scope 模型（跨 HTTP/MCP 共用； Formerly api::auth）。
//!
//! Bearer 中间件（axum）留在 api 层；本模块只放协议无关的身份与 scope 语义，
//! 供 api 路由与 mcp 工具面共用同一套分权检查。

use uuid::Uuid;

/// 资产域 scope。
pub const SCOPES: [&str; 8] = [
    "memory",
    "wiki",
    "codegraph",
    "project",
    "skills",
    "llm",
    "erase",
    "cron",
];

/// 已认证主体。
#[derive(Debug, Clone)]
pub enum Principal {
    /// 管理员（Web UI 会话，全权限）
    Admin,
    /// API key 机器主体（限 scopes）
    ApiKey {
        key_id: Uuid,
        name: String,
        scopes: Vec<String>,
    },
}

impl Principal {
    /// 是否拥有某 scope（Admin 恒真）。
    pub fn has_scope(&self, scope: &str) -> bool {
        match self {
            Principal::Admin => true,
            Principal::ApiKey { scopes, .. } => scopes.iter().any(|s| s == scope),
        }
    }
}

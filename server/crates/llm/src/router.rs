//! purpose 路由：用途 → (provider, model) 有序回退链。
//! 规则存 settings 表（key = 'llm_routing'），未配置走默认 provider。

use sqlx::PgPool;
use std::collections::HashMap;

use crate::types::{LlmError, Purpose};

/// 单条路由规则。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct RouteRule {
    pub provider: String,
    pub model: String,
}

/// 路由表（全部 purpose）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct RoutingTable {
    /// purpose → 有序回退链（第一个为主选）
    #[serde(flatten)]
    pub routes: HashMap<String, Vec<RouteRule>>,
}

impl RoutingTable {
    /// 某 purpose 的回退链（无配置 → 空，由调用方走默认 provider）。
    pub fn chain(&self, purpose: Purpose) -> &[RouteRule] {
        self.routes
            .get(purpose.as_str())
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}

const SETTINGS_KEY: &str = "llm_routing";

/// 路由器：从 settings 表读路由表（每次现读，配置即时生效）。
#[derive(Clone)]
pub struct PurposeRouter {
    pool: PgPool,
}

impl PurposeRouter {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 测试专用：共享内部池。
    #[doc(hidden)]
    pub fn pool_for_test(&self) -> PgPool {
        self.pool.clone()
    }

    /// 当前路由表（未配置 → 空表，全部走默认 provider）。
    pub async fn table(&self) -> Result<RoutingTable, LlmError> {
        let row: Option<(sqlx::types::Json<RoutingTable>,)> =
            sqlx::query_as("SELECT value FROM settings WHERE key = $1")
                .bind(SETTINGS_KEY)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| LlmError::Transient(e.to_string()))?;
        Ok(row.map(|(j,)| j.0).unwrap_or_default())
    }

    /// 保存路由表。
    pub async fn save(&self, table: &RoutingTable) -> Result<(), LlmError> {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES ($1, $2)
             ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = now()",
        )
        .bind(SETTINGS_KEY)
        .bind(sqlx::types::Json(table))
        .execute(&self.pool)
        .await
        .map_err(|e| LlmError::Transient(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_table_roundtrip() {
        let mut table = RoutingTable::default();
        table.routes.insert(
            "extract".into(),
            vec![
                RouteRule {
                    provider: "deepseek".into(),
                    model: "deepseek-chat".into(),
                },
                RouteRule {
                    provider: "openai".into(),
                    model: "gpt-4o-mini".into(),
                },
            ],
        );
        let json = serde_json::to_string(&table).unwrap();
        let parsed: RoutingTable = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.chain(Purpose::Extract).len(), 2);
        assert_eq!(
            parsed.chain(Purpose::Persona).len(),
            0,
            "未配置的 purpose 回退默认"
        );
    }
}

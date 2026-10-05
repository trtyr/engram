//! memory 管理面动作（EN-230）：重复原子检测 / 原子归档（P015：场景/画像浏览已随场景层退役）。
//! 分发接线在 memory.rs 的 action match；scope 走 require_memory。

use super::*;

/// EN-230②：重复/近似原子检测——归一化内容完全相同的 active 原子分组。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AtomDuplicatesParams {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AtomArchiveParams {
    /// 要归档的原子 id（list_atoms 或 atom_duplicates 拿到的 UUID）
    pub id: String,
}

impl EngramMcpServer {
    pub(crate) async fn memory_atom_duplicates(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<AtomDuplicatesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p: Principal = principal_of(&ctx)?;
        require_memory(&p)?;
        let _ = params.0;
        let groups = self.svc().duplicates().await.map_err(from_memory)?;
        let items: Vec<_> = groups
            .into_iter()
            .map(|(normalized, count, ids)| {
                json!({ "normalized": normalized, "count": count, "atom_ids": ids })
            })
            .collect();
        ok_json(json!({ "count": items.len(), "groups": items }))
    }

    pub(crate) async fn memory_atom_archive(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<AtomArchiveParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p: Principal = principal_of(&ctx)?;
        require_memory(&p)?;
        let id = uuid::Uuid::parse_str(&params.0.id).map_err(|_| {
            mcp_err(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "id 不是合法 UUID：{}（用 list_atoms 或 atom_duplicates 拿原子 id）",
                    params.0.id
                ),
            )
        })?;
        let atom = self.svc().archive_atom(id).await.map_err(from_memory)?;
        ok_json(json!({
            "archived": true,
            "atom": atom,
            "hint": "已归档（archived），未删除。恢复渠道：list_atoms status=archived 可见；治理红线不变。",
        }))
    }
}

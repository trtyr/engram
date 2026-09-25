//! memory 管理面动作（EN-230）：L2 场景浏览 / L3 画像浏览 / 重复原子检测 / 原子归档。
//! 分发接线在 memory.rs 的 action match；scope 走 require_memory。

use super::*;

/// EN-230①：L2 场景浏览——不依赖 search 命中，直接翻库存。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ScenariosListParams {
    /// 可选：最多返回条数（默认 100，上限 500）
    #[serde(default)]
    pub limit: Option<i64>,
}

/// EN-230①：L3 画像浏览——分面版本史全列（persona_edit 前先看当前形态）。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PersonaGetParams {}

/// EN-230②：重复/近似原子检测——归一化内容完全相同的 active 原子分组。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AtomDuplicatesParams {}

/// EN-242：同名实体检测——大小写/首尾空白不敏感的重复分组（合并动作待设计，先可观测）。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EntityDuplicatesParams {}

impl EngramMcpServer {
    pub(crate) async fn memory_entity_duplicates(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<EntityDuplicatesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p: Principal = principal_of(&ctx)?;
        require_memory(&p)?;
        let _ = params.0;
        let groups = self.svc().entity_duplicates().await.map_err(from_memory)?;
        let items: Vec<_> = groups
            .into_iter()
            .map(|(normalized, count, ids)| {
                serde_json::json!({ "normalized": normalized, "count": count, "entity_ids": ids })
            })
            .collect();
        ok_json(serde_json::json!({
            "count": items.len(),
            "groups": items,
            "hint": "同名异档清单——合并动作（entity_merge，参照 wiki merge 主从设计）待设计落地；当前可先用 entities 看各档内容",
        }))
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AtomArchiveParams {
    /// 要归档的原子 id（list_atoms 或 atom_duplicates 拿到的 UUID）
    pub id: String,
}

impl EngramMcpServer {
    pub(crate) async fn memory_scenarios_list(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ScenariosListParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p: Principal = principal_of(&ctx)?;
        require_memory(&p)?;
        let limit = params.0.limit.unwrap_or(100).clamp(1, 500);
        let scenarios = self
            .svc()
            .list_scenarios(limit)
            .await
            .map_err(from_memory)?;
        ok_json(json!({
            "count": scenarios.len(),
            "scenarios": scenarios,
        }))
    }

    pub(crate) async fn memory_persona_get(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<PersonaGetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p: Principal = principal_of(&ctx)?;
        require_memory(&p)?;
        let _ = params.0;
        let persona = self.svc().persona().await.map_err(from_memory)?;
        ok_json(json!({
            "count": persona.len(),
            "versions": persona,
        }))
    }

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

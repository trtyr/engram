//! skills_versions 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

/// 版本列表（R 报告建议 #5：skills 快照已在存，MCP 此前未暴露）。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsVersionsParams {
    /// 技能 slug（或技能名）
    #[schemars(description = "技能 slug（或技能名）。")]
    pub slug: String,
}

/// 回滚到历史版本。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsRestoreParams {
    /// 技能 slug（或技能名）
    #[schemars(description = "技能 slug（或技能名）。")]
    pub slug: String,
    /// 目标版本 id（versions 列表返回的 id，非 rev 序号）
    #[schemars(
        description = "目标版本 id（来自 versions 列表的 id 字段）。回滚本身也留版本快照，可再滚回来。"
    )]
    pub revision_id: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SkillsDeleteParams {
    /// 目标技能 slug
    #[schemars(description = "要删除的技能 slug（级联删版本快照，不可逆）。")]
    pub slug: String,
}

#[tool_router(router = skills_versions_router)]
impl EngramMcpServer {
    /// 版本列表（R 报告建议 #5）：语义变更自动留的快照，MCP 此前不可见。
    ///
    /// 何时用：改坏前先看有哪些版本；或挑 revision_id 给 restore 回滚。
    pub(crate) async fn skills_versions(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsVersionsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        let revs = self
            .skills_svc()
            .list_revisions(&params.0.slug)
            .await
            .map_err(from_skills)?;
        // 列表不带正文全文（content_chars 决策用）；回滚走 restore（revision_id）
        let rows: Vec<serde_json::Value> = revs
            .iter()
            .map(|r| {
                json!({
                    "id": r.id, "rev": r.rev, "name": r.name,
                    "description": r.description, "tags": r.tags,
                    "origin": r.origin, "created_at": r.created_at,
                    "content_chars": r.content.chars().count(),
                })
            })
            .collect();
        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
    }

    /// 回滚到历史版本（回滚本身也留快照，可再滚回）。
    ///
    /// 何时用：一次 update 改坏后恢复。revision_id 来自 versions 列表（id 字段）。
    pub(crate) async fn skills_restore(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsRestoreParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        let rp = params.0;
        let rid = Uuid::parse_str(&rp.revision_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "revision_id 不是合法 UUID"))?;
        let s = self
            .skills_svc()
            .restore_revision(&rp.slug, rid)
            .await
            .map_err(from_skills)?;
        let mut v = slim_skill(serde_json::to_value(&s).unwrap_or(serde_json::json!({})));
        v["restored_from"] = json!(rp.revision_id);
        v["hint"] = json!("已回滚（本次回滚前状态留了 restore 快照，可再滚回）");
        ok_json(v)
    }

    /// 删除技能（级联删版本快照，不可逆）。
    ///
    /// 何时用：仅当用户明确要求删除某个技能时。不要因「内容过时」自行删除——用 skills_update 修订。
    pub(crate) async fn skills_delete(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<SkillsDeleteParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_skills(&p)?;
        self.skills_svc()
            .delete_skill(&params.0.slug)
            .await
            .map_err(from_skills)?;
        ok_json(serde_json::json!({ "deleted": params.0.slug }))
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_skills_versions() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::skills_versions_router()
}

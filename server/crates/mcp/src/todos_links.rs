//! todos_links 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct TodoLinkParams {
    /// 源条目（UUID 或 EN-<短号>）
    #[schemars(description = "源条目（UUID 或 EN-<短号>）。")]
    pub from: String,
    /// 目标条目（UUID 或 EN-<短号>）
    #[schemars(description = "目标条目（UUID 或 EN-<短号>）。")]
    pub to: String,
    /// 关联类型：blocked_by（from 被 to 阻塞）/ relates_to（相关）/ parent（to 是 from 的父项）
    #[schemars(description = "关联类型：blocked_by / relates_to / parent。")]
    pub kind: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct TodoUnlinkParams {
    /// 源条目
    #[schemars(description = "源条目（UUID 或 EN-<短号>）。")]
    pub from: String,
    /// 目标条目
    #[schemars(description = "目标条目（UUID 或 EN-<短号>）。")]
    pub to: String,
    /// 关联类型
    #[schemars(description = "关联类型：blocked_by / relates_to / parent。")]
    pub kind: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct TodoLinksParams {
    /// 目标条目（UUID 或 EN-<短号>）
    #[schemars(description = "目标条目（UUID 或 EN-<短号>）。")]
    pub id: String,
}

#[tool_router(router = todos_links_router)]
impl EngramMcpServer {
    /// todo 引用解析（EN-<短号> 或 UUID → id）。
    pub(crate) async fn todo_ref_id(&self, r: &str) -> Result<Uuid, rmcp::ErrorData> {
        let r = r.trim();
        if let Ok(id) = Uuid::parse_str(r) {
            return Ok(id);
        }
        let n = r
            .strip_prefix("EN-")
            .or_else(|| r.strip_prefix("en-"))
            .and_then(|n| n.parse::<i32>().ok())
            .ok_or_else(|| {
                mcp_err(
                    ErrorCode::INVALID_PARAMS,
                    format!("引用格式非法：{r}（UUID 或 EN-<短号>）"),
                )
            })?;
        todo_svc(&self.state)
            .find_by_ref(r)
            .await
            .map_err(from_todo)?
            .map(|d| d.id)
            .ok_or_else(|| mcp_err(ErrorCode::INVALID_PARAMS, format!("待办不存在: EN-{n}")))
    }

    /// 建立关联（link）：blocked_by / relates_to / parent 三类，幂等。
    pub(crate) async fn todo_link(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TodoLinkParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let lp = params.0;
        let from = self.todo_ref_id(&lp.from).await?;
        let to = self.todo_ref_id(&lp.to).await?;
        let created = todo_svc(&self.state)
            .link(from, to, &lp.kind)
            .await
            .map_err(from_todo)?;
        ok_json(serde_json::json!({
            "from": from, "to": to, "kind": lp.kind,
            "created": created,
            "hint": if created { "已关联" } else { "关联已存在（幂等）" },
        }))
    }

    /// 解除关联（unlink）。
    pub(crate) async fn todo_unlink(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TodoUnlinkParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let lp = params.0;
        let from = self.todo_ref_id(&lp.from).await?;
        let to = self.todo_ref_id(&lp.to).await?;
        let removed = todo_svc(&self.state)
            .unlink(from, to, &lp.kind)
            .await
            .map_err(from_todo)?;
        if !removed {
            return Err(mcp_err(ErrorCode::INVALID_PARAMS, "关联不存在"));
        }
        ok_json(serde_json::json!({"from": from, "to": to, "kind": lp.kind, "removed": true}))
    }

    /// 关联列表（links）：双向列出某条目的全部关联（含 EN-短号与方向）——「谁阻塞我」反查入口。
    pub(crate) async fn todo_links(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<TodoLinksParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_todos(&p)?;
        let id = self.todo_ref_id(&params.0.id).await?;
        let raw = todo_svc(&self.state).links(id).await.map_err(from_todo)?;
        let counts = todo_svc(&self.state)
            .list(None, None, None, None, None, None, None, None, 500)
            .await
            .map_err(from_todo)?;
        let short_of: std::collections::HashMap<Uuid, i32> =
            counts.iter().map(|t| (t.id, t.short_no)).collect();
        let items: Vec<serde_json::Value> = raw
            .iter()
            .map(|(from, to, kind, dir)| {
                serde_json::json!({
                    "from": from,
                    "from_ref": short_of.get(from).map(|n| format!("EN-{n}")).unwrap_or_default(),
                    "to": to,
                    "to_ref": short_of.get(to).map(|n| format!("EN-{n}")).unwrap_or_default(),
                    "kind": kind,
                    "direction": dir,
                })
            })
            .collect();
        ok_json(serde_json::json!({"id": id, "count": items.len(), "links": items}))
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_todos_links() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::todos_links_router()
}

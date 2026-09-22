//! projects 域 · 工作线关联切片（2026-09-22 结构红线拆分：自 projects.rs 纯搬移，零行为变化）。
//!
//! 三个方法是非 `#[tool]` 的 `impl EngramMcpServer` 固有实现（由 projects.rs::projects_tool
//! 的 match 分发调用）——搬出原 impl 块不影响 `projects_router` 的宏收集，golden 不变。

use super::*;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectLinkAddParams {
    /// 起点项目（part_of 语义下 = 子方）
    #[schemars(
        description = "起点项目 id（UUID）。与 from_project_name 至少给一个。`part_of` 语义下它是**子方**。"
    )]
    pub from_project_id: Option<String>,
    /// 起点项目名
    #[schemars(description = "起点项目名（唯一）。与 from_project_id 至少给一个。")]
    pub from_project_name: Option<String>,
    /// 终点项目（part_of 语义下 = 母方）
    #[schemars(
        description = "终点项目 id（UUID）。与 to_project_name 至少给一个。`part_of` 语义下它是**母方**。"
    )]
    pub to_project_id: Option<String>,
    /// 终点项目名
    #[schemars(description = "终点项目名（唯一）。与 to_project_id 至少给一个。")]
    pub to_project_name: Option<String>,
    /// 关联类型
    #[schemars(
        description = "关联类型：part_of = 起点**隶属**终点（子 → 母，如「数据集团-驭元 POC」part_of「上海数据集团」）；related = 相关（语义无向，存一行）。只在子工作线真有独立推进节奏时才挂 part_of（判据见《项目与资产模型 · README》§2.2）。"
    )]
    pub kind: String,
    /// 备注
    #[schemars(description = "可选：一句话说明这条关联（为什么相关 / 什么关系）。")]
    pub note: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectLinkRefParams {
    /// 关联 id（UUID）
    #[schemars(description = "关联 id（UUID，来自 links 或项目详情的 links 列表）。")]
    pub link_id: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectLinksParams {
    /// 项目 id（UUID）
    #[schemars(description = "项目 id（UUID）。与 project_name 至少给一个。")]
    pub project_id: Option<String>,
    /// 项目名
    #[schemars(description = "项目名（唯一）。与 project_id 至少给一个。")]
    pub project_name: Option<String>,
}

impl EngramMcpServer {
    /// 建一条工作线关联（`part_of` 隶属 / `related` 相关）。
    pub(crate) async fn project_link_add(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectLinkAddParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let lp = params.0;
        let from = self
            .resolve_project(&lp.from_project_id, &lp.from_project_name)
            .await?;
        let to = self
            .resolve_project(&lp.to_project_id, &lp.to_project_name)
            .await?;
        let link = self
            .svc_project()
            .add_link(from, to, &lp.kind, lp.note.as_deref().unwrap_or(""))
            .await
            .map_err(from_project)?;
        ok_json(serde_json::to_value(&link).unwrap_or(serde_json::json!({})))
    }

    /// 解绑一条工作线关联（按 link_id）。
    pub(crate) async fn project_link_delete(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectLinkRefParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let id = Uuid::parse_str(&params.0.link_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "link_id 不是合法 UUID"))?;
        self.svc_project()
            .remove_link(id)
            .await
            .map_err(from_project)?;
        ok_json(serde_json::json!({ "deleted": params.0.link_id }))
    }

    /// 列某项目的关系（两向合并：隶属 / 下属 / 相关）。
    pub(crate) async fn project_links(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectLinksParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let lp = params.0;
        let id = self
            .resolve_project(&lp.project_id, &lp.project_name)
            .await?;
        let links = self
            .svc_project()
            .list_links(id)
            .await
            .map_err(from_project)?;
        ok_json(serde_json::json!({
            "count": links.len(),
            "links": serde_json::to_value(&links).unwrap_or(serde_json::json!([])),
        }))
    }
}

//! project_locations 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectLocationAddParams {
    /// 定位项目：项目 id（UUID）
    #[schemars(description = "项目 id（UUID）。与 project_name 至少给一个。")]
    pub project_id: Option<String>,
    /// 定位项目：项目名
    #[schemars(description = "项目名（唯一）。与 project_id 至少给一个。")]
    pub project_name: Option<String>,
    /// 关联资产（台账条目：id / 名称 / 别名三态）
    #[schemars(
        description = "可选但**推荐**：关联的资产（资产 id / 台账名 / 别名三态，先 assets list 看台账）。填了它就是「真引用」——host/ip/os 不填会自动从资产带出（身份以台账为准，别在项目里重抄一遍）。"
    )]
    pub asset: Option<String>,
    /// 主机 IP（内网/公网/IPv6 均可，仅登记不校验格式）
    #[schemars(
        description = "主机 IP（内网/公网/IPv6 均可，仅登记不校验格式；本机可填 127.0.0.1）。给了 asset 且这里为空时自动取资产值。"
    )]
    pub ip: Option<String>,
    /// 主机名
    #[schemars(
        description = "主机名（如 MacBook Pro / tencent-beijing）。给了 asset 且这里为空时自动取资产台账名。"
    )]
    pub host: Option<String>,
    /// 操作系统
    #[schemars(
        description = "操作系统（macOS / Ubuntu / Windows…）。给了 asset 且这里为空时自动取资产值。"
    )]
    pub os: Option<String>,
    /// 项目在主机上的文件夹路径
    #[schemars(description = "项目在该主机上的文件夹路径（纯登记，服务端不会读取该路径）。")]
    pub path: String,
    /// 用途（开发 / 部署 / …）
    #[schemars(description = "可选：该位置的用途（开发 / 部署 / 测试机…）。")]
    pub purpose: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectLocationUpdateParams {
    /// 位置 id（UUID，来自 project_get 的 locations 列表）
    #[schemars(description = "位置 id（UUID，来自 project_get 返回的 locations 列表）。")]
    pub location_id: String,
    /// 新 IP
    #[schemars(description = "可选：新 IP。不传不改。")]
    pub ip: Option<String>,
    /// 新主机名
    #[schemars(description = "可选：新主机名。不传不改。")]
    pub host: Option<String>,
    /// 新操作系统
    #[schemars(description = "可选：新操作系统。不传不改。")]
    pub os: Option<String>,
    /// 新路径
    #[schemars(description = "可选：新路径。不传不改。")]
    pub path: Option<String>,
    /// 新用途
    #[schemars(description = "可选：新用途。不传不改。")]
    pub purpose: Option<String>,
    /// 改关联资产
    #[schemars(
        description = "可选：改关联资产（资产 id / 台账名 / 别名；传空字符串 `\"\"` = 显式解绑为纯文本位置）。不传则不动现有引用。"
    )]
    pub asset: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectLocationDeleteParams {
    /// 位置 id（UUID）
    #[schemars(description = "位置 id（UUID，来自 project_get 返回的 locations 列表）。")]
    pub location_id: String,
}

#[tool_router(router = project_locations_router)]
impl EngramMcpServer {
    /// 登记项目位置（多主机：ip / host / os / path / 用途）。
    ///
    /// 何时用：项目代码在某个主机上有了新副本/部署时登记一条。纯元数据登记制，
    /// 服务端不会读取该路径。
    pub(crate) async fn project_location_add(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectLocationAddParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let lp = params.0;
        let id = self
            .resolve_project(&lp.project_id, &lp.project_name)
            .await?;
        let (asset_id, ip, host, os) = self
            .resolve_location_asset(
                lp.asset.as_deref(),
                lp.ip.as_deref(),
                lp.host.as_deref(),
                lp.os.as_deref(),
            )
            .await?;
        let loc = self
            .svc_project()
            .add_location(
                id,
                &ip,
                &host,
                &os,
                &lp.path,
                lp.purpose.as_deref(),
                asset_id,
            )
            .await
            .map_err(from_project)?;
        ok_json(serde_json::to_value(&loc).unwrap_or(serde_json::json!({})))
    }

    /// 编辑项目位置（补丁式，不传不改）。
    ///
    /// 何时用：代码挪了目录、换了机器，更新已登记的位置。
    pub(crate) async fn project_location_update(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectLocationUpdateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let lp = params.0;
        let id = Uuid::parse_str(&lp.location_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "location_id 不是合法 UUID"))?;
        let current = self
            .svc_project()
            .get_location(id)
            .await
            .map_err(from_project)?;
        // asset 三态：给值 = 改引用；空串 = 显式解绑；不传 = 不动现有引用
        let asset_id = match lp.asset.as_deref() {
            Some("") => None,
            Some(k) => {
                self.resolve_location_asset(Some(k), None, None, None)
                    .await?
                    .0
            }
            None => current.asset_id,
        };
        let loc = self
            .svc_project()
            .update_location(
                id,
                &lp.ip.unwrap_or(current.ip),
                &lp.host.unwrap_or(current.host),
                &lp.os.unwrap_or(current.os),
                &lp.path.unwrap_or(current.path),
                lp.purpose.or(current.purpose).as_deref(),
                asset_id,
            )
            .await
            .map_err(from_project)?;
        ok_json(serde_json::to_value(&loc).unwrap_or(serde_json::json!({})))
    }

    /// 删除一条项目位置登记（不动项目本体）。
    ///
    /// 何时用：某个主机上的副本不再属于这个项目。
    pub(crate) async fn project_location_delete(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectLocationDeleteParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let id = Uuid::parse_str(&params.0.location_id)
            .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "location_id 不是合法 UUID"))?;
        self.svc_project()
            .delete_location(id)
            .await
            .map_err(from_project)?;
        ok_json(serde_json::json!({ "deleted": params.0.location_id }))
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_project_locations() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::project_locations_router()
}

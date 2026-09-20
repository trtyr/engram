//! project_files 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectFileUpsertParams {
    /// 定位项目：项目 id（UUID）
    #[schemars(description = "项目 id（UUID）。与 project_name 至少给一个。")]
    pub project_id: Option<String>,
    /// 定位项目：项目名
    #[schemars(description = "项目名（唯一）。与 project_id 至少给一个。")]
    pub project_name: Option<String>,
    /// 文件名（禁路径分隔，≤200 字符；扩展名推断 mime）
    #[schemars(
        description = "文件名（如 architecture.html、config.json）。禁路径分隔符与 ..，≤200 字符。"
    )]
    pub name: String,
    /// 文本内容（HTML/SVG/JSON/配置；≤8MB）
    #[schemars(description = "文本内容。同 name 覆盖更新（version+1，旧版进快照）。")]
    pub content: String,
    /// 可选：显式 MIME（缺省按扩展名推断）
    #[schemars(description = "可选：显式 MIME。缺省按扩展名推断（.html→text/html 等）。")]
    pub mime: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectFileRefParams {
    /// 定位项目：项目 id（UUID）
    #[schemars(description = "项目 id（UUID）。与 project_name 至少给一个。")]
    pub project_id: Option<String>,
    /// 定位项目：项目名
    #[schemars(description = "项目名（唯一）。与 project_id 至少给一个。")]
    pub project_name: Option<String>,
    /// 文件名
    #[schemars(description = "文件名（file_list 返回的 name）。")]
    pub name: String,
    /// 可选：读历史版本内容（缺省读当前）
    #[schemars(description = "可选：版本号（file_versions 列出）。传了读该版本内容而非当前。")]
    pub version: Option<i32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectFileListParams {
    /// 定位项目：项目 id（UUID）
    #[schemars(description = "项目 id（UUID）。与 project_name 至少给一个。")]
    pub project_id: Option<String>,
    /// 定位项目：项目名
    #[schemars(description = "项目名（唯一）。与 project_id 至少给一个。")]
    pub project_name: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectFileDeleteParams {
    /// 定位项目：项目 id（UUID）
    #[schemars(description = "项目 id（UUID）。与 project_name 至少给一个。")]
    pub project_id: Option<String>,
    /// 定位项目：项目名
    #[schemars(description = "项目名（唯一）。与 project_id 至少给一个。")]
    pub project_name: Option<String>,
    /// 文件名
    #[schemars(description = "要删除的文件名（不可逆；历史快照级联删）。")]
    pub name: String,
}

#[tool_router(router = project_files_router)]
impl EngramMcpServer {
    pub(crate) async fn project_file_upsert(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectFileUpsertParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let dp = params.0;
        let id = self
            .resolve_project(&dp.project_id, &dp.project_name)
            .await?;
        let f = self
            .svc_project()
            .upsert_file(id, &dp.name, dp.mime.as_deref(), &dp.content)
            .await
            .map_err(from_project)?;
        // 正文刚发送过，不回显内容本体
        ok_json(serde_json::json!({
            "id": f.id, "project_id": f.project_id, "name": f.name, "mime": f.mime,
            "version": f.version, "content_chars": f.content.chars().count(),
            "updated_at": f.updated_at,
            "hint": if f.mime == "text/html" { "text/html——Web 项目页文件区点开即渲染（iframe sandbox）" } else { "" }
        }))
    }

    pub(crate) async fn project_file_get(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectFileRefParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let dp = params.0;
        let id = self
            .resolve_project(&dp.project_id, &dp.project_name)
            .await?;
        if let Some(v) = dp.version {
            let content = self
                .svc_project()
                .get_file_version(id, &dp.name, v)
                .await
                .map_err(from_project)?;
            return ok_json(serde_json::json!({
                "name": dp.name, "version": v, "content": content
            }));
        }
        let f = self
            .svc_project()
            .get_file(id, &dp.name)
            .await
            .map_err(from_project)?;
        let mut v = serde_json::to_value(&f).unwrap_or(serde_json::json!({}));
        // EN-68②：bytes = UTF-8 字节数（content_chars 是字符数，中文 1:3 差异大）
        if let Some(obj) = v.as_object_mut()
            && let Some(c) = obj.get("content").and_then(|x| x.as_str())
        {
            obj.insert("bytes".into(), serde_json::json!(c.len()));
        }
        ok_json(v)
    }

    pub(crate) async fn project_file_list(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectFileListParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let dp = params.0;
        let id = self
            .resolve_project(&dp.project_id, &dp.project_name)
            .await?;
        let files = self
            .svc_project()
            .list_files(id)
            .await
            .map_err(from_project)?;
        ok_json(serde_json::json!(
            files
                .iter()
                .map(|f| serde_json::json!({
                    "name": f.name, "mime": f.mime, "version": f.version,
                    "content_chars": f.content.chars().count(), "bytes": f.content.len(),
                    "updated_at": f.updated_at
                }))
                .collect::<Vec<_>>()
        ))
    }

    pub(crate) async fn project_file_delete(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<ProjectFileDeleteParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_project(&p)?;
        let dp = params.0;
        let id = self
            .resolve_project(&dp.project_id, &dp.project_name)
            .await?;
        self.svc_project()
            .delete_file(id, &dp.name)
            .await
            .map_err(from_project)?;
        ok_json(serde_json::json!({"deleted": dp.name}))
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_project_files() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::project_files_router()
}

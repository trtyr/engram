//! credentials 域 MCP 工具面（EN-234：机密值的一等台账——按名存取 + 静态加密 + 取用审计）。
//!
//! 凭据独立成域（2026-09-24 用户拍板）：值静态加密（KeyCipher），明文永不落库；
//! 列表/元数据永不回显值；按名 get 返回直接可用值且留取用审计。secrets 不进 search_all。
//! 本模块只做「语义 → core::credentials 服务」的映射，持久化在 `engram_storage::repo::credential`。

use super::*;

// ---------- 参数 ----------

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CredentialPutParams {
    /// 凭据名（唯一，按名取用的定位键）
    #[schemars(description = "凭据名（唯一，按名取用靠它定位，如 newapi/api_key、qa_token）。")]
    pub name: String,
    /// 凭据值（落库前加密）
    #[schemars(
        description = "凭据值（加密落库，明文不落库/落日志）。同名重复 put = 换值（旧取用审计清零）。"
    )]
    pub value: String,
    /// 用途说明
    #[schemars(description = "可选：用途说明（用在哪/找谁等元信息；不要把值写进说明）。")]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CredentialGetParams {
    /// 凭据名
    #[schemars(description = "凭据名。返回直接可用的值；每次取用都留审计痕（reads 可查流水）。")]
    pub name: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CredentialDeleteParams {
    /// 凭据名
    #[schemars(description = "凭据名。删除级联清取用审计，不可逆。")]
    pub name: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CredentialListParams {}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CredentialReadsParams {
    /// 凭据名
    #[schemars(
        description = "凭据名。返回该凭据的取用审计流水（谁/何时，最近在前，封顶 50 条）。"
    )]
    pub name: String,
}

// ---------- 处理器 ----------

#[tool_router(router = credentials_router)]
impl EngramMcpServer {
    /// 凭据域（EN-234）：机密值的一等台账。
    ///
    /// 何时用：要存/取 API Key、Token 等机密时——按名 get 返回直接可用的值（值静态加密落库，
    /// 每次取用留审计痕）；list 只回台账不回值。secrets 不进 search_all，明文不落任何日志。
    #[tool(
        name = "credentials",
        description = "凭据域（机密值的一等台账，EN-234）：按名存取 API Key/Token——值静态加密落库、按名 get 返回直接可用值且每次取用留审计痕、list 只回台账不回值。操作全景：action=\"help\"。"
    )]
    pub(crate) async fn credentials(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(call): Parameters<dispatch::DomainCall>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_credentials(&p)?;
        let action = call
            .args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        match action.as_str() {
            "put" => {
                let p: CredentialPutParams = dispatch::from_args("credentials", "put", call.args)?;
                let meta = self
                    .state
                    .credentials()
                    .put(&p.name, &p.value, p.description.as_deref(), "mcp")
                    .await
                    .map_err(credential_err)?;
                ok_json(json!({
                    "name": meta.name,
                    "sensitive": meta.sensitive,
                    "description": meta.description,
                    "updated_at": meta.updated_at,
                    "hint": "值已加密落库（明文不落任何日志/文档）。同名 put = 换值。取用：action=\"get\"。"
                }))
            }
            "get" => {
                let p: CredentialGetParams = dispatch::from_args("credentials", "get", call.args)?;
                let reader = format!("mcp:{}", p.name);
                let v = self
                    .state
                    .credentials()
                    .get(&p.name, &reader)
                    .await
                    .map_err(credential_err)?;
                ok_json(json!({
                    "name": v.name,
                    "value": v.value,
                    "description": v.description,
                    "read_count": v.read_count,
                    "hint": "值已解密返回（本次取用已留审计痕，reads 可查流水）。凭据明文不得落任何日志/文档/工单。"
                }))
            }
            "list" => {
                let _: CredentialListParams =
                    dispatch::from_args("credentials", "list", call.args)?;
                let items = self
                    .state
                    .credentials()
                    .list()
                    .await
                    .map_err(credential_err)?;
                ok_json(json!({
                    "count": items.len(),
                    "items": items,
                    "hint": "台账不含值——值只在 get 响应中出现。取用某条：action=\"get\" name=…"
                }))
            }
            "reads" => {
                let p: CredentialReadsParams =
                    dispatch::from_args("credentials", "reads", call.args)?;
                let rows = self
                    .state
                    .credentials()
                    .reads(&p.name)
                    .await
                    .map_err(credential_err)?;
                ok_json(json!({
                    "name": p.name,
                    "count": rows.len(),
                    "reads": rows,
                    "hint": "取用审计流水（谁/何时）。流水只记取用行为，不记值。"
                }))
            }
            "delete" => {
                let p: CredentialDeleteParams =
                    dispatch::from_args("credentials", "delete", call.args)?;
                let gone = self
                    .state
                    .credentials()
                    .delete(&p.name)
                    .await
                    .map_err(credential_err)?;
                if !gone {
                    return Err(mcp_err(
                        ErrorCode::INVALID_REQUEST,
                        format!("没有名为「{}」的凭据——credentials list 看台账", p.name),
                    ));
                }
                ok_json(json!({
                    "deleted": p.name,
                    "hint": "已删除（级联清取用审计，不可逆）。"
                }))
            }
            "help" => {
                let manual = dispatch::render_manual("credentials", &[]);
                ok_json(manual)
            }
            other => Err(dispatch::unknown_action("credentials", other)),
        }
    }
}

fn credential_err(e: engram_core::credentials::CredentialError) -> rmcp::ErrorData {
    use engram_core::credentials::CredentialError as E;
    let code = match e {
        E::NotFound(_) => ErrorCode::INVALID_REQUEST,
        E::BadRequest(_) => ErrorCode::INVALID_PARAMS,
        E::Conflict(_) => ErrorCode::INVALID_REQUEST,
        _ => ErrorCode::INTERNAL_ERROR,
    };
    mcp_err(code, e.to_string())
}

// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_credentials() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::credentials_router()
}

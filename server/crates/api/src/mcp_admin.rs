//! MCP 管理端点（Web 控制台「MCP」页）：服务信息 + 配置更新。
//! 工具面本体在 engram-mcp crate；这里只是 HTTP 壳（管理员鉴权 + 400 语义）。

use axum::Json;
use engram_mcp::McpInfo;

use crate::auth::Principal;
use crate::error::ApiError;
use crate::state::AppState;

/// MCP 服务信息（Web 控制台「MCP」页：端点、协议版本、开关状态、工具清单）。
#[utoipa::path(get, path = "/settings/mcp",
    responses((status = 200, body = McpInfo)))]
pub async fn settings_mcp(
    principal: axum::Extension<Principal>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<McpInfo>, ApiError> {
    if !matches!(principal.0, Principal::Admin) {
        return Err(ApiError::Forbidden("仅限管理员".into()));
    }
    Ok(Json(engram_mcp::build_info(&state.pool).await))
}

#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct McpConfigUpdate {
    /// 服务总开关
    pub enabled: Option<bool>,
    /// 停用工具全量清单（覆盖式；空数组 = 全部启用）。未知工具名 400。
    pub disabled_tools: Option<Vec<String>>,
}

/// 更新 MCP 配置（服务开关 / 工具粒度开关）。
#[utoipa::path(put, path = "/settings/mcp",
    request_body = McpConfigUpdate,
    responses((status = 200, body = McpInfo), (status = 400, body = crate::error::ErrorEnvelope)))]
pub async fn settings_mcp_update(
    principal: axum::Extension<Principal>,
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(req): Json<McpConfigUpdate>,
) -> Result<Json<McpInfo>, ApiError> {
    if !matches!(principal.0, Principal::Admin) {
        return Err(ApiError::Forbidden("仅限管理员".into()));
    }
    let mut cfg = engram_mcp::load_config(&state.pool).await;
    if let Some(enabled) = req.enabled {
        cfg.enabled = enabled;
    }
    if let Some(disabled) = req.disabled_tools {
        // 工具名校验：停用一个不存在的名字多半是调用方笔误，宁可 400
        let known: Vec<String> = engram_mcp::tool_catalog();
        for name in &disabled {
            if !known.contains(name) {
                return Err(ApiError::BadRequest(format!(
                    "未知工具 {name:?}——可用：{}",
                    known.join(", ")
                )));
            }
        }
        cfg.disabled_tools = disabled;
    }
    engram_mcp::save_config(&state.pool, &cfg).await?;
    Ok(Json(engram_mcp::build_info(&state.pool).await))
}

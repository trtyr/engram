//! guard 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

/// 错误桥：把本模块的错误统一成 MCP ErrorData。
pub(crate) fn mcp_err(code: ErrorCode, msg: impl Into<String>) -> rmcp::ErrorData {
    rmcp::ErrorData::new(code, msg.into(), None)
}

/// MemoryError → MCP 错误码（与 HTTP API 的 me() 同语义）。
pub(crate) fn from_memory(e: engram_core::memory::MemoryError) -> rmcp::ErrorData {
    use engram_core::memory::MemoryError;
    match e {
        MemoryError::NotFound(m) => rmcp::ErrorData::resource_not_found(m, None),
        MemoryError::BadRequest(m) => rmcp::ErrorData::invalid_params(m, None),
        MemoryError::Storage(m) => rmcp::ErrorData::internal_error(m, None),
        MemoryError::LlmNotConfigured(m) => rmcp::ErrorData::internal_error(m, None),
    }
}

/// ProjectError → MCP 错误码（与 HTTP API 的 pe() 同语义：NotFound/Conflict/BadRequest/Storage）。
pub(crate) fn from_project(e: engram_core::project::ProjectError) -> rmcp::ErrorData {
    use engram_core::project::ProjectError;
    match e {
        ProjectError::NotFound(m) => rmcp::ErrorData::resource_not_found(m, None),
        ProjectError::Conflict(m) => rmcp::ErrorData::new(ErrorCode::INVALID_REQUEST, m, None),
        ProjectError::BadRequest(m) => rmcp::ErrorData::invalid_params(m, None),
        ProjectError::Storage(m) => rmcp::ErrorData::internal_error(m, None),
    }
}

pub(crate) fn from_asset(e: engram_core::assets::AssetError) -> rmcp::ErrorData {
    use engram_core::assets::AssetError;
    match e {
        AssetError::NotFound(m) => rmcp::ErrorData::resource_not_found(m, None),
        AssetError::Conflict(m) => rmcp::ErrorData::new(ErrorCode::INVALID_REQUEST, m, None),
        AssetError::BadRequest(m) => rmcp::ErrorData::invalid_params(m, None),
        AssetError::Storage(m) => rmcp::ErrorData::internal_error(m, None),
    }
}

pub(crate) fn require_memory(principal: &Principal) -> Result<(), rmcp::ErrorData> {
    match principal.domain_access("memory") {
        DomainAccess::None => Err(mcp_err(
            ErrorCode::INVALID_REQUEST,
            "缺少 memory scope——请用带 memory scope 的 amk_ key 连接 MCP",
        )),
        // Full 直过；ReadOnly 的动作级判定在 call_tool 入口（check_action_access）已做
        _ => Ok(()),
    }
}

/// 原件（精确值：凭据/序列号/IP:端口/账号 ID）读写权——从 memory 独立的 scope。
/// 签 key 时「给不给原件」是显式决策：memory scope 不再顺手可读凭据（收录哲学线）。
pub(crate) fn require_original(principal: &Principal) -> Result<(), rmcp::ErrorData> {
    match principal.domain_access("original") {
        DomainAccess::None => Err(mcp_err(
            ErrorCode::INVALID_REQUEST,
            "缺少 original scope（原件：凭据/序列号等精确值读写）——请用带 original scope 的 amk_ key 连接 MCP",
        )),
        _ => Ok(()),
    }
}

pub(crate) fn require_project(principal: &Principal) -> Result<(), rmcp::ErrorData> {
    match principal.domain_access("project") {
        DomainAccess::None => Err(mcp_err(
            ErrorCode::INVALID_REQUEST,
            "缺少 project scope——请用带 project scope 的 amk_ key 连接 MCP",
        )),
        _ => Ok(()),
    }
}

pub(crate) fn require_erase(principal: &Principal) -> Result<(), rmcp::ErrorData> {
    if principal.has_scope("erase") {
        Ok(())
    } else {
        Err(mcp_err(
            ErrorCode::INVALID_REQUEST,
            "擦除需要 erase scope（不可逆操作，与读写分权）——void 模式无需 erase",
        ))
    }
}

/// 资产台账域（2026-09-21 新增）：scope 名同域名，叫 assets。
pub(crate) fn require_credentials(principal: &Principal) -> Result<(), rmcp::ErrorData> {
    match principal.domain_access("credentials") {
        DomainAccess::None => Err(mcp_err(
            ErrorCode::INVALID_REQUEST,
            "缺少 credentials scope——请用带 credentials scope 的 amk_ key 连接 MCP",
        )),
        _ => Ok(()),
    }
}

pub(crate) fn require_assets(principal: &Principal) -> Result<(), rmcp::ErrorData> {
    match principal.domain_access("assets") {
        DomainAccess::None => Err(mcp_err(
            ErrorCode::INVALID_REQUEST,
            "缺少 assets scope——请用带 assets scope 的 amk_ key 连接 MCP",
        )),
        _ => Ok(()),
    }
}

pub(crate) fn require_todos(principal: &Principal) -> Result<(), rmcp::ErrorData> {
    match principal.domain_access("todos") {
        DomainAccess::None => Err(mcp_err(
            ErrorCode::INVALID_REQUEST,
            "缺少 todos scope——请用带 todos scope 的 amk_ key 连接 MCP",
        )),
        _ => Ok(()),
    }
}

/// TodoError → MCP 错误码。
pub(crate) fn from_todo(e: engram_core::todos::TodoError) -> rmcp::ErrorData {
    use engram_core::todos::TodoError;
    match e {
        TodoError::NotFound(m) => rmcp::ErrorData::resource_not_found(m, None),
        TodoError::BadRequest(m) => rmcp::ErrorData::invalid_params(m, None),
        TodoError::Storage(m) => rmcp::ErrorData::internal_error(m, None),
    }
}

/// CgError → MCP 错误码。
pub(crate) fn from_cg(e: engram_cg_bridge::CgError) -> rmcp::ErrorData {
    use engram_cg_bridge::CgError;
    match e {
        CgError::NotFound(m) => rmcp::ErrorData::resource_not_found(m, None),
        CgError::BadRequest(m) => rmcp::ErrorData::invalid_params(m, None),
        // R5 提示可见（2026-09-21 task-5）：版本不符 / CLI 缺失都补可照抄的「装 + 锁版」指引，
        // 否则云端看不到 CLI 的部署者只能看到一句「版本不匹配」而无从下手
        CgError::VersionMismatch { need, got } => rmcp::ErrorData::invalid_params(
            format!(
                "版本不匹配：需要 {need}，实际 {got}——{}",
                engram_cg_bridge::cli_fix_hint(&need)
            ),
            None,
        ),
        CgError::CliUnavailable(m) => rmcp::ErrorData::internal_error(
            format!(
                "{m}——{}",
                engram_cg_bridge::cli_fix_hint(engram_cg_bridge::CG_VERSION_PIN)
            ),
            None,
        ),
        other => rmcp::ErrorData::internal_error(other.to_string(), None),
    }
}

pub(crate) fn require_codegraph(principal: &Principal) -> Result<(), rmcp::ErrorData> {
    match principal.domain_access("codegraph") {
        DomainAccess::None => Err(mcp_err(
            ErrorCode::INVALID_REQUEST,
            "缺少 codegraph scope——请用带 codegraph scope 的 amk_ key 连接 MCP",
        )),
        _ => Ok(()),
    }
}

/// 从工具调用上下文取 HTTP 请求里的 Principal（bearer_auth 已认证并注入）。
pub(crate) fn principal_of(ctx: &RequestContext<RoleServer>) -> Result<Principal, rmcp::ErrorData> {
    let parts = ctx
        .extensions
        .get::<axum::http::request::Parts>()
        .ok_or_else(|| mcp_err(ErrorCode::INTERNAL_ERROR, "内部错误：缺少 HTTP 请求上下文"))?;
    parts
        .extensions
        .get::<Principal>()
        .cloned()
        .ok_or_else(|| mcp_err(ErrorCode::INTERNAL_ERROR, "内部错误：缺少认证主体"))
}

/// 宽容时间解析：RFC3339 全形态或 date-only（与 HTTP API opt_flex_dt 同一套语义）。
pub(crate) fn parse_flex_datetime(
    s: &str,
) -> Result<chrono::DateTime<chrono::Utc>, rmcp::ErrorData> {
    let t = s.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t) {
        return Ok(dt.into());
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d")
        && let Some(ndt) = d.and_hms_opt(0, 0, 0)
    {
        return Ok(chrono::DateTime::from_naive_utc_and_offset(
            ndt,
            chrono::Utc,
        ));
    }
    Err(mcp_err(
        ErrorCode::INVALID_PARAMS,
        format!("无法解析时间 {s:?}：期望 ISO8601（2026-09-02 或 2026-09-02T00:00:00Z）"),
    ))
}

pub(crate) fn ok_json(v: serde_json::Value) -> Result<CallToolResult, rmcp::ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(
        serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string()),
    )]))
}

// ---------- MCP 面返回瘦身（R 报告 P0-1：写操作不回显正文） ----------
//
// 调用方刚发送的正文原样返回是纯浪费（正文已在调用方上下文里）。
// HTTP API 保持全量（Web UI 依赖），MCP 面统一只回元数据——渐进式「列表层」字段集。

/// 通用瘦身：删 body/content 等正文键，补 content_chars。
pub(crate) fn slim_content(v: serde_json::Value, content_keys: &[&str]) -> serde_json::Value {
    let mut v = v;
    let mut chars = 0i64;
    if let Some(obj) = v.as_object_mut() {
        for key in content_keys {
            if let Some(s) = obj.remove(*key).and_then(|x| x.as_str().map(String::from)) {
                chars = s.chars().count() as i64;
            }
        }
        obj.insert("content_chars".into(), json!(chars));
        obj.insert("content_omitted".into(), json!(true));
    }
    v
}

/// 会话写入瘦身：turns 数组换轮次数（全文走 get_session）。
pub(crate) fn slim_session(s: serde_json::Value) -> serde_json::Value {
    let mut v = s;
    if let Some(obj) = v.as_object_mut() {
        let turns = obj.remove("content");
        let n = turns.as_ref().and_then(|t| t.as_array()).map(|a| a.len());
        obj.insert("turns".into(), json!(n.unwrap_or(0)));
        obj.insert(
            "hint".into(),
            json!("已入库（轮次数见 turns）——原文用 get_session 回读；蒸馏产物几分钟后可 search/list_atoms 看到。写入验收：用本会话 id 调 action=\"distill_result\" 查蒸馏出了什么。注意：distill_status 变 done 后本会话不可再 append——续接请用 write_session 开新会话"),
        );
    }
    v
}

/// 项目文档瘦身：正文换 content_chars。
pub(crate) fn slim_doc(d: serde_json::Value) -> serde_json::Value {
    slim_content(d, &["content"])
}

/// 待办瘦身：正文换 content_chars（title/状态/时间全保留）。
pub(crate) fn slim_todo(t: serde_json::Value) -> serde_json::Value {
    slim_content(t, &["body"])
}

/// 递归删除对象键（context include_evidence=false 时剥溯源字段）。
pub(crate) fn strip_keys(v: &mut serde_json::Value, keys: &[&str]) {
    match v {
        serde_json::Value::Object(m) => {
            for k in keys {
                m.remove(*k);
            }
            for (_, child) in m.iter_mut() {
                strip_keys(child, keys);
            }
        }
        serde_json::Value::Array(a) => {
            for child in a.iter_mut() {
                strip_keys(child, keys);
            }
        }
        _ => {}
    }
}

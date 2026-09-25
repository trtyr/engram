//! memory_kv 域 MCP 工具面（架构治理 2026-09-20：自 lib.rs 纯搬移，零行为变化）。

use super::*;

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct MemoryKvPutParams {
    /// 键（唯一；同 key 再写 = 就地覆盖更新，不产生历史链）
    #[schemars(description = "键（唯一，≤200 字符）。同 key 再次写入 = 覆盖更新（就地改值）。")]
    pub key: String,
    /// 值（逐字保存——蒸馏永不加工；序列号/UUID/IP/端口等精确值原样存）
    #[schemars(
        description = "值（逐字保存，零加工）。精确值如序列号/UUID/IP:PORT 原样存，禁概括。"
    )]
    pub value: String,
    /// 可选：说明（这是哪台机器的什么值、怎么探到的）
    #[schemars(description = "可选：上下文说明。")]
    pub context: Option<String>,
    /// 可选：标签
    #[schemars(description = "可选：标签数组。")]
    pub tags: Option<Vec<String>>,
    /// 可选：来源 user_stated/verified_probe/agent_inferred/doc（缺省 user_stated）
    #[schemars(
        description = "可选：值来源。user_stated=用户明示, verified_probe=实测探得, agent_inferred=推断, doc=文档。"
    )]
    pub source: Option<String>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct MemoryKvGetParams {
    /// 键
    #[schemars(description = "要读的键。")]
    pub key: String,
}

/// EN-241：KV 删除（写坏/过期/测试残留的物理清理通道）。
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct MemoryKvDeleteParams {
    /// 要删除的键
    #[schemars(description = "要删除的键（kv_list/kv_search 拿到的 key）。")]
    pub key: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct MemoryKvListParams {
    /// 返回上限（默认 50，≤500）
    #[schemars(description = "可选：返回上限（按 updated_at 倒序）。默认 50。")]
    pub limit: Option<i64>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct MemoryKvSearchParams {
    /// 字面量（≥3 字符；key/value/context ILIKE 直查——精确值不依赖分词）
    #[schemars(
        description = "字面量检索词（≥3 字符）。对 key/value/context 做 ILIKE 直查——序列号等精确值用这个，不依赖分词。"
    )]
    pub query: String,
    /// 返回上限（默认 20，≤100）
    #[schemars(description = "可选：返回上限。默认 20。")]
    pub limit: Option<i64>,
}

#[tool_router(router = memory_kv_router)]
impl EngramMcpServer {
    // ---------- KV 值保值通道（蒸馏零介入——精确值原样透传） ----------

    /// 写入/更新结构化值（kv_put）：同 key 就地覆盖——序列号/UUID/IP:PORT 等精确值的正道。
    pub(crate) async fn memory_kv_put(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<MemoryKvPutParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_original(&p)?;
        // :ro 变体只读——kv_put 是写动作，original 域的写拒绝在 handler 内做
        //（check_action_access 对 memory 域查的是 memory 的域状态，看不到 original:ro）
        if p.domain_access("original") == DomainAccess::ReadOnly {
            return Err(mcp_err(
                ErrorCode::INVALID_REQUEST,
                "权限不足：key 的 original scope 是只读变体（:ro）——原件写操作（kv_put）需要完整 original scope",
            ));
        }
        let kp = params.0;
        let row = self
            .svc()
            .kv_put(
                &kp.key,
                &kp.value,
                kp.context.as_deref(),
                kp.tags,
                kp.source.as_deref(),
            )
            .await
            .map_err(from_memory)?;
        ok_json(serde_json::to_value(&row).unwrap_or(serde_json::json!({})))
    }

    /// 读取结构化值（kv_get）。
    pub(crate) async fn memory_kv_get(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<MemoryKvGetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_original(&p)?;
        let row = self
            .svc()
            .kv_get(&params.0.key)
            .await
            .map_err(from_memory)?;
        match row {
            Some(r) => ok_json(serde_json::to_value(&r).unwrap_or(serde_json::json!({}))),
            None => Err(rmcp::ErrorData::invalid_params(
                format!("kv 键不存在: {}", params.0.key),
                None,
            )),
        }
    }

    /// 删除结构化值（kv_delete，EN-241）：物理删除指定 key——治理清理通道。
    pub(crate) async fn memory_kv_delete(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<MemoryKvDeleteParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_original(&p)?;
        if p.domain_access("original") == DomainAccess::ReadOnly {
            return Err(mcp_err(
                ErrorCode::INVALID_REQUEST,
                "权限不足：key 的 original scope 是只读变体（:ro）——删除（kv_delete）需要完整 original scope",
            ));
        }
        let deleted = self
            .svc()
            .kv_delete(&params.0.key)
            .await
            .map_err(from_memory)?;
        ok_json(serde_json::json!({
            "deleted": deleted,
            "key": params.0.key,
            "hint": if deleted { "已物理删除" } else { "key 不存在——kv_list/kv_search 确认现有键名" },
        }))
    }

    /// 列出全部 KV（kv_list，按 updated_at 倒序）。
    pub(crate) async fn memory_kv_list(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<MemoryKvListParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_original(&p)?;
        let rows = self
            .svc()
            .kv_list(params.0.limit.unwrap_or(50))
            .await
            .map_err(from_memory)?;
        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
    }

    /// 字面量直查 KV（kv_search）——精确值不依赖分词，ILIKE 全字段。
    pub(crate) async fn memory_kv_search(
        &self,
        ctx: RequestContext<RoleServer>,
        params: Parameters<MemoryKvSearchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_original(&p)?;
        let rows = self
            .svc()
            .kv_search(&params.0.query, params.0.limit.unwrap_or(20))
            .await
            .map_err(from_memory)?;
        ok_json(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
    }
}

/// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_memory_kv() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::memory_kv_router()
}

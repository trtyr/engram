//! circles 域 MCP 工具面（EN-229：实体坐标系「圈子」视图的 agent 读写入口）。
//!
//! 前端「圈子」页（web Circle.tsx → Galaxy）浏览的是 memory 实体坐标系——
//! 实体（人物/项目/主题/群组/地点）与类型化关系（member_of/located_in/works_on/part_of/related_to）
//! 加共现边。此前这套模型只有 HTTP 入口，MCP 面对 agent 不可见——本域补齐读写。
//! 底座全复用 MemoryService 实体服务（同表同底座），scope 沿用 memory（todos/tickets 先例）。

use super::*;

// ---------- 参数 ----------

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CirclesGraphParams {}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CirclesEntityParams {
    /// 实体 id
    #[schemars(description = "实体 id（UUID，来自 graph 的节点或 entities 检索）。")]
    pub entity_id: String,
    /// 可选：true = 含 superseded 历史原子（T021② 全量考古；默认仅活跃）
    #[schemars(description = "可选：true = 含 superseded 历史原子（全量考古）；默认仅活跃。")]
    #[serde(default)]
    pub include_superseded: bool,
}

/// T021：forget/delete/merge/attach/detach 共用的实体 id 参数。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CirclesTargetParams {
    /// 实体 id（操作目标）
    #[schemars(description = "实体 id（UUID）。forget=归档目标；delete=物理删目标；merge=被并方（from）；attach/detach=挂摘目标实体。")]
    pub entity_id: String,
}

/// T021：merge 的并入方。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CirclesMergeParams {
    /// 被并方实体 id（from——墓碑化，原子改挂）
    #[schemars(description = "被并方实体 id（from）：墓碑化后原子改挂到 into。")]
    pub from_id: String,
    /// 并入方实体 id（into——保留方，档案延续）
    #[schemars(description = "并入方实体 id（into）：保留方，档案延续。")]
    pub into_id: String,
}

/// T021：attach/detach 的原子挂摘参数。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CirclesAtomLinkParams {
    /// 实体 id
    #[schemars(description = "实体 id（UUID）。")]
    pub entity_id: String,
    /// 原子 id
    #[schemars(description = "原子 id（UUID）。attach=挂上；detach=摘下。")]
    pub atom_id: String,
}

/// T021：实体详情全量开关。
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CirclesEntityFullParams {
    /// 实体 id
    #[schemars(description = "实体 id（UUID）。")]
    pub entity_id: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CirclesCreateParams {
    /// 实体名（唯一，同名同类型幂等返回已有实体）
    #[schemars(description = "实体名（唯一）。同名同类型重复创建 = 幂等返回已有实体。")]
    pub name: String,
    /// 实体类型
    #[schemars(
        description = "实体类型：person=人物 / project=项目 / topic=主题 / group=群组 / place=地点。"
    )]
    pub kind: String,
    /// 一句话摘要
    #[schemars(description = "可选：一句话摘要（谁/是什么）。")]
    pub summary: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CirclesUpdateParams {
    /// 实体 id
    #[schemars(description = "实体 id（UUID）。")]
    pub entity_id: String,
    /// 新名（可选）
    #[schemars(description = "可选：新名。")]
    pub name: Option<String>,
    /// 新摘要（可选；留修订史）
    #[schemars(description = "可选：新摘要（手编留修订版本链）。")]
    pub summary: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CirclesRelateParams {
    /// 关系起点实体 id
    #[schemars(description = "关系起点实体 id（from）。")]
    pub from_id: String,
    /// 关系终点实体 id
    #[schemars(description = "关系终点实体 id（to）。")]
    pub to_id: String,
    /// 关系类型
    #[schemars(
        description = "关系类型：member_of=隶属于 / located_in=位于 / works_on=参与 / part_of=组成部分 / related_to=相关。同向同类型重复 = upsert（权重+1）。"
    )]
    pub rel_type: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CirclesUnrelateParams {
    /// 关系 id
    #[schemars(description = "关系 id（UUID，来自 entity 详情或 graph 的 relations）。")]
    pub relation_id: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CirclesRelationsParams {
    /// 实体 id
    #[schemars(description = "实体 id（UUID）。返回以该实体为起点或终点的全部类型化关系。")]
    pub entity_id: String,
}

// ---------- 处理器 ----------

#[tool_router(router = circles_router)]
impl EngramMcpServer {
    /// 圈子域（EN-229）：实体坐标系「圈子」视图的读写入口。
    ///
    /// 何时用：agent 要看「记忆里都有谁/什么事、谁和谁什么关系」（graph）、
    /// 建实体（create）、连类型化关系（relate）、改实体档案（update）时。
    /// 数据与前端「圈子」页同源（memory 实体坐标系），scope 沿用 memory。
    #[tool(
        name = "circles",
        description = "圈子域（实体坐标系读写，EN-229）：实体图全景 graph、实体详情 entity、建实体 create、类型化关系 relate/unrelate、改档案 update、关系清单 relations。操作全景：action=\"help\"。"
    )]
    pub(crate) async fn circles(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(call): Parameters<dispatch::DomainCall>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let p = principal_of(&ctx)?;
        require_memory(&p)?;
        let action = call.action.clone();
        let svc = self.svc();
        match action.as_str() {
            "graph" => {
                let _: CirclesGraphParams = dispatch::from_args("circles", "graph", call.args)?;
                let g = svc.entity_graph().await.map_err(memory_err)?;
                ok_json(json!({
                    "nodes": g.nodes,
                    "edges": g.edges,
                    "relations": g.relations,
                    "hint": "实体坐标系全景：nodes=实体（含密度）、edges=共现边（同原子双挂）、relations=类型化关系。实体详情用 action=\"entity\"。"
                }))
            }
            "entity" => {
                let p: CirclesEntityParams = dispatch::from_args("circles", "entity", call.args)?;
                let id = uuid::Uuid::parse_str(&p.entity_id)
                    .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "entity_id 不是合法 UUID"))?;
                let d = if p.include_superseded {
                    svc.get_entity_full(id).await.map_err(memory_err)?
                } else {
                    svc.get_entity(id).await.map_err(memory_err)?
                };
                ok_json(json!({
                    "entity": d.entity,
                    "atoms": d.atoms,
                    "scenarios": d.scenarios,
                    "relations": d.relations,
                }))
            }
            "create" => {
                let p: CirclesCreateParams = dispatch::from_args("circles", "create", call.args)?;
                let e = svc
                    .create_entity(&p.name, &p.kind, p.summary.as_deref().unwrap_or(""))
                    .await
                    .map_err(memory_err)?;
                ok_json(json!({
                    "id": e.id,
                    "name": e.name,
                    "kind": e.kind,
                    "summary": e.summary,
                    "hint": "同名同类型幂等：重复 create 返回已有实体。连关系用 action=\"relate\"。"
                }))
            }
            "update" => {
                let p: CirclesUpdateParams = dispatch::from_args("circles", "update", call.args)?;
                let id = uuid::Uuid::parse_str(&p.entity_id)
                    .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "entity_id 不是合法 UUID"))?;
                let e = svc
                    .update_entity(id, p.name.as_deref(), p.summary.as_deref(), "mcp:circles")
                    .await
                    .map_err(memory_err)?;
                ok_json(json!({ "entity": e, "hint": "摘要手编已留修订史（entity revisions）。" }))
            }
            "forget" => {
                // T021/Q004：归档式遗忘——实体归档 + 挂链 active 原子级联归档（可恢复）
                let p: CirclesTargetParams = dispatch::from_args("circles", "forget", call.args)?;
                let id = uuid::Uuid::parse_str(&p.entity_id)
                    .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "entity_id 不是合法 UUID"))?;
                let archived = svc.forget_entity(id).await.map_err(memory_err)?;
                ok_json(json!({
                    "entity_id": id,
                    "archived_atoms": archived,
                    "hint": "归档式遗忘：实体与挂链活跃原子级联归档（可恢复）。显式清理走 action=\"delete\"。"
                }))
            }
            "delete" => {
                // T021/Q004：物理删——档案蒸发（显式清理意图，不可恢复）
                let p: CirclesTargetParams = dispatch::from_args("circles", "delete", call.args)?;
                let id = uuid::Uuid::parse_str(&p.entity_id)
                    .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "entity_id 不是合法 UUID"))?;
                svc.delete_entity(id).await.map_err(memory_err)?;
                ok_json(json!({
                    "entity_id": id,
                    "deleted": true,
                    "hint": "物理删除完成（不可恢复）。默认安全路径用 action=\"forget\"（归档可恢复）。"
                }))
            }
            "merge" => {
                // T021：实体合并——from 墓碑化，原子改挂到 into（档案延续）
                let p: CirclesMergeParams = dispatch::from_args("circles", "merge", call.args)?;
                let from = uuid::Uuid::parse_str(&p.from_id)
                    .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "from_id 不是合法 UUID"))?;
                let into = uuid::Uuid::parse_str(&p.into_id)
                    .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "into_id 不是合法 UUID"))?;
                let moved = svc.merge_entities(from, into).await.map_err(memory_err)?;
                ok_json(json!({
                    "from_id": from, "into_id": into, "moved_atoms": moved,
                    "hint": "合并完成：from 墓碑化，原子改挂到 into。"
                }))
            }
            "attach" => {
                // T021：挂原子到实体（跨域操作按发起视角归 circles——动的是 atom_entities 边）
                let p: CirclesAtomLinkParams =
                    dispatch::from_args("circles", "attach", call.args)?;
                let eid = uuid::Uuid::parse_str(&p.entity_id)
                    .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "entity_id 不是合法 UUID"))?;
                let aid = uuid::Uuid::parse_str(&p.atom_id)
                    .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "atom_id 不是合法 UUID"))?;
                svc.attach_atom(eid, aid).await.map_err(memory_err)?;
                ok_json(json!({ "entity_id": eid, "atom_id": aid, "attached": true }))
            }
            "detach" => {
                let p: CirclesAtomLinkParams =
                    dispatch::from_args("circles", "detach", call.args)?;
                let eid = uuid::Uuid::parse_str(&p.entity_id)
                    .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "entity_id 不是合法 UUID"))?;
                let aid = uuid::Uuid::parse_str(&p.atom_id)
                    .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "atom_id 不是合法 UUID"))?;
                svc.detach_atom(eid, aid).await.map_err(memory_err)?;
                ok_json(json!({ "entity_id": eid, "atom_id": aid, "detached": true }))
            }
            "duplicates" => {
                // T021：实体重复检测（从 memory 域按域界挪入）
                let _: CirclesGraphParams = dispatch::from_args("circles", "duplicates", call.args)?;
                let dups = svc.entity_duplicates().await.map_err(memory_err)?;
                ok_json(json!({
                    "duplicates": dups,
                    "count": dups.len(),
                    "hint": "同名实体分组（归一化名/数量/id 列表，EN-242）。合并用 action=\"merge\"。"
                }))
            }
            "relate" => {
                let p: CirclesRelateParams = dispatch::from_args("circles", "relate", call.args)?;
                let from = uuid::Uuid::parse_str(&p.from_id)
                    .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "from_id 不是合法 UUID"))?;
                let to = uuid::Uuid::parse_str(&p.to_id)
                    .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "to_id 不是合法 UUID"))?;
                let r = svc
                    .create_relation(from, to, &p.rel_type, "manual")
                    .await
                    .map_err(memory_err)?;
                ok_json(json!({
                    "relation": r,
                    "hint": "类型化关系已建（同向同类型 upsert，权重累加）。删用 action=\"unrelate\"。"
                }))
            }
            "unrelate" => {
                let p: CirclesUnrelateParams =
                    dispatch::from_args("circles", "unrelate", call.args)?;
                let rid = uuid::Uuid::parse_str(&p.relation_id)
                    .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "relation_id 不是合法 UUID"))?;
                svc.delete_relation(rid).await.map_err(memory_err)?;
                ok_json(json!({ "deleted": rid, "hint": "关系已删除。" }))
            }
            "relations" => {
                let p: CirclesRelationsParams =
                    dispatch::from_args("circles", "relations", call.args)?;
                let id = uuid::Uuid::parse_str(&p.entity_id)
                    .map_err(|_| mcp_err(ErrorCode::INVALID_PARAMS, "entity_id 不是合法 UUID"))?;
                let rels = svc.list_relations(Some(id)).await.map_err(memory_err)?;
                ok_json(json!({
                    "entity_id": id,
                    "count": rels.len(),
                    "relations": rels,
                }))
            }
            "help" => {
                let manual = dispatch::render_manual("circles", &[]);
                ok_json(manual)
            }
            other => Err(dispatch::unknown_action("circles", other)),
        }
    }
}

fn memory_err(e: engram_core::memory::MemoryError) -> rmcp::ErrorData {
    use engram_core::memory::MemoryError as E;
    let code = match e {
        E::NotFound(_) => ErrorCode::INVALID_REQUEST,
        E::BadRequest(_) => ErrorCode::INVALID_PARAMS,
        _ => ErrorCode::INTERNAL_ERROR,
    };
    mcp_err(code, e.to_string())
}

// 供装配层合并（宏生成的 router 方法私有，本模块内包一层）。
pub(crate) fn routes_circles() -> ToolRouter<EngramMcpServer> {
    EngramMcpServer::circles_router()
}

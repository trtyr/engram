//! `memory_api` 的实现切片（架构治理 2026-09-21：自 memory_api.rs 纯搬移，零行为变化）。

use super::*;

/// 实体摘要版本链（手编档案历史，最近在前）。
#[utoipa::path(get, path = "/memory/entities/{id}/revisions",
    responses((status = 200, body = [engram_core::EntityRevision])))]
pub async fn entity_revisions(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<engram_core::EntityRevision>>, ApiError> {
    require_memory_read(&principal)?;
    Ok(Json(svc(&state).entity_revisions(id).await.map_err(me)?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CreateRelationRequest {
    pub to_id: Uuid,
    /// member_of / located_in / works_on / part_of / related_to
    pub rel_type: String,
}

/// 实体关系列表（有向：本实体作为 from 或 to）。
#[utoipa::path(get, path = "/memory/entities/{id}/relations",
    responses((status = 200, body = [engram_core::EntityRelationDto])))]
pub async fn list_entity_relations(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<engram_core::EntityRelationDto>>, ApiError> {
    require_memory_read(&principal)?;
    Ok(Json(
        svc(&state).list_relations(Some(id)).await.map_err(me)?,
    ))
}

/// 建关系（有向：本实体 --rel_type--> to；同向同类型 upsert）。
#[utoipa::path(post, path = "/memory/entities/{id}/relations", request_body = CreateRelationRequest,
    responses((status = 201, body = engram_core::EntityRelationDto)))]
pub async fn create_entity_relation(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateRelationRequest>,
) -> Result<(StatusCode, Json<engram_core::EntityRelationDto>), ApiError> {
    require_memory(&principal)?;
    // 权限收窄：关系由蒸馏抽取（source=distill），AI 写会话即可
    if !matches!(&*principal, Principal::Admin) {
        return Err(ApiError::Forbidden(
            "关系由蒸馏抽取（source=distill）——AI 写会话即可，蒸馏自动抽取实体间关系".into(),
        ));
    }
    let r = svc(&state)
        .create_relation(id, req.to_id, &req.rel_type, "manual")
        .await
        .map_err(me)?;
    Ok((StatusCode::CREATED, Json(r)))
}

/// 删关系。
#[utoipa::path(delete, path = "/memory/entities/{id}/relations/{rid}",
    responses((status = 204, description = "已删除")))]
pub async fn delete_entity_relation(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path((_id, rid)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    require_memory(&principal)?;
    require_erase(&principal)?;
    svc(&state).delete_relation(rid).await.map_err(me)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct BatchEntitiesRequest {
    pub ids: Vec<Uuid>,
    /// true = 级联归档原子后再删实体（forget 语义）
    #[serde(default)]
    pub forget: bool,
    /// 破坏性批量操作确认短语："批量删除"
    pub confirm: String,
}

/// 批量删除实体（破坏性：erase scope + 确认短语）。
#[utoipa::path(post, path = "/memory/entities/batch", request_body = BatchEntitiesRequest,
    responses((status = 200, body = serde_json::Value)))]
pub async fn batch_entities(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<BatchEntitiesRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_memory(&principal)?;
    // 破坏性批量：与 erase 同级（admin 全权 / erase scope）
    match &*principal {
        Principal::Admin => {}
        Principal::ApiKey { scopes, .. } if scopes.iter().any(|s| s == "erase") => {}
        _ => {
            return Err(ApiError::Forbidden(
                "批量删除需要 erase scope（不可逆操作，与读写分权）".into(),
            ));
        }
    }
    if req.confirm != "批量删除" {
        return Err(ApiError::BadRequest(
            "批量删除需要确认短语（confirm=批量删除）——破坏半径大，AI 应先复述破坏半径，用户确认后再执行".into(),
        ));
    }
    let mut deleted = 0i64;
    let mut archived = 0i64;
    for id in &req.ids {
        if req.forget {
            archived += svc(&state).forget_entity(*id).await.map_err(me)? as i64;
        } else {
            svc(&state).delete_entity(*id).await.map_err(me)?;
        }
        deleted += 1;
    }
    Ok(Json(
        serde_json::json!({"deleted": deleted, "archived": archived}),
    ))
}

/// 圈子独立实体导出（数据主权，memory scope，无破坏性）。
#[utoipa::path(get, path = "/memory/entities/export",
    responses((status = 200, body = serde_json::Value)))]
pub async fn export_entities(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_memory_read(&principal)?;
    let entities = svc(&state).list_entities(None).await.map_err(me)?;
    let relations = svc(&state).list_relations(None).await.map_err(me)?;
    Ok(Json(serde_json::json!({
        "format": "engram-entities-export",
        "version": 1,
        "exported_at": chrono::Utc::now(),
        "entities": entities,
        "relations": relations,
    })))
}

#[derive(Deserialize, IntoParams)]
pub struct ListEntitiesParams {
    /// person / project / topic / group
    pub kind: Option<String>,
}

#[derive(Deserialize, utoipa::IntoParams)]
pub struct SearchEntitiesParams {
    /// 检索词（jieba 分词；name 命中权重 1.0，summary 0.3）
    pub q: String,
    /// 返回条数（默认 20）
    pub limit: Option<i64>,
}

/// 圈子语义检索：按 token 命中打分（实体量小，无向量/FTS，名字命中优先）。
#[utoipa::path(get, path = "/memory/entities/search", params(SearchEntitiesParams),
    responses((status = 200, body = [SearchHit])))]
pub async fn search_entities_handler(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<SearchEntitiesParams>,
) -> Result<Json<Vec<SearchHit>>, ApiError> {
    require_memory_read(&principal)?;
    Ok(Json(
        search_entities(&state.pool, &p.q, p.limit.unwrap_or(20))
            .await
            .map_err(|e| ApiError::Database(engram_storage::StoreError::Sql(e)))?,
    ))
}

/// 实体列表（按记忆密度降序）。
#[utoipa::path(get, path = "/memory/entities", params(ListEntitiesParams),
    responses((status = 200, body = [EntityDto])))]
pub async fn list_entities(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ListEntitiesParams>,
) -> Result<Json<Vec<EntityDto>>, ApiError> {
    require_memory_read(&principal)?;
    Ok(Json(
        svc(&state)
            .list_entities(p.kind.as_deref())
            .await
            .map_err(me)?,
    ))
}

/// 星系图：节点 + 共现边。
#[utoipa::path(get, path = "/memory/entities/graph",
    responses((status = 200, body = EntityGraph)))]
pub async fn entity_graph(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<EntityGraph>, ApiError> {
    require_memory_read(&principal)?;
    Ok(Json(svc(&state).entity_graph().await.map_err(me)?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CreateEntityRequest {
    pub name: String,
    /// person / project / topic / group
    pub kind: String,
    /// 画像摘要（关系行文，可后补）
    #[serde(default)]
    pub summary: String,
}

/// 手动建实体。
#[utoipa::path(post, path = "/memory/entities", request_body = CreateEntityRequest,
    responses((status = 201, body = EntityDto)))]
pub async fn create_entity(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<CreateEntityRequest>,
) -> Result<(StatusCode, Json<EntityDto>), ApiError> {
    require_memory(&principal)?;
    // 权限收窄：实体由蒸馏从会话抽取，AI 写会话即可
    if !matches!(&*principal, Principal::Admin) {
        return Err(ApiError::Forbidden(
            "实体由蒸馏从会话中抽取——AI 写会话即可，蒸馏自动抽取人物/项目/主题/群组".into(),
        ));
    }
    let e = svc(&state)
        .create_entity(&req.name, &req.kind, &req.summary)
        .await
        .map_err(me)?;
    Ok((StatusCode::CREATED, Json(e)))
}

/// 实体详情：画像摘要 + 相关原子时间线 + 相关场景。
#[utoipa::path(get, path = "/memory/entities/{id}",
    responses((status = 200, body = EntityDetail)))]
pub async fn get_entity(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<EntityDetail>, ApiError> {
    require_memory_read(&principal)?;
    Ok(Json(svc(&state).get_entity(id).await.map_err(me)?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateEntityRequest {
    pub name: Option<String>,
    pub summary: Option<String>,
}

/// 改名/改画像摘要。
#[utoipa::path(patch, path = "/memory/entities/{id}", request_body = UpdateEntityRequest,
    responses((status = 200, body = EntityDto)))]
pub async fn update_entity(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateEntityRequest>,
) -> Result<Json<EntityDto>, ApiError> {
    require_memory(&principal)?;
    // 实体名/摘要是改写语义（档案手编 = 钉住，consolidate 绕开）——仅限用户
    if !matches!(&*principal, Principal::Admin) {
        return Err(ApiError::Forbidden(
            "实体名/摘要编辑仅限用户（Web 登录态）；AI 的实体档案由蒸馏维护，关联走 attach/detach"
                .into(),
        ));
    }
    let actor = actor_of(&principal);
    Ok(Json(
        svc(&state)
            .update_entity(id, req.name.as_deref(), req.summary.as_deref(), &actor)
            .await
            .map_err(me)?,
    ))
}

/// 删实体（关联原子保留，仅解除关联）；?forget=true 级联归档挂链原子——「把 XX 忘了」。
#[derive(Deserialize, utoipa::IntoParams)]
pub struct ForgetParams {
    /// true = 实体级遗忘：挂链 active 原子全部归档，再删实体
    pub forget: Option<bool>,
}

#[utoipa::path(delete, path = "/memory/entities/{id}",
    params(ForgetParams),
    responses((status = 204), (status = 200, description = "forget=true 时返回 {archived: N}")))]
pub async fn delete_entity(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::extract::Query(fp): axum::extract::Query<ForgetParams>,
) -> Result<axum::response::Response, ApiError> {
    require_memory(&principal)?;
    require_erase(&principal)?;
    if fp.forget.unwrap_or(false) {
        let n = svc(&state).forget_entity(id).await.map_err(me)?;
        return Ok((
            StatusCode::OK,
            axum::Json(serde_json::json!({ "archived": n })),
        )
            .into_response());
    }
    svc(&state).delete_entity(id).await.map_err(me)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// 挂原子到实体（幂等）。
#[utoipa::path(post, path = "/memory/entities/{id}/atoms/{atom_id}",
    responses((status = 204)))]
pub async fn attach_atom(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path((id, atom_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    require_memory(&principal)?;
    // 权限收窄：原子-实体挂接由蒸馏自动完成
    if !matches!(&*principal, Principal::Admin) {
        return Err(ApiError::Forbidden(
            "原子-实体挂接由蒸馏自动完成——AI 写会话即可".into(),
        ));
    }
    svc(&state).attach_atom(id, atom_id).await.map_err(me)?;
    Ok(StatusCode::NO_CONTENT)
}

/// 摘除原子关联。
#[utoipa::path(delete, path = "/memory/entities/{id}/atoms/{atom_id}",
    responses((status = 204)))]
pub async fn detach_atom(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path((id, atom_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    require_memory(&principal)?;
    require_erase(&principal)?;
    svc(&state).detach_atom(id, atom_id).await.map_err(me)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct MergeEntityRequest {
    /// 合并目标（幸存实体）
    pub into: Uuid,
}

/// 合并实体：原子关联全部改挂目标，from 置 merged_into 让出唯一名。
#[utoipa::path(post, path = "/memory/entities/{id}/merge", request_body = MergeEntityRequest,
    responses((status = 200, body = Object)))]
pub async fn merge_entity(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<MergeEntityRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_memory(&principal)?;
    let moved = svc(&state).merge_entities(id, req.into).await.map_err(me)?;
    Ok(Json(serde_json::json!({ "moved": moved })))
}

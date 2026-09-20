//! `memory_api` 的实现切片（架构治理 2026-09-21：自 memory_api.rs 纯搬移，零行为变化）。

use super::*;

/// KV 列表查询参数。
#[derive(Deserialize, IntoParams)]
pub struct ListKvParams {
    /// 可选：key/value/context 字面量子串过滤（ILIKE）
    pub q: Option<String>,
    /// 条数上限（缺省 100，上限 500）
    pub limit: Option<i64>,
}

/// KV 列表（只读治理面）：按 updated_at 倒序，含 context/tags/source/updated_at。
#[utoipa::path(get, path = "/memory/kv", params(ListKvParams),
    responses((status = 200, body = [KvEntryDto])))]
pub async fn list_kv(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ListKvParams>,
) -> Result<Json<Vec<KvEntryDto>>, ApiError> {
    require_memory(&principal)?;
    let limit = p.limit.unwrap_or(100).clamp(1, 500);
    let rows = match p.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        Some(q) => svc(&state).kv_search(q, limit).await,
        None => svc(&state).kv_list(limit).await,
    }
    .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(rows))
}

/// KV 单条（只读治理面）：按 key 精确取。
#[utoipa::path(get, path = "/memory/kv/{key}", params(("key" = String, Path, description = "KV key")),
    responses((status = 200, body = KvEntryDto), (status = 404)))]
pub async fn get_kv(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<KvEntryDto>, ApiError> {
    require_memory(&principal)?;
    let row = svc(&state)
        .kv_get(&key)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "KV {key:?} 不存在——用 GET /memory/kv?q= 子串检索确认 key"
            ))
        })?;
    Ok(Json(row))
}

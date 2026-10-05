//! 画像活文档（persona_doc）——离线整理 Agent 维护，用户可手动编辑（P015 场景层退役：
//! 旧分面画像（persona_aspects）连同 scenarios 一起退役，画像唯一形态 = 单份 Markdown 活文档）。

use super::*;

#[derive(Deserialize, utoipa::ToSchema)]
pub struct PersonaDocEditRequest {
    /// 全量新内容（Markdown 活文档，整文替换——历史自动留版本链）
    pub content: String,
    /// 摘要；缺省沿用旧 summary
    pub summary: Option<String>,
}

/// 用户手动编辑画像活文档（Admin-only；与离线整理 Agent 同一条 save_doc 通道，版本链留痕）。
#[utoipa::path(post, path = "/memory/persona-doc",
    request_body = PersonaDocEditRequest,
    responses((status = 200, body = Object)))]
pub async fn persona_doc_edit(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<PersonaDocEditRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_memory(&principal)?;
    // 编辑画像是"用户直改"语义——AI 禁入（它有离线整理 Agent 通道）
    if !matches!(&*principal, Principal::Admin) {
        return Err(ApiError::Forbidden(
            "画像编辑仅限用户（Web 登录态）——AI 的画像认知由离线整理 Agent 维护".into(),
        ));
    }
    let content = req.content.trim();
    if content.is_empty() {
        return Err(ApiError::BadRequest("画像内容不能为空".into()));
    }
    Ok(Json(
        svc(&state)
            .persona_doc_edit(content, req.summary.as_deref())
            .await
            .map_err(me)?,
    ))
}

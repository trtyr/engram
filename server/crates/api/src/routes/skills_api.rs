//! 技能域端点（skills scope，第六域）。
//!
//! 技能 = SKILL.md 形态的 AI 指令资产：slug 唯一、版本快照、批量导入、全量导出。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use engram_core::skills::{
    SkillDto, SkillImportReport, SkillPatch, SkillRevisionDto, SkillSummaryDto, SkillsError,
    SkillsService,
};
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::auth::{Principal, require_scope};
use crate::error::ApiError;
use crate::state::AppState;

fn require_skills(p: &Principal) -> Result<(), ApiError> {
    require_scope(p, "skills")
}

fn se(e: SkillsError) -> ApiError {
    match e {
        SkillsError::NotFound(m) => ApiError::NotFound(m),
        SkillsError::Conflict(m) => ApiError::Conflict(m),
        SkillsError::BadRequest(m) => ApiError::BadRequest(m),
        SkillsError::Storage(m) => ApiError::Unavailable(m),
    }
}

fn svc(state: &AppState) -> SkillsService {
    SkillsService::new(state.pool.clone())
}

// ---------- 请求体 ----------

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CreateSkillRequest {
    /// kebab-case 标识（缺省从 name 推导；中文名必须显式给）
    pub slug: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}
fn default_enabled() -> bool {
    true
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateSkillRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub enabled: Option<bool>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ImportSkillDoc {
    /// 可选文件名（无 frontmatter name 时兜底命名）
    pub filename: Option<String>,
    /// SKILL.md 全文（frontmatter 容错解析：name/description/slug/tags）
    pub content: String,
    /// 可选附带标签（如来源子目录名），与 frontmatter tags 合并
    pub tags: Option<Vec<String>>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ImportSkillsRequest {
    pub documents: Vec<ImportSkillDoc>,
    /// 命中已有 slug 时覆盖更新（默认 false——冲突按条目失败，不阻断其余）
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Deserialize, IntoParams)]
pub struct ListSkillsParams {
    /// 关键词（搜 name/description）
    pub q: Option<String>,
    /// 标签过滤（含即命中）
    pub tag: Option<String>,
    /// true=只看启用 / false=只看停用 / 缺省=全部
    pub enabled: Option<bool>,
}

// ---------- 技能 CRUD ----------

/// 技能列表（摘要，不含正文；q/tag/enabled 过滤）。
#[utoipa::path(get, path = "/skills", params(ListSkillsParams),
    operation_id = "skills_list",
    responses((status = 200, body = [SkillSummaryDto])))]
pub async fn list_skills(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ListSkillsParams>,
) -> Result<Json<Vec<SkillSummaryDto>>, ApiError> {
    require_skills(&principal)?;
    Ok(Json(
        svc(&state)
            .list_skills(p.q.as_deref(), p.tag.as_deref(), p.enabled)
            .await
            .map_err(se)?,
    ))
}

/// 新建技能（slug 唯一，冲突 409；初始状态留 rev1 快照）。
#[utoipa::path(post, path = "/skills",
    request_body = CreateSkillRequest,
    responses((status = 201, body = SkillDto)))]
pub async fn create_skill(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<CreateSkillRequest>,
) -> Result<(StatusCode, Json<SkillDto>), ApiError> {
    require_skills(&principal)?;
    let s = svc(&state)
        .create_skill(engram_core::skills::NewSkill {
            slug: req.slug.as_deref(),
            name: &req.name,
            description: &req.description,
            content: &req.content,
            tags: &req.tags,
            enabled: req.enabled,
            source: "manual",
        })
        .await
        .map_err(se)?;
    Ok((StatusCode::CREATED, Json(s)))
}

/// 批量导入 SKILL.md 全文（逐条成败互不阻断，返回逐条报告）。
#[utoipa::path(post, path = "/skills/import",
    request_body = ImportSkillsRequest,
    responses((status = 200, body = SkillImportReport)))]
pub async fn import_skills(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<ImportSkillsRequest>,
) -> Result<Json<SkillImportReport>, ApiError> {
    require_skills(&principal)?;
    let docs: Vec<(Option<String>, String, Vec<String>)> = req
        .documents
        .into_iter()
        .map(|d| (d.filename, d.content, d.tags.unwrap_or_default()))
        .collect();
    Ok(Json(
        svc(&state)
            .import_skills(&docs, req.overwrite, "import")
            .await
            .map_err(se)?,
    ))
}

/// 全量导出（含正文，数据主权：技能库随时整体带走）。
#[utoipa::path(get, path = "/skills/export",
    responses((status = 200, body = [SkillDto])))]
pub async fn export_skills(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<SkillDto>>, ApiError> {
    require_skills(&principal)?;
    Ok(Json(svc(&state).export_skills().await.map_err(se)?))
}

/// 技能详情（含 markdown 正文）。
#[utoipa::path(get, path = "/skills/{slug}",
    operation_id = "skills_get",
    responses((status = 200, body = SkillDto)))]
pub async fn get_skill(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<SkillDto>, ApiError> {
    require_skills(&principal)?;
    Ok(Json(svc(&state).get_skill(&slug).await.map_err(se)?))
}

/// 编辑技能（语义字段变更留版本快照；enabled-only 不留）。
#[utoipa::path(put, path = "/skills/{slug}",
    request_body = UpdateSkillRequest,
    responses((status = 200, body = SkillDto)))]
pub async fn update_skill(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<UpdateSkillRequest>,
) -> Result<Json<SkillDto>, ApiError> {
    require_skills(&principal)?;
    Ok(Json(
        svc(&state)
            .update_skill(
                &slug,
                SkillPatch {
                    name: req.name,
                    description: req.description,
                    content: req.content,
                    tags: req.tags,
                    enabled: req.enabled,
                },
            )
            .await
            .map_err(se)?,
    ))
}

/// 删除技能（级联删版本快照，不可逆）。
#[utoipa::path(delete, path = "/skills/{slug}",
    responses((status = 204, description = "已删除")))]
pub async fn delete_skill(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_skills(&principal)?;
    svc(&state).delete_skill(&slug).await.map_err(se)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- 版本 ----------

/// 版本快照列表（新→旧）。
#[utoipa::path(get, path = "/skills/{slug}/revisions",
    responses((status = 200, body = [SkillRevisionDto])))]
pub async fn list_revisions(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Vec<SkillRevisionDto>>, ApiError> {
    require_skills(&principal)?;
    Ok(Json(svc(&state).list_revisions(&slug).await.map_err(se)?))
}

/// 回滚到某版本（回滚前先快照现状，回滚本身可再撤销）。
#[utoipa::path(post, path = "/skills/{slug}/revisions/{rev_id}/restore",
    responses((status = 200, body = SkillDto)))]
pub async fn restore_revision(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path((slug, rev_id)): Path<(String, Uuid)>,
) -> Result<Json<SkillDto>, ApiError> {
    require_skills(&principal)?;
    Ok(Json(
        svc(&state)
            .restore_revision(&slug, rev_id)
            .await
            .map_err(se)?,
    ))
}

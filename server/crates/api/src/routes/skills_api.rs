//! 技能域端点（skills scope）。
//!
//! 技能 = SKILL.md 形态的 AI 指令资产：slug 唯一、版本快照、批量导入、全量导出。

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use engram_core::skills::{
    SkillDto, SkillExportDto, SkillFileInfoDto, SkillImportReport, SkillPatch, SkillRevisionDto,
    SkillSummaryDto, SkillsError, SkillsService,
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
    /// 存储形态：text=整体入库（缺省）/ script=脚本存本地、库中只存指针
    #[serde(default = "default_kind")]
    pub kind: String,
    /// 来源：self=自建（缺省）/ github=源自 GitHub / both=自建且已发布
    #[serde(default = "default_origin")]
    pub origin: String,
    /// script 型必填：本地技能文件夹路径（含 SKILL.md）；text 型不接受
    #[serde(default)]
    pub local_path: Option<String>,
    /// origin 含 github 时可填：仓库地址（纯元数据，不做远端拉取）
    #[serde(default)]
    pub repo_url: Option<String>,
}
fn default_enabled() -> bool {
    true
}
fn default_kind() -> String {
    "text".into()
}
fn default_origin() -> String {
    "self".into()
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateSkillRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub enabled: Option<bool>,
    /// 来源（终值语义；origin=self 时 repo_url 自动清空）
    #[serde(default)]
    pub origin: Option<String>,
    /// 仓库地址（origin 含 github 时有意义）
    #[serde(default)]
    pub repo_url: Option<String>,
    /// script 型指针改址（script 型专用）
    #[serde(default)]
    pub local_path: Option<String>,
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
            kind: &req.kind,
            origin: &req.origin,
            local_path: req.local_path.as_deref(),
            repo_url: req.repo_url.as_deref(),
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
    responses((status = 200, body = [SkillExportDto])))]
pub async fn export_skills(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<SkillExportDto>>, ApiError> {
    require_skills(&principal)?;
    Ok(Json(svc(&state).export_skills().await.map_err(se)?))
}

/// 技能详情（含 markdown 正文；script 型从 local_path 现读，指针失效报 404）。
#[utoipa::path(get, path = "/skills/{slug}",
    operation_id = "skills_get",
    responses((status = 200, body = SkillDto)))]
pub async fn get_skill(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<SkillDto>, ApiError> {
    require_skills(&principal)?;
    Ok(Json(
        svc(&state)
            .get_skill_with_content(&slug)
            .await
            .map_err(se)?,
    ))
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
                    origin: req.origin,
                    repo_url: req.repo_url,
                    local_path: req.local_path,
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

// ---------- 附属文件（folder 形态：scripts/ / references/…） ----------

#[derive(Deserialize, IntoParams)]
pub struct SkillFileParams {
    /// 相对路径（/ 分隔，如 scripts/run.py）
    pub path: String,
    /// raw=1 → 直接回文件本体（text/plain），curl -o 一条命令落盘（消费形态②：只要一个文件）
    pub raw: Option<String>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct SkillFilePutRequest {
    /// 相对路径（/ 分隔；禁止 .. 与绝对路径；SKILL.md 本体走技能更新）
    pub path: String,
    /// 文本内容
    pub content: String,
}

/// 附属文件索引（path + 字节大小；SKILL.md 本体不在其中）。
#[utoipa::path(get, path = "/skills/{slug}/files", params(("slug" = String, Path, description = "技能 slug")),
    operation_id = "skills_files_list",
    responses((status = 200, body = [SkillFileInfoDto])))]
pub async fn list_files(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Vec<SkillFileInfoDto>>, ApiError> {
    require_skills(&principal)?;
    Ok(Json(svc(&state).list_files(&slug).await.map_err(se)?))
}

/// 读一个附属文件全文。
#[utoipa::path(get, path = "/skills/{slug}/file", params(("slug" = String, Path), SkillFileParams),
    operation_id = "skills_file_get",
    responses((status = 200, body = SkillExportDto)))]
pub async fn get_file(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(p): Query<SkillFileParams>,
) -> Result<Response, ApiError> {
    require_skills(&principal)?;
    let content = svc(&state).get_file(&slug, &p.path).await.map_err(se)?;
    if p.raw.is_some() {
        // 消费形态②：单文件直下——curl -s ".../file?path=…&raw=1" -o 文件名
        let name = p.path.rsplit('/').next().unwrap_or("file");
        return Response::builder()
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            )
            .body(Body::from(content))
            .map_err(|e| ApiError::Internal(e.into()));
    }
    Ok(Json(engram_core::skills::SkillFileEntryDto {
        path: p.path,
        content,
    })
    .into_response())
}

/// 写（upsert）一个附属文件；返回 path 与字节数。
#[utoipa::path(put, path = "/skills/{slug}/file", params(("slug" = String, Path)),
    request_body = SkillFilePutRequest,
    operation_id = "skills_file_put",
    responses((status = 200, body = SkillFileInfoDto)))]
pub async fn put_file(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<SkillFilePutRequest>,
) -> Result<Json<SkillFileInfoDto>, ApiError> {
    require_skills(&principal)?;
    let (path, size) = svc(&state)
        .put_file(&slug, &req.path, &req.content)
        .await
        .map_err(se)?;
    Ok(Json(SkillFileInfoDto { path, size }))
}

/// 删除一个附属文件。
#[utoipa::path(delete, path = "/skills/{slug}/file", params(("slug" = String, Path), SkillFileParams),
    operation_id = "skills_file_delete",
    responses((status = 204)))]
pub async fn delete_file(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(p): Query<SkillFileParams>,
) -> Result<StatusCode, ApiError> {
    require_skills(&principal)?;
    svc(&state).delete_file(&slug, &p.path).await.map_err(se)?;
    Ok(StatusCode::NO_CONTENT)
}

/// 技能导入闭环：吃 /skills/export 同构数据（含附属文件），slug 冲突跳过。
/// 与 POST /skills/import（SKILL.md 文本粘贴）互补——本端点做迁移/合并。
#[utoipa::path(post, path = "/skills/import-transfer", request_body = Object,
    operation_id = "skills_import_transfer",
    responses((status = 200, body = Object)))]
pub async fn import_transfer(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(data): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_skills(&principal)?;
    Ok(Json(
        engram_core::transfer::import_skills_bundle(&state.pool, &data)
            .await
            .map_err(|e| match e {
                engram_core::transfer::TransferError::BadRequest(m) => ApiError::BadRequest(m),
                other => ApiError::Unavailable(other.to_string()),
            })?,
    ))
}

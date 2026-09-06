//! 技能域端点（skills scope，第六域）。
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
    responses((status = 200, body = [SkillExportDto])))]
pub async fn export_skills(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<SkillExportDto>>, ApiError> {
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
        return Ok(Response::builder()
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            )
            .body(Body::from(content))
            .unwrap());
    }
    Ok(Json(engram_core::skills::SkillFileEntryDto {
        path: p.path,
        content,
    })
    .into_response())
}

/// 整包拉取（消费形态③）：skill = 文件夹 → 一个 zip（SKILL.md + scripts/ + references/…）。
/// 客户端两条命令落盘即可执行：
///   curl -s -H "Authorization: Bearer $KEY" {base}/skills/{slug}/bundle -o s.zip
///   tar -xf s.zip（bsdtar 直接解 zip；或 unzip -o s.zip）
/// 路径在写入侧已校验（禁 .. / 绝对路径 / 反斜杠），zip-slip 不可能。
#[utoipa::path(get, path = "/skills/{slug}/bundle", params(("slug" = String, Path, description = "技能 slug")),
    operation_id = "skills_bundle",
    responses((status = 200, content_type = "application/zip")))]
pub async fn bundle(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Response, ApiError> {
    require_skills(&principal)?;
    let e = svc(&state).export_one(&slug).await.map_err(se)?;
    let io_err = |e: std::io::Error| ApiError::Internal(anyhow::anyhow!(e.to_string()));
    let zip_err = |e: zip::result::ZipError| ApiError::Internal(anyhow::anyhow!(e.to_string()));
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        zw.start_file("SKILL.md", opts).map_err(zip_err)?;
        std::io::Write::write_all(
            &mut zw,
            engram_core::skills::render_skill_md(&e.skill).as_bytes(),
        )
        .map_err(io_err)?;
        for f in &e.files {
            zw.start_file(&f.path, opts).map_err(zip_err)?;
            std::io::Write::write_all(&mut zw, f.content.as_bytes()).map_err(io_err)?;
        }
        zw.finish().map_err(zip_err)?;
    }
    Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}.zip\"", e.skill.slug),
        )
        .body(Body::from(buf.into_inner()))
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))
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

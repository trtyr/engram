//! CodeGraph 域端点（codegraph scope）。
//!
//! 索引/同步走平台 job 队列（返回 202 + job_id，异步执行）——不再阻塞 HTTP 10 分钟。

use axum::Json;
use axum::extract::multipart::Multipart;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use engram_core::codegraph::{CgBridge, CgError, CgProjectDto, CliStatus, QueryKind};
use engram_jobs::{JobQueue, JobTemplate};
use serde::Deserialize;
use serde_json::json;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::auth::{Principal, require_scope, require_scope_read};
use crate::error::ApiError;
use crate::state::AppState;

fn require_cg(p: &Principal) -> Result<(), ApiError> {
    require_scope(p, "codegraph")
}
/// 读语义变体：:ro 只读 key 放行（RJ-20，对齐 MCP 读动作口径）。
fn require_cg_read(p: &Principal) -> Result<(), ApiError> {
    require_scope_read(p, "codegraph")
}

fn ce(e: CgError) -> ApiError {
    match e {
        CgError::NotFound(m) => ApiError::NotFound(m),
        CgError::BadRequest(m) => ApiError::BadRequest(m),
        CgError::VersionMismatch { need, got } => ApiError::BadRequest(format!(
            "CodeGraph 版本不匹配：需要 {need}，实际 {got}——{}",
            engram_core::codegraph::cli_fix_hint(&need)
        )),
        CgError::Timeout(s, cmd) => ApiError::Unavailable(format!("CodeGraph {cmd} 超时（{s}s）")),
        CgError::Failed(code, msg) => {
            ApiError::Unavailable(format!("CodeGraph 失败（exit {code}）: {msg}"))
        }
        CgError::Parse(m) => ApiError::Unavailable(m),
        CgError::CliUnavailable(m) => ApiError::Unavailable(format!(
            "{m}——{}",
            engram_core::codegraph::cli_fix_hint(engram_core::codegraph::CG_VERSION_PIN)
        )),
        CgError::Storage(m) => ApiError::Unavailable(m),
    }
}

fn bridge(state: &AppState) -> CgBridge {
    CgBridge::new(state.pool.clone(), state.data_dir.join("codegraph"))
}

/// 入队索引/同步 job，返回 job_id。执行进度看 jobs 事件流与项目状态。
async fn enqueue(
    state: &AppState,
    kind: &str,
    id: Uuid,
) -> Result<engram_jobs::types::Job, ApiError> {
    JobQueue::new(state.pool.clone())
        .enqueue(JobTemplate::new(kind).with_payload(json!({ "project_id": id })))
        .await
        .map_err(|e| ApiError::Unavailable(format!("job 入队失败: {e}")))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct RegisterProjectRequest {
    pub name: String,
    /// git 仓库地址（如 https://github.com/you/repo）——本地路径已退场
    pub source_uri: String,
    /// 可选：自定义落盘父目录（绝对路径，须在白名单根内）；留空 = 默认 `<数据根>/codegraph/<项目名>`
    pub dest_parent: Option<String>,
}

/// 注册回调体（入口收敛 2026-09-21）：注册成功后**自动入队建索引**（一步到位）。
/// 入队失败不算注册失败（clone 已落盘）——`warning` 说明原因，前端可手动重试。
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct RegisterProjectResponse {
    pub project: CgProjectDto,
    pub index_job_id: Option<Uuid>,
    pub warning: Option<String>,
}

/// 注册项目（入口收敛 2026-09-21）：**只接受 git 仓库地址**；注册即 `git clone --depth 1`
/// 到默认/自定义目录，并**自动入队建索引**（一步到 ready）。
#[utoipa::path(post, path = "/codegraph/projects",
    request_body = RegisterProjectRequest,
    responses((status = 201, body = RegisterProjectResponse)))]
pub async fn register_project(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<RegisterProjectRequest>,
) -> Result<(StatusCode, Json<RegisterProjectResponse>), ApiError> {
    require_cg(&principal)?;
    let project = bridge(&state)
        .register(&req.name, &req.source_uri, req.dest_parent.as_deref())
        .await
        .map_err(ce)?;
    // 一步到位：注册成功即入队建索引。入队失败**不算注册失败**（clone 已落盘、条目已建）——
    // 返回 warning 让前端可见并可手动点「建索引」重试，避免「明明 clone 好了却报注册失败」。
    match enqueue(&state, "cg_index", project.id).await {
        Ok(job) => Ok((
            StatusCode::CREATED,
            Json(RegisterProjectResponse {
                project,
                index_job_id: Some(job.id),
                warning: None,
            }),
        )),
        Err(e) => {
            tracing::warn!(error = ?e, "注册成功后自动入队建索引失败——注册本身已生效");
            Ok((
                StatusCode::CREATED,
                Json(RegisterProjectResponse {
                    project,
                    index_job_id: None,
                    warning: Some(
                        "已 clone 落盘并建好条目，但自动建索引入队失败——可手动点「建索引」重试"
                            .to_string(),
                    ),
                }),
            ))
        }
    }
}

/// 上传体上限（MB）：产物本体 256MB（与桥层同一口径）+ 1MB multipart 开销余量。
/// 路由级 `DefaultBodyLimit` 用它——全局那道是 MCP base64 口径（约 349MB），对 multipart 过宽，
/// 会让「超限」在缓冲完整个 body 之后才拒。
pub const MAX_ARTIFACT_BODY_MB: usize = 257;
pub const MAX_ARTIFACT_BODY_BYTES: usize = MAX_ARTIFACT_BODY_MB * 1024 * 1024;

/// multipart 读取错误 → 可行动文案（超限单独分型，其余按解析失败）。
fn multipart_err(e: axum::extract::multipart::MultipartError) -> ApiError {
    let msg = e.to_string();
    if msg.contains("length limit") || msg.contains("too large") || msg.contains("body limit") {
        ApiError::BadRequest(format!(
            "上传体超限（上限 {MAX_ARTIFACT_BODY_MB}MB）——产物 db 本体上限 256MB：更大的仓库请改用\
             「git 仓库地址」入口（服务端自己 clone 后索引，不受此限）"
        ))
    } else {
        ApiError::BadRequest(format!("multipart 解析失败: {msg}"))
    }
}

/// 产物上传入口（2026-09-21 入口收敛）：Web/脚本用 multipart 传本机
/// `.codegraph/codegraph.db` 本体——与 MCP `codegraph_upload` 走**同一道**入库校验
/// （SQLite 魔数 / CLI 版本硬拒 / extraction 版本仅告警 / 256MB 上限 / 同名覆盖留痕）。
///
/// 字段：`name`（项目名，必填）、`file`（db 二进制，必填）、`head`（commit hash，可留空 = 未声明）。
#[utoipa::path(post, path = "/codegraph/artifacts",
    request_body(content = Vec<u8>, content_type = "multipart/form-data"),
    responses((status = 201, body = serde_json::Value)))]
pub async fn upload_artifact(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    require_cg(&principal)?;
    let mut name: Option<String> = None;
    let mut head: Option<String> = None;
    let mut file: Option<axum::body::Bytes> = None;
    while let Some(field) = multipart.next_field().await.map_err(multipart_err)? {
        match field.name() {
            Some("name") => name = Some(field.text().await.map_err(multipart_err)?),
            Some("head") => head = Some(field.text().await.map_err(multipart_err)?),
            Some("file") => file = Some(field.bytes().await.map_err(multipart_err)?),
            // 未知字段必须消费掉（否则迭代卡住），内容本身不使用
            _ => {
                let _ = field.bytes().await;
            }
        }
    }
    let name = name.unwrap_or_default().trim().to_string();
    if name.is_empty() {
        return Err(ApiError::BadRequest(
            "multipart 缺字段 name——上传产物要指定项目名（同名即覆盖产物并留痕 stats.previous）"
                .into(),
        ));
    }
    let bytes = file.ok_or_else(|| {
        ApiError::BadRequest(
            "multipart 缺字段 file——请上传本机 `codegraph index` 产出的 `.codegraph/codegraph.db` \
             本体（原始二进制，不要压缩/文本化）"
                .into(),
        )
    })?;
    // 投递者留痕（与 MCP 同口径）：API key 记 `client:<key 名>`，管理员会话记 `client:admin`
    let producer = match &principal.0 {
        Principal::ApiKey { name, .. } => format!("client:{name}"),
        Principal::Admin => "client:admin".to_string(),
    };
    let proj = bridge(&state)
        .upload_artifact(&name, head.as_deref().unwrap_or(""), &producer, &bytes)
        .await
        .map_err(ce)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "project": proj,
            "db_bytes": bytes.len(),
            "hint": "产物已就位，codegraph 查询即查即用；同名再传即覆盖产物并留痕（stats.previous）",
        })),
    ))
}

#[utoipa::path(get, path = "/codegraph/projects", responses((status = 200, body = [CgProjectDto])))]
pub async fn list_projects(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_cg_read(&principal)?;
    // freshness 与 MCP 对齐（EN-26）：每项带 {head, snapshot_head, stale, hint}——
    // 版本诚实不区分接入面。返回数组包一层 {"items": [...]}？否——保持数组形态，
    // freshness 直接注入每项（serde_json Value 数组逐项改写）。
    let rows = bridge(&state).list().await.map_err(ce)?;
    let mut items = Vec::with_capacity(rows.len());
    for r in rows {
        let mut item = serde_json::to_value(&r).map_err(|e| ApiError::Internal(e.into()))?;
        item["freshness"] = bridge(&state).freshness_for(&r).await;
        items.push(item);
    }
    Ok(Json(serde_json::Value::Array(items)))
}

#[utoipa::path(get, path = "/codegraph/projects/{id}", responses((status = 200, body = CgProjectDto)))]
pub async fn get_project(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<CgProjectDto>, ApiError> {
    require_cg_read(&principal)?;
    Ok(Json(bridge(&state).get(id).await.map_err(ce)?))
}

/// 删除项目（入口收敛 2026-09-21）：默认落盘（服务端自建目录）连目录清；
/// 自定义落盘只删注册与产物、**目录保留**——响应 `note` 说明。
#[utoipa::path(delete, path = "/codegraph/projects/{id}", operation_id = "delete_codegraph_project", responses((status = 200, body = Object)))]
pub async fn delete_project(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_cg(&principal)?;
    let out = bridge(&state).delete(id).await.map_err(ce)?;
    Ok(Json(json!({
        "deleted": out.deleted,
        "workdir_removed": out.workdir_removed,
        "note": out.note,
    })))
}

/// 建索引/重建索引：入队 cg_index job 异步执行（202 + job_id；进度看 jobs 与项目状态）。
#[utoipa::path(post, path = "/codegraph/projects/{id}/index", responses((status = 202, body = Object)))]
pub async fn index_project(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    require_cg(&principal)?;
    // 拒绝对不存在项目的入队（404 语义）
    bridge(&state).get(id).await.map_err(ce)?;
    let job = enqueue(&state, "cg_index", id).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "job_id": job.id, "status": "queued", "kind": "cg_index" })),
    ))
}

/// 增量同步：入队 cg_sync job 异步执行。
#[utoipa::path(post, path = "/codegraph/projects/{id}/sync", responses((status = 202, body = Object)))]
pub async fn sync_project(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    require_cg(&principal)?;
    bridge(&state).get(id).await.map_err(ce)?;
    let job = enqueue(&state, "cg_sync", id).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "job_id": job.id, "status": "queued", "kind": "cg_sync" })),
    ))
}

/// CLI 可用性（前端状态条：装没装、版本、pin）。
#[utoipa::path(get, path = "/codegraph/status", operation_id = "codegraph_status", responses((status = 200, body = CliStatus)))]
pub async fn status(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<CliStatus>, ApiError> {
    require_cg_read(&principal)?;
    Ok(Json(bridge(&state).cli_status().await))
}

/// 失效条目对账（EN-48）：路径已不存在 / 索引产物已丢失的 ready 条目标为 error，
/// 使列表不再把幽灵条目冒充可用资产。
///
/// 自愈（EN-48 残留）：对「路径仍在、仅产物丢失」的条目自动入队重建 job——
/// 报告 `queued_rebuild` 带各条目的 job_id；入队失败不中断对账（报告里如实标注）。
#[utoipa::path(post, path = "/codegraph/gc", responses((status = 200, body = Object)))]
pub async fn gc(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_cg(&principal)?;
    let mut report = bridge(&state).gc().await.map_err(ce)?;
    // 自愈：产物丢失（路径仍在）的条目自动重建——异步 job，状态走 indexing → ready/error
    let mut queued = Vec::new();
    if let Some(items) = report["needs_rebuild"].as_array() {
        for item in items {
            let Some(id) = item["id"].as_str().and_then(|s| Uuid::parse_str(s).ok()) else {
                continue;
            };
            match enqueue(&state, "cg_index", id).await {
                Ok(job) => queued.push(serde_json::json!({
                    "id": id,
                    "name": item["name"],
                    "job_id": job.id,
                })),
                Err(e) => queued.push(serde_json::json!({
                    "id": id,
                    "name": item["name"],
                    "enqueue_error": e.to_string(),
                })),
            }
        }
    }
    report["queued_rebuild"] = serde_json::json!(queued);
    Ok(Json(report))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CgQueryRequest {
    /// explore | search | node | callers | callees | impact
    pub kind: String,
    /// 查询文本或符号名
    pub target: String,
    /// explore→max-files；impact→depth
    pub depth: Option<u32>,
    /// explore 专用：true = CLI 原生输出（含完整源码）；缺省 true（HTTP 是人与 Web UI 的
    /// 通道，保留源码形态；MCP 侧缺省 false 走符号大纲，R 报告 P0-3 省 AI 上下文）
    pub include_source: Option<bool>,
}

/// 代理查询（explore/node 返回 Markdown 文本，其余归一 JSON）。
#[utoipa::path(post, path = "/codegraph/projects/{id}/query",
    request_body = CgQueryRequest,
    responses((status = 200, body = Object)))]
pub async fn query(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<CgQueryRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_cg(&principal)?;
    let kind = QueryKind::from_str_opt(&req.kind)
        .ok_or_else(|| ApiError::BadRequest(format!("未知查询类型: {}", req.kind)))?;
    if req.target.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "target 不能为空——先用 kind=search 搜符号，再对具体符号做 callers/impact".into(),
        ));
    }
    let v = bridge(&state)
        .query(
            id,
            kind,
            &req.target,
            req.depth,
            req.include_source.unwrap_or(true),
        )
        .await
        .map_err(ce)?;
    Ok(Json(v))
}

#[derive(Deserialize, IntoParams, utoipa::ToSchema)]
pub struct GraphParams {
    /// 中心符号名（省略 = 返回文件级全图——全部跨文件依赖按文件聚合）
    pub symbol: Option<String>,
}

/// 调用图：带 symbol = 以该符号为中心的 callers/callees 子图；
/// 不带 symbol = 文件级全图（全部跨文件依赖按文件聚合，看项目全貌）。
#[utoipa::path(get, path = "/codegraph/projects/{id}/graph", params(("id" = Uuid, Path), GraphParams),
    responses((status = 200, body = Object)))]
pub async fn graph(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(p): Query<GraphParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_cg_read(&principal)?;
    let v = match p.symbol.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(symbol) => bridge(&state).graph(id, symbol).await.map_err(ce)?,
        None => bridge(&state).full_graph(id).await.map_err(ce)?,
    };
    Ok(Json(v))
}

/// 导出当前产物字节（R2 推送/拉取通道的「拉」侧；`engramctl codegraph pull` 消费）。
///
/// 走 HTTP 直接传字节本体（不做 base64——避免 33% 膨胀；MCP 侧才需要 base64）。附带元数据响应头
/// （name / head / built-with-version / source-kind / artifact-path），使拉取端能把同一份「声明」
/// 原样投给本地实例——等价于一次 upload，无需再问远端要状态。
#[utoipa::path(get, path = "/codegraph/projects/{id}/artifact", responses((status = 200, body = Object)))]
pub async fn project_artifact(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<axum::response::Response, ApiError> {
    require_cg_read(&principal)?;
    let b = bridge(&state);
    let proj = b.get(id).await.map_err(ce)?;
    let (bytes, path) = b.artifact_bytes(id).await.map_err(ce)?;
    let mut builder = axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "application/octet-stream")
        .header(axum::http::header::CONTENT_LENGTH, bytes.len().to_string())
        .header("x-codegraph-name", proj.name.clone())
        .header("x-codegraph-artifact-path", path)
        .header("x-codegraph-source-kind", proj.source_kind.clone());
    if let Some(h) = proj.head.clone() {
        builder = builder.header("x-codegraph-head", h);
    }
    if let Some(v) = proj.built_with_version.clone() {
        builder = builder.header("x-codegraph-built-with-version", v);
    }
    builder
        .body(axum::body::Body::from(bytes))
        .map_err(|e| ApiError::Unavailable(format!("构造产物响应失败: {e}")))
}

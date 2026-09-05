//! 记忆域端点（memory scope）。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use engram_core::memory::{
    AtomDto, ContextPack, EmbeddingStatus, EntityDetail, EntityDto, EntityGraph, MemoryError,
    MemoryService, PersonaVersion, ScenarioDto, SearchResponse, SessionDto,
};
use engram_search::{SearchHit, search_entities};
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::auth::{Principal, require_scope};
use crate::error::ApiError;
use crate::state::AppState;

fn require_memory(p: &Principal) -> Result<(), ApiError> {
    require_scope(p, "memory")
}

/// cron 专属 scope（memory-rhythm 分权）：cron 通道与心跳只能由 cron key 走——
/// 否则 AI 可伪造 `via:"cron"` 审计行、可伪造心跳掩盖 cron 失联。AI 别碰这条线。
fn require_cron(p: &Principal) -> Result<(), ApiError> {
    if p.has_scope("cron") {
        return Ok(());
    }
    Err(ApiError::Forbidden(
        "cron 通道需要 cron scope 的独立 key（AI 主动蒸馏走 manual 不带 via；心跳是 cron 专属，AI 别碰——设置页会读 status 判 cron 死活）".into(),
    ))
}

/// erase scope（破坏性分权）：删实体/摘原子/删关系与擦除会话同级——admin 全权 / erase scope。
/// 权限收窄后 AI（memory-only key）能读能建（走会话蒸馏），但不能毁（删除类操作都要 erase）。
fn require_erase(p: &Principal) -> Result<(), ApiError> {
    if p.has_scope("erase") {
        return Ok(());
    }
    Err(ApiError::Forbidden(
        "删除需要 erase scope（不可逆操作，与读写分权）".into(),
    ))
}

fn me(e: MemoryError) -> ApiError {
    match e {
        MemoryError::NotFound(m) => ApiError::NotFound(m),
        MemoryError::BadRequest(m) => ApiError::BadRequest(m),
        MemoryError::Storage(m) => ApiError::Unavailable(m),
        MemoryError::LlmNotConfigured(m) => ApiError::Unavailable(m),
    }
}

fn svc(state: &AppState) -> MemoryService {
    MemoryService::new(state.pool.clone(), state.registry())
}

// ---------- L0 会话 ----------

#[derive(Deserialize, utoipa::ToSchema)]
pub struct WriteSessionRequest {
    pub agent: Option<String>,
    /// 轮次数组：[{speaker, text, ts?}]
    #[schema(value_type = Object)]
    pub turns: serde_json::Value,
    /// auto（默认，防抖触发蒸馏）| manual（立即）| off
    #[serde(default = "default_distill")]
    pub distill: String,
    /// 会话级敏感标记：整段对话含隐私（医疗/感情/财务），蒸馏产物自动继承 sensitive
    #[serde(default)]
    pub sensitive: bool,
}
fn default_distill() -> String {
    "auto".into()
}

/// 写入 L0 会话（AI 客户端的主要写入口）。
#[utoipa::path(post, path = "/memory/sessions",
    request_body = WriteSessionRequest,
    responses((status = 201, body = SessionDto)))]
pub async fn write_session(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<WriteSessionRequest>,
) -> Result<(StatusCode, Json<SessionDto>), ApiError> {
    require_memory(&principal)?;
    let agent = req.agent.unwrap_or_else(|| "default".into());
    let s = svc(&state)
        .write_session(&agent, req.turns, &req.distill, req.sensitive)
        .await
        .map_err(me)?;
    Ok((StatusCode::CREATED, Json(s)))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ImportSessionRequest {
    pub agent: Option<String>,
    /// 导入内容全文：JSONL（每行 {role, content}）或纯文本（空行分段）
    pub content: String,
    /// jsonl | text
    pub format: String,
    /// auto（默认，防抖触发蒸馏）| manual（立即）| off
    #[serde(default = "default_distill")]
    pub distill: String,
}

/// 批量导入历史对话为会话（phase-2）：JSONL/纯文本 → turns → 落 session（source=import）。
#[utoipa::path(post, path = "/memory/sessions/import",
    request_body = ImportSessionRequest,
    responses((status = 201, body = SessionDto)))]
pub async fn import_session(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<ImportSessionRequest>,
) -> Result<(StatusCode, Json<SessionDto>), ApiError> {
    require_memory(&principal)?;
    let agent = req.agent.unwrap_or_else(|| "default".into());
    let s = svc(&state)
        .import_session(&agent, &req.content, &req.format, &req.distill)
        .await
        .map_err(me)?;
    Ok((StatusCode::CREATED, Json(s)))
}

#[derive(Deserialize, IntoParams)]
pub struct ListSessionsParams {
    pub agent: Option<String>,
    pub cursor: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<i64>,
}

#[utoipa::path(get, path = "/memory/sessions", params(ListSessionsParams),
    responses((status = 200, body = [SessionDto])))]
pub async fn list_sessions(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ListSessionsParams>,
) -> Result<Json<Vec<SessionDto>>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(
        svc(&state)
            .list_sessions(p.agent.as_deref(), p.cursor, p.limit.unwrap_or(50))
            .await
            .map_err(me)?,
    ))
}

#[utoipa::path(get, path = "/memory/sessions/{id}",
    responses((status = 200, body = SessionDto), (status = 404, body = crate::error::ErrorEnvelope)))]
pub async fn get_session(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<SessionDto>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(svc(&state).get_session(id).await.map_err(me)?))
}

/// 擦除会话（引用它的原子来源标记失效）。
#[utoipa::path(delete, path = "/memory/sessions/{id}", responses((status = 204)))]
pub async fn erase_session(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    // erase 不可逆且污染溯源——比读写高一级：memory + erase 双 scope（admin 全权）。
    // 2026-08-31 用户批准：多 AI 共享 key 时任何一个不能单独毁库。
    require_memory(&principal)?;
    match &*principal {
        Principal::Admin => {}
        Principal::ApiKey { scopes, .. } if scopes.iter().any(|s| s == "erase") => {}
        _ => {
            return Err(ApiError::Forbidden(
                "擦除需要 erase scope（不可逆操作，与读写分权）".into(),
            ));
        }
    }
    svc(&state).erase_session(id).await.map_err(me)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct DistillRequest {
    /// true 时附带 consolidate
    #[serde(default)]
    pub full: bool,
    /// 触发通道："cron"（外部定时器）或缺省（人工/AI 主动）。
    /// cron 通道的 consolidate 走日桶幂等——同日重复调用只跑一次全量整理。
    #[serde(default)]
    pub via: Option<String>,
}

/// 手动触发蒸馏链。
#[utoipa::path(post, path = "/memory/distill",
    request_body = DistillRequest,
    responses((status = 202, body = [engram_jobs::Job])))]
pub async fn trigger_distill(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<DistillRequest>,
) -> Result<(StatusCode, Json<Vec<engram_jobs::Job>>), ApiError> {
    require_memory(&principal)?;
    let by = actor_of(&principal);
    let via = if req.via.as_deref() == Some("cron") {
        // cron 通道只能由 cron scope 的 key 走——防 AI 伪造 cron 审计行
        require_cron(&principal)?;
        "cron"
    } else {
        "manual"
    };
    let jobs = svc(&state)
        .trigger_distill(req.full, via, by.as_str())
        .await
        .map_err(me)?;
    Ok((StatusCode::ACCEPTED, Json(jobs)))
}

#[derive(Deserialize, utoipa::IntoParams)]
pub struct HeartbeatParams {
    /// 调用方声明：仅 `cron` 值合法——crontab 命令模板自带，防 AI 误报心跳
    pub via: Option<String>,
}

/// 节律心跳（memory-rhythm）：外部 cron 每次运行时报到——设置页据此判定逾期。
/// 落 jobs 审计行（kind=rhythm_heartbeat），不新建表。
#[utoipa::path(post, path = "/memory/rhythm/heartbeat", params(HeartbeatParams),
    responses((status = 200, body = serde_json::Value)))]
pub async fn rhythm_heartbeat(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<HeartbeatParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_cron(&principal)?;
    // 防伪第二道（2026-09-03 测试报告 R-1）：scope 是软挡（取决于签 key 纪律，
    // 管理员可能给 AI 签出含 cron 的 key）；via=cron 是显式声明——普通 AI 误调用
    // （不带 via）直接 403，不再可能无声伪造「cron 在役」状态。
    if p.via.as_deref() != Some("cron") {
        return Err(ApiError::Forbidden(
            "heartbeat 仅限 crontab 报到（需 via=cron）——AI 请勿调用：会伪造「cron 在役」状态，掩盖真实 cron 失联".into(),
        ));
    }
    let by = actor_of(&principal);
    svc(&state)
        .audit("rhythm_heartbeat", serde_json::json!({ "by": by }))
        .await;
    Ok(Json(serde_json::json!({ "ok": true, "by": by })))
}

/// 节律状态：最近心跳 + pending 会话积压年龄（cron 兜底的对象面）。
#[utoipa::path(get, path = "/memory/rhythm/status",
    responses((status = 200, body = serde_json::Value)))]
pub async fn rhythm_status(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_memory(&principal)?;
    let st = svc(&state).rhythm_status().await.map_err(me)?;
    Ok(Json(st))
}

// ---------- L1 原子 ----------

#[derive(Deserialize, IntoParams)]
pub struct ListAtomsParams {
    pub kind: Option<String>,
    pub status: Option<String>,
    pub needs_review: Option<bool>,
    pub cursor: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<i64>,
}

#[utoipa::path(get, path = "/memory/atoms", params(ListAtomsParams),
    responses((status = 200, body = [AtomDto])))]
pub async fn list_atoms(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ListAtomsParams>,
) -> Result<Json<Vec<AtomDto>>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(
        svc(&state)
            .list_atoms(
                p.kind.as_deref(),
                p.status.as_deref(),
                p.needs_review,
                p.cursor,
                p.limit.unwrap_or(100),
            )
            .await
            .map_err(me)?,
    ))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CreateAtomRequest {
    pub kind: String,
    pub content: String,
    #[serde(default = "default_conf")]
    pub confidence: f32,
    /// 事件时间（ISO8601 或 date-only；"下周三"这类相对时间解析后的绝对值）
    #[serde(default, deserialize_with = "opt_flex_dt")]
    pub occurred_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 有效期（ISO8601；到期事件可过滤/降权）
    pub valid_until: Option<chrono::DateTime<chrono::Utc>>,
    /// P3 隐私标记：默认不进检索与 context_pack（reveal 才可见）
    #[serde(default)]
    pub sensitive: bool,
}
fn default_conf() -> f32 {
    0.9
}

/// 宽容时间反序列化：RFC3339 全形态或 date-only（"2026-09-02" → 当日零点 UTC）——
/// 与 distill 层 parse_iso 同一套语义，API/蒸馏两层不再打架。
fn parse_flex_datetime(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let t = s.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t) {
        return Some(dt.into());
    }
    chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|ndt| chrono::DateTime::from_naive_utc_and_offset(ndt, chrono::Utc))
}

fn opt_flex_dt<'de, D>(d: D) -> Result<Option<chrono::DateTime<chrono::Utc>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: Option<String> = Option::deserialize(d)?;
    match raw {
        None => Ok(None),
        Some(s) => match parse_flex_datetime(&s) {
            Some(dt) => Ok(Some(dt)),
            None => Err(serde::de::Error::custom(format!(
                "无法解析时间 {s:?}：期望 ISO8601（2026-09-02 或 2026-09-02T00:00:00Z）"
            ))),
        },
    }
}

/// P5 会话作废：「这段白记了」——蒸馏跳过、记录保留（只对未蒸馏会话）。
#[utoipa::path(post, path = "/memory/sessions/{id}/void",
    responses((status = 200, body = SessionDto), (status = 400, description = "不存在或已蒸馏")))]
pub async fn void_session(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<SessionDto>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(svc(&state).void_session(id).await.map_err(me)?))
}

/// P11 按 agent 清场（测试隔离）：会话置 void + 产出 active 原子归档（可恢复）。
/// 破坏半径大——与 erase 同级，需 erase scope。
#[derive(Deserialize, utoipa::ToSchema)]
pub struct PurgeRequest {
    /// agent 清场（deep=false 时必填）
    pub agent: Option<String>,
    /// F1：全库清空（四层+实体，单事务）——需 confirm 短语双因子
    pub deep: Option<bool>,
    /// 确认短语，deep=true 时必须精确等于「清空记忆库」
    pub confirm: Option<String>,
    /// P-C 两阶段：携带 armed job id → 立即执行（跳过剩余冷却期）
    pub token: Option<Uuid>,
    /// P-C 两阶段：取消已 armed 的 job（后悔药）
    pub cancel: Option<Uuid>,
}

/// deep purge 的确认短语（用户亲口授权的载体——AI 复述破坏半径后由用户给出）
pub use engram_core::memory::PURGE_CONFIRM_PHRASE;

#[utoipa::path(post, path = "/memory/purge",
    request_body = PurgeRequest,
    responses(
        (status = 200, description = "agent 清场 {erased_sessions, archived_atoms}（SEC-E：全部会话物理删除）或 deep 清空五计数"),
        (status = 400, body = crate::error::ErrorEnvelope),
        (status = 403, body = crate::error::ErrorEnvelope),
    ))]
pub async fn purge_agent(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<PurgeRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_memory(&principal)?;
    // F1 双因子：erase scope（持有）× 逐操作二校验
    match &*principal {
        Principal::Admin => {}
        Principal::ApiKey { scopes, .. } if scopes.iter().any(|s| s == "erase") => {}
        _ => {
            return Err(ApiError::Forbidden(
                "清场需要 erase scope（不可逆操作，与读写分权）".into(),
            ));
        }
    }

    if req.deep.unwrap_or(false) {
        // deep 收权（2026-09-03 测试报告 SEC-D/R-1 决策）：全库清空仅限管理员会话
        // （Web 登录态 → 设置 → 危险区）。确认短语是公开常量（防误操作），挡不住蓄意；
        // 真正的防线是把 deep 移出 AI key 能力面——erase scope 保留给 agent 级清场。
        if !matches!(&*principal, Principal::Admin) {
            return Err(ApiError::Forbidden(
                "deep 全库清空仅限管理员（Web 登录态 → 设置 → 危险区）——AI key 即使有 erase scope 也不可。\
                 按 agent 清场请用 {\"agent\":\"…\"}（erase scope 即可）"
                    .into(),
            ));
        }
        // 事故防线（2026-08-31 测试方案）：deep 是全库清空，agent 在此无过滤语义——
        // 组合传入会让人误以为"只清这个 agent"。强制分开调用，语义零歧义。
        if req.agent.is_some() {
            return Err(ApiError::BadRequest(
                "deep=true 是全库清空，不接受 agent 参数（agent 会在 deep 下被忽略，语义误导）。\
                 按 agent 清场请去掉 deep；全库清空请去掉 agent 并带 confirm=\"清空记忆库\""
                    .into(),
            ));
        }
        // 此处 principal 已收权为 Admin（见上），审计 executed_by 恒为 admin
        let source = "admin".to_string();
        // P-C 后悔药先于确认短语：取消是安全方向，不该要危险确认
        if let Some(job_id) = req.cancel {
            // 后悔药：取消 armed job
            let n = sqlx::query(
                "UPDATE jobs SET status = 'cancelled', finished_at = now() \
                 WHERE id = $1 AND kind = 'deep_purge' AND status = 'pending'",
            )
            .bind(job_id)
            .execute(&state.pool.clone())
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))?
            .rows_affected();
            if n == 0 {
                return Err(ApiError::BadRequest(
                    "取消失败：job 不存在或已执行/已取消".into(),
                ));
            }
            return Ok(Json(
                serde_json::json!({"phase": "cancelled", "job_id": job_id}),
            ));
        }
        // F2 一等清空：确认短语（用户亲口授权）+ P-C 两阶段（arm → 5min 冷却 → 到期执行）
        if req.confirm.as_deref() != Some(PURGE_CONFIRM_PHRASE) {
            return Err(ApiError::BadRequest(format!(
                "deep 清空需要确认短语（AI 应先复述破坏半径，用户确认后传 confirm=\"{}\"）",
                PURGE_CONFIRM_PHRASE
            )));
        }

        // P-C 两阶段（两次真数据事故教训）：arm → 5 分钟冷却 → 到期执行。
        // token = 立即执行；cancel = 后悔药。job 本身就是审计链。
        if let Some(token) = req.token {
            // 阶段二：确认执行——校验 armed job 存在且未执行，跳过剩余冷却
            let armed: Option<(Uuid, serde_json::Value)> = sqlx::query_as(
                "SELECT id, payload FROM jobs \
                 WHERE id = $1 AND kind = 'deep_purge' AND status = 'pending' AND payload->>'phase' = 'armed'",
            )
            .bind(token)
            .fetch_optional(&state.pool.clone())
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))?;
            let Some((job_id, mut payload)) = armed else {
                return Err(ApiError::BadRequest(
                    "token 无效或已过期（armed 状态 5 分钟，到期自动执行或已被取消/执行）".into(),
                ));
            };
            let counts = svc(&state).purge_deep().await.map_err(me)?;
            payload["executed_by"] = serde_json::json!(source);
            sqlx::query(
                "UPDATE jobs SET status = 'succeeded', payload = $2, progress = $3, \
                 started_at = now(), finished_at = now() WHERE id = $1",
            )
            .bind(job_id)
            .bind(&payload)
            .bind(&counts)
            .execute(&state.pool.clone())
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))?;
            return Ok(Json(counts));
        }

        // 阶段一：arm——5 分钟冷却窗口（手滑后悔药），到期由 deep_purge handler 执行
        let job = svc(&state).arm_deep_purge(&source).await.map_err(me)?;
        return Ok(Json(serde_json::json!({
            "phase": "armed",
            "job_id": job.id,
            "executes_at": job.due_at,
            "confirm_now": format!("再次调用并带 token=\"{}\" 立即执行", job.id),
            "cancel": format!("再次调用并带 cancel=\"{}\" 取消", job.id),
        })));
    }

    let agent = req.agent.clone().unwrap_or_default();
    if agent.is_empty() {
        return Err(ApiError::BadRequest(
            "agent 清场需要 agent 参数；全库清空用 deep=true + confirm".into(),
        ));
    }
    let (erased, archived) = svc(&state).purge_agent(&agent).await.map_err(me)?;
    Ok(Json(serde_json::json!({
        "erased_sessions": erased, "archived_atoms": archived
    })))
}

/// P4 全量导出（数据主权）：记忆域五表 JSON 快照。sensitive 原子默认排除。
#[derive(Deserialize, utoipa::IntoParams)]
pub struct ExportParams {
    /// true = 包含 sensitive 原子（R4：隐私面不默认随导出扩大）
    pub include_sensitive: Option<bool>,
}

#[utoipa::path(get, path = "/memory/export",
    params(ExportParams),
    responses((status = 200, description = "完整导出 JSON（format=engram-memory-export）")))]
pub async fn export_memory(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    axum::extract::Query(p): axum::extract::Query<ExportParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(
        svc(&state)
            .export(p.include_sensitive.unwrap_or(false))
            .await
            .map_err(me)?,
    ))
}

/// 追加轮次到既有会话（自动节律 b 配套：长对话分片落库，不等收尾）。
#[derive(Deserialize, utoipa::ToSchema)]
pub struct AppendSessionRequest {
    #[schema(value_type = Object)]
    pub turns: serde_json::Value,
    /// 补记会话归属（pi extension 传 session/model 名；不传保持原值）
    pub agent: Option<String>,
    /// auto（默认，防抖）| off
    #[serde(default = "default_distill")]
    pub distill: String,
}

#[utoipa::path(post, path = "/memory/sessions/{id}/append",
    request_body = AppendSessionRequest,
    responses((status = 200, body = SessionDto), (status = 400, description = "已蒸馏会话不可追加")))]
pub async fn append_session(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<AppendSessionRequest>,
) -> Result<Json<SessionDto>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(
        svc(&state)
            .append_session(id, req.turns, req.agent.as_deref(), &req.distill)
            .await
            .map_err(me)?,
    ))
}

/// 手工新增原子（人审补充，直接 active）。
#[utoipa::path(post, path = "/memory/atoms",
    request_body = CreateAtomRequest,
    responses((status = 201, body = AtomDto)))]
pub async fn create_atom(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<CreateAtomRequest>,
) -> Result<(StatusCode, Json<AtomDto>), ApiError> {
    require_memory(&principal)?;
    // 权限收窄：AI 只写会话（原料），直写原子是蒸馏的活——收回直写加工权
    if !matches!(&*principal, Principal::Admin) {
        return Err(ApiError::Forbidden(
            "原子由会话蒸馏产生——AI 记录对话（session-write）即可，蒸馏自动抽取；人工直写断溯源且绕过判重".into(),
        ));
    }
    let a = svc(&state)
        .create_atom(
            &req.kind,
            &req.content,
            req.confidence,
            req.occurred_at,
            req.valid_until,
            req.sensitive,
        )
        .await
        .map_err(me)?;
    Ok((StatusCode::CREATED, Json(a)))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateAtomRequest {
    pub content: Option<String>,
    /// 分面类型修正（仅用户会话；AI 禁改改写语义）
    pub kind: Option<String>,
    pub confidence: Option<f32>,
    /// 只允许 "archived" / "active"
    pub status: Option<String>,
    /// 人审结论：true=转待审，false=通过（清标记）
    pub needs_review: Option<bool>,
    /// correction 取代链：本原子被哪条新原子取代（arbitrate 自动维护，手动 correction 补链）
    pub superseded_by: Option<Uuid>,
    #[serde(default, deserialize_with = "opt_flex_dt")]
    pub occurred_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, deserialize_with = "opt_flex_dt")]
    pub valid_until: Option<chrono::DateTime<chrono::Utc>>,
    /// P3 隐私标记切换
    pub sensitive: Option<bool>,
}

#[utoipa::path(patch, path = "/memory/atoms/{id}",
    request_body = UpdateAtomRequest,
    responses((status = 200, body = AtomDto)))]
pub async fn update_atom(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateAtomRequest>,
) -> Result<Json<AtomDto>, ApiError> {
    require_memory(&principal)?;
    // 编辑分权：改写语义（content/kind/confidence）仅限用户（Web 登录态）；
    // AI 走 correction：atom-add 新原子 + PATCH 旧原子 superseded_by/status。
    // sensitive/needs_review/status/superseded_by/时间字段对 AI 开放（保护与追加语义）。
    let actor = actor_of(&principal);
    let rewriting = req.content.is_some() || req.kind.is_some() || req.confidence.is_some();
    if rewriting && !matches!(&*principal, Principal::Admin) {
        return Err(ApiError::Forbidden(
            "content/kind/confidence 编辑仅限用户（Web 登录态）；AI 纠错走 correction：写纠正对话（session-write），蒸馏自动取代".into(),
        ));
    }
    // 权限收窄：superseded_by 收回——correction 走会话，取代链由蒸馏自动维护
    if req.superseded_by.is_some() && !matches!(&*principal, Principal::Admin) {
        return Err(ApiError::Forbidden(
            "取代链由蒸馏自动维护——AI 写纠正对话（session-write）即可，correction 走会话".into(),
        ));
    }
    Ok(Json(
        svc(&state)
            .update_atom(
                id,
                req.content.as_deref(),
                req.kind.as_deref(),
                req.confidence,
                req.status.as_deref(),
                req.needs_review,
                req.superseded_by,
                req.occurred_at,
                req.valid_until,
                req.sensitive,
                &actor,
            )
            .await
            .map_err(me)?,
    ))
}

/// 原子改写历史（新→旧；编辑留痕）。
#[utoipa::path(get, path = "/memory/atoms/{id}/revisions",
    responses((status = 200, body = [engram_core::AtomRevision])))]
pub async fn atom_revisions(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<engram_core::AtomRevision>>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(svc(&state).atom_revisions(id).await.map_err(me)?))
}

/// 实体摘要版本链（手编档案历史，最近在前）。
#[utoipa::path(get, path = "/memory/entities/{id}/revisions",
    responses((status = 200, body = [engram_core::EntityRevision])))]
pub async fn entity_revisions(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<engram_core::EntityRevision>>, ApiError> {
    require_memory(&principal)?;
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
    require_memory(&principal)?;
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

#[derive(Deserialize, IntoParams)]
pub struct TimelineParams {
    /// 返回条数（默认 100）
    pub limit: Option<i64>,
}

/// 全局记忆时间轴：原子（occurred_at 优先）/场景/实体按时间倒序合并。
#[utoipa::path(get, path = "/memory/timeline", params(TimelineParams),
    responses((status = 200, body = [engram_core::TimelineEvent])))]
pub async fn timeline(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<TimelineParams>,
) -> Result<Json<Vec<engram_core::TimelineEvent>>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(
        svc(&state)
            .timeline(p.limit.unwrap_or(100))
            .await
            .map_err(me)?,
    ))
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
    require_memory(&principal)?;
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

/// 审计/留痕用主体来源："admin" / "key:名"
fn actor_of(principal: &Principal) -> String {
    match principal {
        Principal::Admin => "admin".to_string(),
        Principal::ApiKey { name, .. } => format!("key:{name}"),
    }
}

// ---------- L2 场景 ----------

#[utoipa::path(get, path = "/memory/scenarios", responses((status = 200, body = [ScenarioDto])))]
pub async fn list_scenarios(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ScenarioDto>>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(svc(&state).list_scenarios(100).await.map_err(me)?))
}

#[utoipa::path(get, path = "/memory/scenarios/{id}",
    responses((status = 200, body = ScenarioDto)))]
pub async fn get_scenario(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ScenarioDto>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(svc(&state).get_scenario(id).await.map_err(me)?))
}

// ---------- L3 画像 ----------

#[utoipa::path(get, path = "/memory/persona", responses((status = 200, body = [PersonaVersion])))]
pub async fn get_persona(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<Vec<PersonaVersion>>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(svc(&state).persona().await.map_err(me)?))
}

#[derive(Deserialize, IntoParams)]
pub struct HistoryParams {
    pub aspect: String,
}

#[utoipa::path(get, path = "/memory/persona/history", params(HistoryParams),
    responses((status = 200, body = [PersonaVersion])))]
pub async fn persona_history(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<HistoryParams>,
) -> Result<Json<Vec<PersonaVersion>>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(
        svc(&state).persona_history(&p.aspect).await.map_err(me)?,
    ))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct RollbackRequest {
    pub aspect: String,
    pub to_version: i32,
}

/// 回滚分面到历史版本（以新版本落地，历史不可变）。
#[utoipa::path(post, path = "/memory/persona/rollback",
    request_body = RollbackRequest,
    responses((status = 200, body = PersonaVersion)))]
pub async fn persona_rollback(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<RollbackRequest>,
) -> Result<Json<PersonaVersion>, ApiError> {
    require_memory(&principal)?;
    // 回滚 = 改写语义（重写当前版本 + 钉住），仅限用户
    if !matches!(&*principal, Principal::Admin) {
        return Err(ApiError::Forbidden(
            "画像回滚仅限用户（Web 登录态）；AI 纠错走 correction 流程".into(),
        ));
    }
    let actor = actor_of(&principal);
    Ok(Json(
        svc(&state)
            .persona_rollback(&req.aspect, req.to_version, &actor)
            .await
            .map_err(me)?,
    ))
}

// ---------- 画像编辑（编辑能力：用户直改分面，钉住=蒸馏绕开） ----------

#[derive(Deserialize, utoipa::ToSchema)]
pub struct PersonaEditRequest {
    /// 分面（identity/preferences/skills/constraints/communication_style/goals/routines）
    pub aspect: String,
    /// 新内容；缺省时不改内容
    pub content: Option<String>,
    /// false = 解除钉住，回归蒸馏管辖（手编保护关闭）
    pub pinned: Option<bool>,
}

/// 用户编辑画像分面（留痕：新版本 manually_edited=true + 审计行）。
#[utoipa::path(patch, path = "/memory/persona",
    request_body = PersonaEditRequest,
    responses(
        (status = 200, body = PersonaVersion, description = "编辑/解钉后的分面最新版"),
        (status = 403, body = crate::error::ErrorEnvelope),
    ))]
pub async fn persona_edit(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<PersonaEditRequest>,
) -> Result<Json<PersonaVersion>, ApiError> {
    require_memory(&principal)?;
    // 编辑画像是"用户直改"语义——AI 禁入（它有自己的蒸馏/correction 通道）
    if !matches!(&*principal, Principal::Admin) {
        return Err(ApiError::Forbidden(
            "画像编辑仅限用户（Web 登录态）——AI 的画像认知由蒸馏与 correction 维护".into(),
        ));
    }
    let actor = actor_of(&principal);
    let svc = svc(&state);
    if let Some(content) = req.content.as_deref() {
        let v = svc
            .persona_edit(&req.aspect, content, &actor)
            .await
            .map_err(me)?;
        return Ok(Json(v));
    }
    match req.pinned {
        Some(false) => svc.persona_unpin(&req.aspect, &actor).await.map_err(me)?,
        Some(true) => {
            svc.persona_repin(&req.aspect, &actor).await.map_err(me)?;
        }
        None => {}
    }
    let latest = svc
        .persona_history(&req.aspect)
        .await
        .map_err(me)?
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::NotFound(format!("分面 {} 不存在", req.aspect)))?;
    Ok(Json(latest))
}

// ---------- 检索 ----------

#[derive(Deserialize, utoipa::ToSchema)]
pub struct SearchRequest {
    pub query: String,
    /// 层过滤：["l1","l2","l3","entities"]，空 = 全部
    #[serde(default)]
    pub layers: Vec<String>,
    pub max_items: Option<i64>,
    /// true = 命中不回写热度（harness 注入/测试用，防 B6 污染）
    #[serde(default)]
    pub no_feedback: bool,
    /// true = 结果包含 sensitive 原子（隐私项默认排除；P3）
    #[serde(default)]
    pub reveal: bool,
    /// 时间范围过滤起点（occurred_at 优先，NULL fallback created_at；UTC）
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    /// 时间范围过滤终点
    pub to: Option<chrono::DateTime<chrono::Utc>>,
}

#[utoipa::path(post, path = "/memory/search", operation_id = "memory_search",
    request_body = SearchRequest,
    responses((status = 200, body = SearchResponse)))]
pub async fn search(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, ApiError> {
    require_memory(&principal)?;
    let layers: Vec<&str> = req.layers.iter().map(|s| s.as_str()).collect();
    Ok(Json(
        svc(&state)
            .search(
                &req.query,
                &layers,
                req.max_items.unwrap_or(20),
                req.no_feedback,
                req.reveal,
                req.from,
                req.to,
            )
            .await
            .map_err(me)?,
    ))
}

#[derive(Deserialize, IntoParams)]
pub struct ContextParams {
    /// 可选相关性查询；缺省按最近
    pub query: Option<String>,
    pub budget_items: Option<usize>,
    pub budget_chars: Option<usize>,
    /// true = 命中不回写热度（harness 注入/测试用）
    #[serde(default)]
    pub no_feedback: bool,
}

/// AI 冷启动首选：一站式上下文包（L3+L2+L1 按预算裁剪）。
#[utoipa::path(get, path = "/memory/context", params(ContextParams),
    responses((status = 200, body = ContextPack)))]
pub async fn context(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
    Query(p): Query<ContextParams>,
) -> Result<Json<ContextPack>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(
        svc(&state)
            .context_pack(
                p.query.as_deref(),
                p.budget_items.unwrap_or(20),
                p.budget_chars.unwrap_or(8000),
                p.no_feedback,
            )
            .await
            .map_err(me)?,
    ))
}

// ---------- 实体（记忆星系） ----------

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
    require_memory(&principal)?;
    Ok(Json(
        search_entities(&state.pool, &p.q, p.limit.unwrap_or(20)).await?,
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
    require_memory(&principal)?;
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
    require_memory(&principal)?;
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
    require_memory(&principal)?;
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

/// 记忆域缺失向量统计（原子/场景）。
#[utoipa::path(get, path = "/memory/embeddings/status",
    responses((status = 200, body = EmbeddingStatus)))]
pub async fn embedding_status(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<Json<EmbeddingStatus>, ApiError> {
    require_memory(&principal)?;
    Ok(Json(svc(&state).embedding_status().await.map_err(me)?))
}

/// 入队重嵌（换 embedding 供应商后的修复路径）。
#[utoipa::path(post, path = "/memory/reembed",
    responses((status = 202, description = "重嵌 job 已入队")))]
pub async fn reembed_memory(
    principal: axum::Extension<Principal>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    require_memory(&principal)?;
    svc(&state).reembed().await.map_err(me)?;
    Ok(StatusCode::ACCEPTED)
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

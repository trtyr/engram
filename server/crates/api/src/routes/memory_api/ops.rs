//! `memory_api` 的实现切片（架构治理 2026-09-21：自 memory_api.rs 纯搬移，零行为变化）。

use super::*;

/// cron 专属 scope（memory-rhythm 分权）：cron 通道与心跳只能由 cron key 走——
/// 否则 AI 可伪造 `via:"cron"` 审计行、可伪造心跳掩盖 cron 失联。AI 别碰这条线。
pub(crate) fn require_cron(p: &Principal) -> Result<(), ApiError> {
    if p.has_scope("cron") {
        return Ok(());
    }
    Err(ApiError::Forbidden(
        "cron 通道需要 cron scope 的独立 key（AI 主动蒸馏走 manual 不带 via；心跳是 cron 专属，AI 别碰——设置页会读 status 判 cron 死活）".into(),
    ))
}

/// erase scope（破坏性分权）：删实体/摘原子/删关系与擦除会话同级——admin 全权 / erase scope。
/// 权限收窄后 AI（memory-only key）能读能建（走会话蒸馏），但不能毁（删除类操作都要 erase）。
pub(crate) fn require_erase(p: &Principal) -> Result<(), ApiError> {
    if p.has_scope("erase") {
        return Ok(());
    }
    Err(ApiError::Forbidden(
        "删除需要 erase scope（不可逆操作，与读写分权）".into(),
    ))
}

pub(crate) fn me(e: MemoryError) -> ApiError {
    match e {
        MemoryError::NotFound(m) => ApiError::NotFound(m),
        MemoryError::BadRequest(m) => ApiError::BadRequest(m),
        MemoryError::Storage(m) => ApiError::Unavailable(m),
        MemoryError::LlmNotConfigured(m) => ApiError::BadRequest(format!("LLM 未配置——{m}")),
    }
}

pub(crate) fn svc(state: &AppState) -> MemoryService {
    MemoryService::new(state.pool.clone(), state.registry())
}

pub(crate) fn default_conf() -> f32 {
    0.9
}

/// 宽容时间反序列化：RFC3339 全形态或 date-only（"2026-09-02" → 当日零点 UTC）——
/// 与 distill 层 parse_iso 同一套语义，API/蒸馏两层不再打架。
pub(crate) fn parse_flex_datetime(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let t = s.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t) {
        return Some(dt.into());
    }
    chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|ndt| chrono::DateTime::from_naive_utc_and_offset(ndt, chrono::Utc))
}

pub(crate) fn opt_flex_dt<'de, D>(d: D) -> Result<Option<chrono::DateTime<chrono::Utc>>, D::Error>
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
    require_erase_scope(&principal)?;

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
            let n = engram_jobs::admin::cancel_pending_deep_purge(&state.pool, job_id)
                .await
                .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))?;
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
            let armed = engram_jobs::admin::find_armed_deep_purge(&state.pool, token)
                .await
                .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))?;
            let Some((job_id, mut payload)) = armed else {
                return Err(ApiError::BadRequest(
                    "token 无效或已过期（armed 状态 5 分钟，到期自动执行或已被取消/执行）".into(),
                ));
            };
            let counts = svc(&state).purge_deep().await.map_err(me)?;
            payload["executed_by"] = serde_json::json!(source);
            engram_jobs::admin::complete_deep_purge(&state.pool, job_id, &payload, &counts)
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
            .export(p.include_sensitive.unwrap_or(true)) // 敏感口径放开（2026-09-12）——默认全量
            .await
            .map_err(me)?,
    ))
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

/// 审计/留痕用主体来源："admin" / "key:名"
pub(crate) fn actor_of(principal: &Principal) -> String {
    match principal {
        Principal::Admin => "admin".to_string(),
        Principal::ApiKey { name, .. } => format!("key:{name}"),
    }
}

/// F1 双因子：erase scope（持有）× 逐操作二校验。
fn require_erase_scope(principal: &Principal) -> Result<(), ApiError> {
    // F1 双因子：erase scope（持有）× 逐操作二校验
    match principal {
        Principal::Admin => {}
        Principal::ApiKey { scopes, .. } if scopes.iter().any(|s| s == "erase") => {}
        _ => {
            return Err(ApiError::Forbidden(
                "清场需要 erase scope（不可逆操作，与读写分权）".into(),
            ));
        }
    }
    Ok(())
}

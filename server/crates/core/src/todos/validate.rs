use super::*;

/// update 字段校验（title/body/tags）。
pub(super) fn validate_todo_update_basic(
    kind: &str,
    title: Option<&str>,
    body: Option<&str>,
    tags: Option<&[String]>,
) -> Result<(), TodoError> {
    let kind = kind.to_string();
    if let Some(t) = title {
        let t = t.trim();
        if t.is_empty() {
            return Err(TodoError::BadRequest("title 不能为空".into()));
        }
        TodoService::reject_nul("title", t)?;
    }
    if let Some(b) = body {
        TodoService::reject_nul("body", b)?;
    }
    let tags = tags.map(TodoService::normalize_tags);
    if let Some(ts) = &tags {
        for t in ts {
            TodoService::reject_nul("tags", t)?;
        }
    }
    Ok(())
}

/// update 字段校验（priority 白名单 + 工单禁用 priority）。
pub(super) fn validate_todo_update_priority(
    kind: &str,
    priority: Option<&str>,
) -> Result<(), TodoError> {
    let kind = kind.to_string();
    if let Some(p) = priority {
        TodoService::validate_priority(p)?;
        if kind == "ticket" {
            return Err(TodoError::BadRequest(
                "工单分级用 severity（P0-P3）——priority 已对 ticket 退役".into(),
            ));
        }
    }
    Ok(())
}

/// update 字段校验（status/severity 白名单 + 工单转 resolved/verified 需解决记录）。
pub(super) fn validate_todo_update_status(
    kind: &str,
    status: Option<&str>,
    severity: Option<Option<&str>>,
    resolution: Option<&str>,
    existing_resolution: &str,
) -> Result<(), TodoError> {
    let kind = kind.to_string();
    // （update 的 kind 转换 todo→ticket 携带旧 priority 值属合法存量迁移，不拒）
    if let Some(s) = status
        && !valid_status(&kind, s)
    {
        return Err(status_error(&kind, s));
    }
    if let Some(Some(sv)) = severity
        && !SEVERITIES.contains(&sv)
    {
        return Err(TodoError::BadRequest(format!(
            "severity 仅接受 {}（收到 {sv}）",
            SEVERITIES.join("/")
        )));
    }
    if kind != "ticket" && severity.is_some() {
        return Err(TodoError::BadRequest(
            "severity 仅工单（kind=ticket）可用".into(),
        ));
    }
    // 工单转 resolved/verified 必须带解决记录（0041 CHECK 兜底前的友好版；
    // verified 从 resolved 来时 resolution 已有，不再强制重填）
    if kind == "ticket"
        && let Some(st) = status
        && ["resolved", "verified"].contains(&st)
        && resolution
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .or(Some(existing_resolution))
            .filter(|r| !r.is_empty())
            .is_none()
    {
        return Err(TodoError::BadRequest(
            "工单转 resolved/verified 必须填写解决记录（resolution）——做了什么/怎么修的".into(),
        ));
    }
    Ok(())
}

/// 建单校验（severity 白名单 + 仅工单可用；tags 归一化 + NUL 拒绝），返回归一化 tags。
pub(super) fn prepare_todo_create(
    kind: &str,
    severity: Option<&str>,
    tags: &[String],
) -> Result<Vec<String>, TodoError> {
    if let Some(sv) = severity {
        if !SEVERITIES.contains(&sv) {
            return Err(TodoError::BadRequest(format!(
                "severity 仅接受 {}（收到 {sv}）",
                SEVERITIES.join("/")
            )));
        }
        if kind != "ticket" {
            return Err(TodoError::BadRequest(
                "severity 仅工单（kind=ticket）可用——todo 不需要严重度".into(),
            ));
        }
    }
    // 工单建议带症状描述（不强制——建票后可补）
    let tags = TodoService::normalize_tags(tags);
    for t in &tags {
        TodoService::reject_nul("tags", t)?;
    }
    Ok(tags)
}

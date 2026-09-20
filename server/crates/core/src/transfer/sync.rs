use super::*;

/// 从远端 engram 拉取迁移包并导入本机。返回导入报告（含来源与包 counts）。
pub async fn pull_from(
    pool: &PgPool,
    source_base: &str,
    source_admin_password: &str,
) -> Result<Value> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| TransferError::Storage(format!("HTTP 客户端构建失败: {e}")))?;

    // 1) 登录 A（拿 ams_ 会话）
    let login: Value = client
        .post(format!("{source_base}/auth/login"))
        .json(&serde_json::json!({ "password": source_admin_password }))
        .send()
        .await
        .map_err(|e| TransferError::BadRequest(format!("连接源机失败（{source_base}）: {e}")))?
        .error_for_status()
        .map_err(|e| {
            TransferError::BadRequest(match e.status() {
                Some(code) if code.as_u16() == 401 => "源机认证失败：admin 密码错误".into(),
                Some(code) => format!("源机登录失败：HTTP {code}"),
                _ => e.to_string(),
            })
        })?
        .json()
        .await
        .map_err(|e| TransferError::BadRequest(format!("源机登录响应解析失败: {e}")))?;
    let token = login
        .get("token")
        .and_then(|x| x.as_str())
        .ok_or_else(|| TransferError::BadRequest("源机登录响应缺 token——确认对端是 engram".into()))?
        .to_string();

    // 2) 拉迁移包
    let bundle = client
        .get(format!("{source_base}/migrate/export"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| TransferError::BadRequest(format!("拉取迁移包失败: {e}")))?
        .error_for_status()
        .map_err(|e| {
            TransferError::BadRequest(format!(
                "源机导出失败：HTTP {}",
                e.status().map(|c| c.as_u16()).unwrap_or(0)
            ))
        })?
        .json::<Value>()
        .await
        .map_err(|e| TransferError::BadRequest(format!("迁移包解析失败: {e}")))?;

    let counts = bundle.get("counts").cloned().unwrap_or(json!({}));

    // 3) 落地本机
    let report = import_bundle(pool, &bundle).await?;

    Ok(json!({
        "source": source_base,
        "bundle_counts": counts,
        "imported": report,
    }))
}

/// 校验并归一目标地址：非 loopback 且非 https 时拒绝（allow_insecure 逃生）。返回去尾斜杠地址。
pub fn check_sync_target(
    target: &str,
    allow_insecure: bool,
) -> std::result::Result<String, String> {
    let raw = target.trim().trim_end_matches('/');
    let (scheme, rest) = match raw.split_once("://") {
        Some((s, r)) => (s.to_ascii_lowercase(), r),
        None => return Err("target_url 必须含 scheme（http(s)://）".into()),
    };
    let host_end = rest.find(['/', ':']).unwrap_or(rest.len());
    let host = &rest[..host_end];
    let loopback = matches!(host, "127.0.0.1" | "localhost" | "::1");
    if scheme != "https" && !loopback && !allow_insecure {
        return Err(format!(
            "目标 {raw} 非 loopback 且为 http 明文——公网同步必须走 TLS；ssh 隧道后可用 http://127.0.0.1:<port>；确要明文请传 allow_insecure=true"
        ));
    }
    Ok(raw.to_string())
}

/// 双向同步转发。dry_run=true 时 push 不连目标（仅本地导出对比）、pull 只拉不写。
pub async fn sync_transfer(
    pool: &PgPool,
    direction: &str,
    target_base: &str,
    token: &str,
    allow_insecure: bool,
    dry_run: bool,
) -> Result<SyncOutcome> {
    if direction != "push" && direction != "pull" {
        return Err(TransferError::BadRequest(
            "direction 只支持 push/pull".into(),
        ));
    }
    let base = check_sync_target(target_base, allow_insecure).map_err(TransferError::BadRequest)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| TransferError::Storage(format!("HTTP 客户端构建失败: {e}")))?;

    match direction {
        "push" => sync_push(&client, &base, token, pool, dry_run).await,
        "pull" => sync_pull(&client, &base, token, pool, dry_run).await,
        _ => unreachable!("direction 已在入口校验"),
    }
}

/// push：本地导出 → POST 目标 /migrate/import（dry_run 仅导出对比，不连目标）。
pub(super) async fn sync_push(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    pool: &PgPool,
    dry_run: bool,
) -> Result<SyncOutcome> {
    let bundle = export_bundle(pool).await?;
    let source_counts = bundle.get("counts").cloned().unwrap_or(Value::Null);
    if dry_run {
        return Ok(SyncOutcome {
            direction: "push",
            source_counts,
            dry_run: true,
            import_report: None,
        });
    }
    let resp = client
        .post(format!("{base}/migrate/import"))
        .bearer_auth(token)
        .json(&bundle)
        .send()
        .await
        .map_err(|e| TransferError::Storage(format!("目标不可达: {e}")))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(TransferError::Storage(format!(
            "目标导入失败（HTTP {status}）: {}",
            serde_json::to_string(&body).unwrap_or_default()
        )));
    }
    Ok(SyncOutcome {
        direction: "push",
        source_counts,
        dry_run: false,
        import_report: Some(body),
    })
}

/// pull：GET 目标 /migrate/export → 本地导入（dry_run 只拉不写）。
pub(super) async fn sync_pull(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    pool: &PgPool,
    dry_run: bool,
) -> Result<SyncOutcome> {
    let resp = client
        .get(format!("{base}/migrate/export"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| TransferError::Storage(format!("目标不可达: {e}")))?;
    let status = resp.status();
    let bundle: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(TransferError::Storage(format!(
            "目标导出失败（HTTP {status}）: {}",
            serde_json::to_string(&bundle).unwrap_or_default()
        )));
    }
    let source_counts = bundle.get("counts").cloned().unwrap_or(Value::Null);
    if dry_run {
        return Ok(SyncOutcome {
            direction: "pull",
            source_counts,
            dry_run: true,
            import_report: None,
        });
    }
    let report = import_bundle(pool, &bundle).await?;
    Ok(SyncOutcome {
        direction: "pull",
        source_counts,
        dry_run: false,
        import_report: Some(report),
    })
}

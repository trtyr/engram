//! 内置节律集成测试（内置节律线 roadmap v3）：
//! 启动自检补建幂等（重复调用不重复投递）、停用短路、/settings/rhythm 配置端点。

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::util::ServiceExt;

mod support;

#[tokio::test]
async fn rhythm_bootstrap_idempotent_and_disabled_short_circuits() {
    let (_app, pg) = support::app().await;
    let url = support::connection_url(&pg).await.unwrap();
    let pool = support::connect_with_retry(&url).await.unwrap();
    let queue = engram_jobs::JobQueue::new(pool.clone());
    let kinds = [
        "rhythm_extract".to_string(),
        "rhythm_consolidate".to_string(),
    ];

    // 首次补建：两期待办在队（周期增量 + 每日全量）
    let n = engram_distill::rhythm::bootstrap(&queue, &pool)
        .await
        .unwrap();
    assert_eq!(n, 2, "首次补建应排两期");
    let jobs = queue.list(&kinds, &[], None, 50).await.unwrap();
    assert_eq!(jobs.len(), 2);
    for j in &jobs {
        assert_eq!(
            j.status,
            engram_jobs::types::JobStatus::Pending,
            "补建产物必须是待办：{j:?}"
        );
        assert!(j.due_at > chrono::Utc::now(), "下一期必须在未来：{j:?}");
    }

    // 重复补建（重启语义）：幂等键命中复用，不重复投递
    let n2 = engram_distill::rhythm::bootstrap(&queue, &pool)
        .await
        .unwrap();
    assert_eq!(n2, 2);
    let jobs2 = queue.list(&kinds, &[], None, 50).await.unwrap();
    assert_eq!(jobs2.len(), 2, "重复补建不得新增");

    // 键形状：extract=槽位键；consolidate=日期-小时（缺省钟点 3）
    for j in &jobs2 {
        let key = j.idempotency_key.as_deref().unwrap_or_default();
        match j.kind.as_str() {
            "rhythm_extract" => {
                assert!(key.starts_with("rhythm-extract-"), "extract 键形状：{key}")
            }
            "rhythm_consolidate" => {
                assert!(
                    key.starts_with("rhythm-consolidate-") && key.ends_with("-03"),
                    "consolidate 键形状（缺省钟点 03）：{key}"
                );
            }
            other => panic!("意外任务种类 {other}"),
        }
    }

    // 停用：bootstrap 短路（不续排；既有待办保留——下一期跑完自然停）
    engram_storage::repo::settings::put_json(&pool, "rhythm", &json!({"enabled": false}))
        .await
        .unwrap();
    let n3 = engram_distill::rhythm::bootstrap(&queue, &pool)
        .await
        .unwrap();
    assert_eq!(n3, 0, "停用后不得续排");
    let jobs3 = queue.list(&kinds, &[], None, 50).await.unwrap();
    assert_eq!(jobs3.len(), 2, "既有待办保留");
}

#[tokio::test]
async fn rhythm_config_endpoints_admin_surface() {
    let (app, _pg) = support::app().await;
    let token = support::login_token(&app).await;

    // GET：settings 缺行 → 缺省配置
    let (st, v) = send(&app, "GET", "/settings/rhythm", &token, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["enabled"], json!(true));
    assert_eq!(v["extract_every_hours"], json!(6));
    assert_eq!(v["consolidate_hour_local"], json!(3));

    // PUT 合法：落库并回显
    let (st, v) = send(
        &app,
        "PUT",
        "/settings/rhythm",
        &token,
        Some(json!({
            "enabled": true,
            "extract_every_hours": 12,
            "consolidate_hour_local": 5
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["extract_every_hours"], json!(12));

    // 再 GET：读到已存值
    let (_, v) = send(&app, "GET", "/settings/rhythm", &token, None).await;
    assert_eq!(v["extract_every_hours"], json!(12));
    assert_eq!(v["consolidate_hour_local"], json!(5));

    // PUT 越界 → 400（周期下界 / 钟点上界各一）
    let (st, _) = send(
        &app,
        "PUT",
        "/settings/rhythm",
        &token,
        Some(json!({"enabled": true, "extract_every_hours": 0, "consolidate_hour_local": 3})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = send(
        &app,
        "PUT",
        "/settings/rhythm",
        &token,
        Some(json!({"enabled": true, "extract_every_hours": 6, "consolidate_hour_local": 24})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

/// 带 Authorization 的 JSON 请求（同 migrate_scope_test 口径）。
async fn send(
    app: &axum::Router,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder().method(method).uri(path);
    let req = match body {
        Some(v) => builder
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => builder
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    };
    let res = app.clone().oneshot(req).await.unwrap();
    let st = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let v = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (st, v)
}

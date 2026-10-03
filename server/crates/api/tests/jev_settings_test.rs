//! JEV 决策模型配置端点集成测试（决策 001 · T013）：
//! GET 脱敏（key 永不回显）、PUT 加密落库 + key_configured 翻转、阈值校验、
//! enabled 需先配 key、非 Admin 拒绝。

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::util::ServiceExt;

mod support;

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

#[tokio::test]
async fn jev_config_endpoints_admin_surface() {
    let (app, _pg) = support::app().await;
    let token = support::login_token(&app).await;

    // GET：settings 缺行 → 缺省配置（关 / 默认模型 / 双阈），provider 固定 openrouter
    let (st, v) = send(&app, "GET", "/settings/jev", &token, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["enabled"], json!(false));
    assert_eq!(v["model"], json!("typesafe/jev-1.13"));
    assert_eq!(v["key_configured"], json!(false));
    assert_eq!(v["provider"], json!("openrouter"));
    assert!(v.get("api_key_enc").is_none(), "GET 不得回显密文");

    // enabled=true 但未配 key → 400
    let (st, _) = send(
        &app,
        "PUT",
        "/settings/jev",
        &token,
        Some(json!({"enabled": true})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // 配 key + 启用 → 200，key_configured 翻转，密文不回显
    let (st, v) = send(
        &app,
        "PUT",
        "/settings/jev",
        &token,
        Some(json!({"enabled": true, "api_key": "sk-or-test-key", "reject_threshold": 0.25})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "body={v}");
    assert_eq!(v["enabled"], json!(true));
    assert_eq!(v["key_configured"], json!(true));
    assert_eq!(v["reject_threshold"], json!(0.25));
    assert!(v.get("api_key_enc").is_none());

    // 再 GET：持久化生效
    let (_, v) = send(&app, "GET", "/settings/jev", &token, None).await;
    assert_eq!(v["enabled"], json!(true));
    assert_eq!(v["key_configured"], json!(true));
    assert_eq!(v["reject_threshold"], json!(0.25));

    // 阈值乱序（reject ≥ review）→ 400
    let (st, _) = send(
        &app,
        "PUT",
        "/settings/jev",
        &token,
        Some(json!({"reject_threshold": 0.6, "review_threshold": 0.5})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // 阈值越上界 → 400
    let (st, _) = send(
        &app,
        "PUT",
        "/settings/jev",
        &token,
        Some(json!({"review_threshold": 1.0})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // 空 key 忽略（不抹掉已存值）、model 更新生效
    let (st, v) = send(
        &app,
        "PUT",
        "/settings/jev",
        &token,
        Some(json!({"api_key": "  ", "model": "typesafe/jev-next"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["key_configured"], json!(true), "空串 key 不得清掉已存值");
    assert_eq!(v["model"], json!("typesafe/jev-next"));
}

#[tokio::test]
async fn jev_config_requires_admin() {
    let (app, _pg) = support::app().await;
    // 未带 token → 401/403（bearer 缺失在鉴权层拒绝）
    let req = Request::builder()
        .method("GET")
        .uri("/settings/jev")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert!(
        res.status() == StatusCode::UNAUTHORIZED || res.status() == StatusCode::FORBIDDEN,
        "无凭证不得放行：{}",
        res.status()
    );
}

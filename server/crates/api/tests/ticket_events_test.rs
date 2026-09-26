//! 工单活动时间线 HTTP 旅程（task-4）：建工单 → 状态流转自动留痕 → 评论入流 → 时间线升序时序正确。
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

mod support;

async fn req(
    app: &axum::Router,
    tok: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    b = b.header("authorization", format!("Bearer {tok}"));
    let req = match body {
        Some(v) => b
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, v)
}

#[tokio::test]
async fn ticket_timeline_http_journey() {
    let (app, _pg) = support::app().await;
    let tok = support::login_token(&app).await;

    // ① 建工单
    let (st, v) = req(
        &app,
        &tok,
        "POST",
        "/todos",
        Some(json!({ "title": "T 时间线", "kind": "ticket", "severity": "P2" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let id = v["id"].as_str().unwrap().to_string();

    // ② 状态流转 open → confirmed（自动留痕）
    let (st, v) = req(
        &app,
        &tok,
        "PUT",
        &format!("/todos/{id}"),
        Some(json!({ "status": "confirmed" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");

    // ③ 评论入流
    let (st, v) = req(
        &app,
        &tok,
        "POST",
        &format!("/todos/{id}/events"),
        Some(json!({ "text": "控制台评论" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");

    // ④ 时间线：event(open→confirmed) + comment，升序、payload 正确
    let (st, v) = req(&app, &tok, "GET", &format!("/todos/{id}/events"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let ev = v["events"].as_array().unwrap();
    assert_eq!(ev.len(), 2, "{v}");
    assert_eq!(ev[0]["kind"], "event");
    assert_eq!(ev[0]["payload"]["from"], "open");
    assert_eq!(ev[0]["payload"]["to"], "confirmed");
    assert_eq!(ev[0]["actor"], "console");
    assert_eq!(ev[1]["kind"], "comment");
    assert_eq!(ev[1]["payload"]["text"], "控制台评论");
}

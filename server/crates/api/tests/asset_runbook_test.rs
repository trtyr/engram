//! 资产运行手册 HTTP 旅程测试：建档 → PUT v1 → PUT v2 → GET=v2 → versions=1(old=v1)
//! → restore 回 v1（versions=2）→ detail 携 runbook。
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
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
    let v: Value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap_or(Value::Null)
    };
    (status, v)
}

#[tokio::test]
async fn asset_runbook_http_journey() {
    let (app, _pg) = support::app().await;
    let tok = support::login_token(&app).await;

    // ① 建档一台主机
    let (st, v) = req(
        &app,
        &tok,
        "POST",
        "/assets",
        Some(serde_json::json!({"kind":"host","name":"rb-http-host","ip":"10.0.0.9","os":"macOS"})),
    )
    .await;
    assert!(st == StatusCode::CREATED || st == StatusCode::OK, "{v}");
    let id = v["id"].as_str().expect("建档返回 id").to_string();

    // ② PUT runbook v1 → v2
    let (st, v) = req(
        &app,
        &tok,
        "PUT",
        &format!("/assets/{id}/runbook"),
        Some(serde_json::json!({"md":"# v1\n- 磁盘 2T","editor":"tester"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, _) = req(
        &app,
        &tok,
        "PUT",
        &format!("/assets/{id}/runbook"),
        Some(serde_json::json!({"md":"# v2\n- 磁盘 2T\n- 32G","editor":"tester"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    // ③ GET = v2
    let (st, v) = req(&app, &tok, "GET", &format!("/assets/{id}/runbook"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["runbook_md"], "# v2\n- 磁盘 2T\n- 32G");

    // ④ versions：1 条（old = v1）
    let (st, v) = req(
        &app,
        &tok,
        "GET",
        &format!("/assets/{id}/runbook/versions"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let versions = v["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 1, "{v}");
    assert_eq!(versions[0]["old_runbook_md"], "# v1\n- 磁盘 2T");
    let vid = versions[0]["id"].as_str().unwrap().to_string();

    // ⑤ restore 回 v1（versions 变 2）
    let (st, _) = req(
        &app,
        &tok,
        "POST",
        &format!("/assets/{id}/runbook/restore"),
        Some(serde_json::json!({"version_id": vid, "editor":"tester"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (_, v) = req(&app, &tok, "GET", &format!("/assets/{id}/runbook"), None).await;
    assert_eq!(v["runbook_md"], "# v1\n- 磁盘 2T");
    let (_, v) = req(
        &app,
        &tok,
        "GET",
        &format!("/assets/{id}/runbook/versions"),
        None,
    )
    .await;
    assert_eq!(
        v["versions"].as_array().unwrap().len(),
        2,
        "回滚本身留痕：{v}"
    );

    // ⑥ detail 携 runbook_md
    let (st, v) = req(&app, &tok, "GET", &format!("/assets/{id}"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["runbook_md"], "# v1\n- 磁盘 2T");
}

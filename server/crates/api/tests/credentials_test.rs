//! 凭据域 HTTP 面旅程测试（EN-234 控制台治理面）：
//! 写入 → 台账（不含值）→ 揭示留痕 → 取用流水 → 同名换值清零旧审计 → 删除级联。
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
async fn credentials_http_journey() {
    let (app, _pg) = support::app().await;
    let tok = support::login_token(&app).await;

    // ① 写入：201，返回元数据不含值
    let (st, v) = req(
        &app,
        &tok,
        "POST",
        "/credentials",
        Some(serde_json::json!({"name":"t/journey","value":"secret-abc","description":"测试"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let meta = &v["credential"];
    assert_eq!(meta["name"], "t/journey");
    assert!(meta.get("value").is_none(), "元数据永不回显值：{meta}");

    // ② 台账：条目在，值不在
    let (st, v) = req(&app, &tok, "GET", "/credentials", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let items = v["items"].as_array().unwrap();
    let row = items
        .iter()
        .find(|i| i["name"] == "t/journey")
        .expect("台账含 t/journey");
    assert!(row.get("value").is_none(), "台账永不回显值：{row}");
    assert_eq!(row["read_count"], 0);

    // ③ 揭示：值返回 + 留痕（read_count=1）
    let (st, v) = req(&app, &tok, "GET", "/credentials/t%2Fjourney/value", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["value"], "secret-abc");
    assert_eq!(v["read_count"], 1, "揭示留痕：read_count 递增");

    // ④ 流水：一行（谁/何时）
    let (st, v) = req(&app, &tok, "GET", "/credentials/t%2Fjourney/reads", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["reads"].as_array().unwrap().len(), 1, "揭示后流水 1 条");

    // ⑤ 同名换值：流水清零（值变了旧痕作废）
    let (st, _) = req(
        &app,
        &tok,
        "POST",
        "/credentials",
        Some(serde_json::json!({"name":"t/journey","value":"secret-v2"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, v) = req(&app, &tok, "GET", "/credentials/t%2Fjourney/reads", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        v["reads"].as_array().unwrap().len(),
        0,
        "换值后旧流水清零：{v}"
    );
    let (st, v) = req(&app, &tok, "GET", "/credentials/t%2Fjourney/value", None).await;
    assert_eq!(v["value"], "secret-v2", "换值后取到新值");

    // ⑥ 删除：级联清流水；再揭示 404
    let (st, _) = req(&app, &tok, "DELETE", "/credentials/t%2Fjourney", None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = req(&app, &tok, "GET", "/credentials/t%2Fjourney/value", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "删后揭示应 404");
}

//! study HTTP 端点集成测试（P007 二期 T008）：CRUD 全流程 + :ro 拒写 + 非 study scope 403。

mod support;

use tower::ServiceExt;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use support::{app, create_key, login_token};

/// 发 JSON 请求的便捷封装。
async fn req_json(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    let req = match body {
        Some(b) => builder
            .header("content-type", "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, v)
}

async fn req_empty(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: &str,
) -> (StatusCode, Value) {
    req_json(app, method, uri, token, None).await
}

#[tokio::test]
async fn study_http_crud_full_flow() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    // ① 开题 → 201
    let (st, v) = req_json(
        &app,
        "POST",
        "/study/topics",
        &admin,
        Some(json!({"name": "RAG 入门", "goal": "能设计切分管线"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let topic = v["id"].as_str().unwrap().to_string();

    // ② list → 1 条
    let (st, v) = req_empty(&app, "GET", "/study/topics", &admin).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["count"], json!(1));

    // ③ 加两个知识点 → 201
    let (st, v) = req_json(
        &app,
        "POST",
        &format!("/study/topics/{topic}/items"),
        &admin,
        Some(json!({"name": "基础流程", "position": 10})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let item_a = v["id"].as_str().unwrap().to_string();
    let (st, _) = req_json(
        &app,
        "POST",
        &format!("/study/topics/{topic}/items"),
        &admin,
        Some(json!({"name": "切分策略"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // ④ get 全量：进度 0/2、next_up 2
    let (st, v) = req_empty(&app, "GET", &format!("/study/topics/{topic}"), &admin).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["progress"]["total"], json!(2));
    assert_eq!(v["progress"]["learned"], json!(0));
    assert_eq!(v["next_up"].as_array().unwrap().len(), 2);
    assert_eq!(v["goal"], "能设计切分管线", "serde flatten 后字段在顶层");

    // ⑤ 勾 learned + 挂 wiki
    let (st, _) = req_json(
        &app,
        "PATCH",
        &format!("/study/items/{item_a}"),
        &admin,
        Some(json!({"status": "learned", "wiki_slugs": ["rag-basic-pipeline"]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (_st, v) = req_empty(&app, "GET", &format!("/study/topics/{topic}"), &admin).await;
    assert_eq!(v["progress"]["learned"], json!(1));
    assert_eq!(v["next_up"].as_array().unwrap().len(), 1);

    // ⑥ topic 归档
    let (st, _) = req_json(
        &app,
        "PATCH",
        &format!("/study/topics/{topic}"),
        &admin,
        Some(json!({"status": "paused"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    // ⑦ 非法 status 拒
    let (st, _) = req_json(
        &app,
        "PATCH",
        &format!("/study/topics/{topic}"),
        &admin,
        Some(json!({"status": "bogus"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // ⑧ 删 item → 204，再删 topic → 204，get → 404
    let (st, _) = req_empty(&app, "DELETE", &format!("/study/items/{item_a}"), &admin).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _) = req_empty(&app, "DELETE", &format!("/study/topics/{topic}"), &admin).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _) = req_empty(&app, "GET", &format!("/study/topics/{topic}"), &admin).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn study_http_readonly_write_rejected() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;
    let ro = create_key(&app, &admin, &["study:ro"]).await;

    // 读放行
    let (st, _) = req_empty(&app, "GET", "/study/topics", &ro).await;
    assert_eq!(st, StatusCode::OK);

    // 写拒绝
    let (st, _) = req_json(
        &app,
        "POST",
        "/study/topics",
        &ro,
        Some(json!({"name": "x"})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, ":ro 写应 403");
}

#[tokio::test]
async fn study_http_wrong_scope_rejected() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;
    let other = create_key(&app, &admin, &["todos"]).await;

    let (st, _) = req_empty(&app, "GET", "/study/topics", &other).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "非 study scope 应 403");
}

#[tokio::test]
async fn study_http_reviews_and_journal() {
    // P007 二期 T011/T012：复习队列端点 + journal 时间线端点
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    // 建题+知识点
    let (st, v) = req_json(
        &app,
        "POST",
        "/study/topics",
        &admin,
        Some(json!({"name": "RAG 入门"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let topic = v["id"].as_str().unwrap().to_string();
    let (_st, v) = req_json(
        &app,
        "POST",
        &format!("/study/topics/{topic}/items"),
        &admin,
        Some(json!({"name": "基础流程"})),
    )
    .await;
    let item = v["id"].as_str().unwrap().to_string();

    // 标复习（立即到期）→ GET /study/reviews 命中
    let (st, _) = req_json(
        &app,
        "PATCH",
        &format!("/study/items/{item}"),
        &admin,
        Some(json!({"needs_review": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, v) = req_empty(&app, "GET", "/study/reviews", &admin).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["count"], json!(1), "立即到期应命中");

    // journal 记两笔 → GET 时间线（新→旧）+ topic_get 带 recent_journal
    for note in ["学了基础流程", "开始切分策略"] {
        let (st, _) = req_json(
            &app,
            "POST",
            &format!("/study/topics/{topic}/journal"),
            &admin,
            Some(json!({"note": note})),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED);
    }
    let (st, v) = req_empty(
        &app,
        "GET",
        &format!("/study/topics/{topic}/journal"),
        &admin,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["count"], json!(2));
    assert_eq!(v["journal"][0]["note"], "开始切分策略", "新→旧");

    let (st, v) = req_empty(&app, "GET", &format!("/study/topics/{topic}"), &admin).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        v["recent_journal"].as_array().unwrap().len(),
        2,
        "topic_get 应带时间线"
    );
}

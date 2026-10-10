//! /search 域准入与结果过滤（P019-M2）：
//! ① 准定改走 domain_access——:ro 变体放行（读语义），任意单域 key 可调；
//! ② 结果按 key 实际可读域过滤——wiki-only key 不再看到 todos/tickets/entity 命中。

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use support::{app, create_key, login_token};

async fn post_search(
    app: &axum::Router,
    key: &str,
    query: &str,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/search")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {key}"))
                .body(Body::from(format!(r#"{{"query":"{}","limit":20}}"#, query)))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    (status, v)
}

/// 种子：一条含唯一关键词的 todo（admin 建），供跨域命中断言用。
async fn seed_todo_with_keyword(app: &axum::Router, token: &str, keyword: &str) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/todos")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(format!(
                    r#"{{"title":"牵牛星检索种子 {keyword}"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "种子 todo 应建成");
}

#[tokio::test]
async fn search_ro_key_allowed_and_wiki_only_filtered() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;
    let keyword = format!("p019kw{}", uuid::Uuid::now_v7().simple());
    seed_todo_with_keyword(&app, &admin, &keyword).await;

    // ① wiki-only key：可调（有 wiki 读域），但结果不含 todos 域命中（旧行为越权可见）
    let wiki_key = create_key(&app, &admin, &["wiki"]).await;
    let (status, v) = post_search(&app, &wiki_key, &keyword).await;
    assert_eq!(status, StatusCode::OK, "wiki-only key 应可调 /search: {v}");
    let hits = v["hits"].as_array().unwrap();
    assert!(
        hits.iter().all(|h| h["domain"] == "wiki"),
        "wiki-only key 不得看到非 wiki 域命中: {hits:?}"
    );

    // ② admin（全权限）同查询：能看到该 todo 命中——证明过滤是按域裁剪而非检索失败
    let (status, v) = post_search(&app, &admin, &keyword).await;
    assert_eq!(status, StatusCode::OK);
    let hits = v["hits"].as_array().unwrap();
    assert!(
        hits.iter().any(|h| h["domain"] == "todo"),
        "admin 应看到 todo 命中: {hits:?}"
    );

    // ③ memory:ro 只读 key：可调（旧实现 has_scope 精确匹配直接 403）
    let ro = create_key(&app, &admin, &["memory:ro"]).await;
    let (status, _) = post_search(&app, &ro, &keyword).await;
    assert_eq!(status, StatusCode::OK, ":ro 只读 key 应可调 /search");

    // ④ 完全无关域（codegraph-only）：403
    let cg = create_key(&app, &admin, &["codegraph"]).await;
    let (status, v) = post_search(&app, &cg, &keyword).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "无任何检索域权限应 403: {v}");
}

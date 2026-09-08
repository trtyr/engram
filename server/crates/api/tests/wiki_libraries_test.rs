//! Wiki 多库（2026-09-08 A 方案）集成测试：
//! 库管理（建/列/删）+ 同名 slug 跨库隔离 + 图谱/检索按库 + purpose 每库一份。

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use support::{app, login_token};
use tower::util::ServiceExt;

struct Ctx {
    app: axum::Router,
    token: String,
}

impl Ctx {
    async fn new() -> Self {
        let (app, _pg) = app().await;
        let token = login_token(&app).await;
        Self { app, token }
    }

    async fn req(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {}", self.token))
            .header("content-type", "application/json");
        let req = match body {
            Some(b) => builder.body(Body::from(b.to_string())).unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let resp = self.app.clone().oneshot(req).await.unwrap();
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
}

#[tokio::test]
async fn wiki_multi_library_isolation() {
    let ctx = Ctx::new().await;

    // 1) 建第二个库 + 建库校验（slug 非法 / 撞名）
    let (st, lib) = ctx
        .req(
            "POST",
            "/wiki/libraries",
            Some(json!({"slug": "climb", "name": "攀岩库"})),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{lib}");
    assert_eq!(lib["slug"], "climb");
    let (st, _) = ctx
        .req(
            "POST",
            "/wiki/libraries",
            Some(json!({"slug": "bad slug!", "name": "x"})),
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "非法 slug 应 400");
    let (st, _) = ctx
        .req(
            "POST",
            "/wiki/libraries",
            Some(json!({"slug": "climb", "name": "x"})),
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "撞名应 400");

    // 2) 同名 slug 跨库共存：main 与 climb 各写一页 beta-page
    for (q, content) in [("main", "主库版本的 beta"), ("climb", "攀岩库版本的 beta")] {
        let (st, _) = ctx
            .req(
                "PUT",
                &format!("/wiki/pages/beta-page?lib={q}"),
                Some(json!({"title": "多库靶页", "content": content})),
            )
            .await;
        assert_eq!(st, StatusCode::OK, "{q}");
    }

    // 3) 列表/读取按库隔离
    let (st, main_pages) = ctx.req("GET", "/wiki/pages?lib=main", None).await;
    assert_eq!(st, StatusCode::OK);
    let main_hit = main_pages
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["slug"] == "beta-page")
        .expect("main 库应有靶页");
    assert_eq!(main_hit["content"], "主库版本的 beta");
    let (st, climb_pages) = ctx.req("GET", "/wiki/pages?lib=climb", None).await;
    assert_eq!(st, StatusCode::OK);
    let climb_hit = climb_pages
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["slug"] == "beta-page")
        .expect("climb 库应有同名页");
    assert_eq!(climb_hit["content"], "攀岩库版本的 beta");

    // 4) 检索按库：主库检索不命中 climb 的内容
    let (st, r) = ctx
        .req(
            "POST",
            "/wiki/search?lib=main",
            Some(json!({"query": "攀岩库版本"})),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        !r["pages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["slug"] == "beta-page"),
        "main 库检索不应命中 climb 内容"
    );

    // 5) purpose 每库一份
    let (st, _) = ctx
        .req(
            "PUT",
            "/wiki/purpose?lib=climb",
            Some(json!({"goals": ["攀岩知识沉淀"], "key_questions": [], "scope": []})),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (_, p_main) = ctx.req("GET", "/wiki/purpose?lib=main", None).await;
    let (_, p_climb) = ctx.req("GET", "/wiki/purpose?lib=climb", None).await;
    assert!(
        p_main.is_null()
            || p_main["goals"]
                .as_array()
                .map(|g| g.is_empty())
                .unwrap_or(true)
            || !p_main["goals"]
                .as_array()
                .unwrap()
                .iter()
                .any(|g| g.as_str().unwrap_or("").contains("攀岩")),
        "主库 purpose 不应被 climb 覆盖：{p_main}"
    );
    assert_eq!(p_climb["goals"][0], "攀岩知识沉淀");

    // 6) 未知库 → 404
    let (st, _) = ctx.req("GET", "/wiki/pages?lib=nope", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // 7) 删库：非空拒绝，force 级联后 main 不受影响
    let (st, e) = ctx.req("DELETE", "/wiki/libraries/climb", None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "非空库应拒绝删除：{e}");
    let (st, _) = ctx
        .req("DELETE", "/wiki/libraries/climb?force=true", None)
        .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = ctx.req("GET", "/wiki/pages?lib=climb", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "已删库应 404");
    let (_, main_pages) = ctx.req("GET", "/wiki/pages?lib=main", None).await;
    assert!(
        main_pages
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["slug"] == "beta-page"),
        "force 删 climb 不得波及 main"
    );
}

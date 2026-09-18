//! 会话身份归因 + 幂等键（公网多Agent P001 步骤1，2026-09-17）：
//! 同 client_ref 重写返回原会话不新建（网络重试防重）；
//! key 删除（revoke 路由实为硬删）不级联删会话：api_key_id 置 NULL、key 名快照保留。

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use support::{app, create_key, login_token};
use tower::util::ServiceExt;

struct Ctx {
    app: axum::Router,
    token: String,
    /// 测试库守卫：随 Ctx 活到测试结束（提前 drop 会中途 FORCE 删库）
    _pg: support::TestPg,
}

impl Ctx {
    async fn new() -> Self {
        let (app, _pg) = app().await;
        let token = login_token(&app).await;
        Self { app, token, _pg }
    }

    async fn req(
        &self,
        method: &str,
        path: &str,
        auth: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {auth}"))
            .header("content-type", "application/json");
        let req = match body {
            Some(b) => builder.body(Body::from(b.to_string())).unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let resp = self.app.clone().oneshot(req).await.unwrap();
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
}

fn payload(client_ref: &str) -> Value {
    json!({
        "agent": "tester",
        "turns": [{"speaker": "user", "text": "记住：多 Agent 幂等键验证"}],
        "client_ref": client_ref
    })
}

#[tokio::test]
async fn client_ref_rewrite_returns_original_session() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["memory"]).await;

    let (st, first) = ctx
        .req("POST", "/memory/sessions", &key, Some(payload("retry-1")))
        .await;
    assert_eq!(st, StatusCode::CREATED, "{first}");
    let id1 = first["id"].as_str().unwrap().to_string();
    assert_eq!(
        first["key_name_snapshot"].as_str(),
        Some("mcp-test"),
        "归因快照应为 key 名：{first}"
    );

    // 同 client_ref 重写（模拟网络重试）→ 返回原会话，不新建
    let (st, again) = ctx
        .req("POST", "/memory/sessions", &key, Some(payload("retry-1")))
        .await;
    assert_eq!(st, StatusCode::CREATED, "{again}");
    assert_eq!(
        again["id"].as_str().unwrap(),
        id1,
        "同 client_ref 应返回原会话：{again}"
    );

    // 不同 ref → 新会话
    let (st, other) = ctx
        .req("POST", "/memory/sessions", &key, Some(payload("retry-2")))
        .await;
    assert_eq!(st, StatusCode::CREATED, "{other}");
    assert_ne!(
        other["id"].as_str().unwrap(),
        id1,
        "不同 client_ref 应新建会话"
    );
}

#[tokio::test]
async fn key_removed_keeps_session_with_snapshot() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["memory"]).await;

    let (st, s) = ctx
        .req("POST", "/memory/sessions", &key, Some(payload("keep-1")))
        .await;
    assert_eq!(st, StatusCode::CREATED, "{s}");
    let sid = s["id"].as_str().unwrap().to_string();
    assert!(!s["api_key_id"].is_null(), "写入应带归因 key id：{s}");

    // 吊销写它的 key（产品口径：key 只有 revoke 没有硬删除）
    let (st, list) = ctx.req("GET", "/settings/api-keys", &ctx.token, None).await;
    assert_eq!(st, StatusCode::OK, "{list}");
    let key_id = list
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["name"].as_str() == Some("mcp-test"))
        .and_then(|k| k["id"].as_str())
        .expect("应能找到测试 key")
        .to_string();
    let (st, rv) = ctx
        .req(
            "POST",
            &format!("/settings/api-keys/{key_id}/revoke"),
            &ctx.token,
            None,
        )
        .await;
    assert!(st.is_success(), "吊销 key 应成功：{rv}");

    // 会话仍在（不级联删）：归因 key id 保留、key 名快照保留
    let (st, after) = ctx
        .req("GET", &format!("/memory/sessions/{sid}"), &ctx.token, None)
        .await;
    assert_eq!(st, StatusCode::OK, "{after}");
    assert_eq!(
        after["api_key_id"],
        Value::Null,
        "key 删除后 api_key_id 应 SET NULL：{after}"
    );
    assert_eq!(
        after["key_name_snapshot"].as_str(),
        Some("mcp-test"),
        "key 名快照应保留：{after}"
    );

    // 被吊销的 key 不能再写入（写入口已封，历史不丢）
    let (st, _) = ctx
        .req(
            "POST",
            "/memory/sessions",
            &key,
            Some(payload("after-revoke")),
        )
        .await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "吊销后的 key 应无法认证：{st}"
    );
}

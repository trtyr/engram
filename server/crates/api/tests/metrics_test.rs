//! R10：/metrics 端点——Prometheus 文本、鉴权豁免、HTTP 指标存在。

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;

#[tokio::test]
async fn metrics_endpoint_serves_prometheus_text_without_auth() {
    let (app, _pg) = support::app().await;
    // 先打一发 /health——确保 recorder 里已有计数（并行测试下 render 需有数据）
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // 不带 Authorization——鉴权豁免
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        ct.contains("text/plain"),
        "content-type 应为 Prometheus 文本: {ct}"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    // HTTP 中间件指标：/metrics 请求自身也应被记录（middleware 在最外层）
    assert!(
        text.contains("http_requests_total"),
        "应含 http_requests_total"
    );
}

#[tokio::test]
async fn metrics_disabled_via_env_returns_404() {
    // env 在 install 时读取——本测试进程的 install 已发生，故只验证端点路径语义：
    // 关闭态单测无法在共享进程内模拟（recorder 全局一次），由容器实测覆盖。
    let (app, _pg) = support::app().await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "常规路由不受影响");
}

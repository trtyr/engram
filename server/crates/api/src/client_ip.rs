//! 客户端 IP 注入（活跃会话归因用）。
//!
//! X-Forwarded-For 首段（反代场景）优先；否则取连接对端地址（直连场景，
//! 需 main 以 into_make_service_with_connect_info 启动）；测试 oneshot 无连接信息 → None。

use axum::{
    extract::{ConnectInfo, Request},
    middleware::Next,
    response::Response,
};
use std::net::SocketAddr;

/// 客户端 IP（None = 未知，如测试请求）。
#[derive(Clone, Debug)]
pub struct ClientIp(pub Option<String>);

pub async fn inject_client_ip(mut req: Request, next: Next) -> Response {
    let ip = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            req.extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ci| ci.0.ip().to_string())
        });
    req.extensions_mut().insert(ClientIp(ip));
    next.run(req).await
}

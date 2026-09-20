//! engram HTTP 服务库（bin 是薄壳，测试从这里进）。
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))] // 架构治理 task-5：生产代码禁裸崩溃（测试豁免）

pub mod auth;
pub mod client_ip;
pub mod config;
pub mod error;
pub mod login_throttle;
pub mod mcp_admin;
pub mod metrics;
pub mod routes;
pub mod state;
pub mod web_assets;

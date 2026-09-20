//! dump OpenAPI 到 stdout（CI 类型生成用）。
//! 用法：cargo run -p engram-api --bin openapi-dump > openapi.json
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))] // 架构治理 task-5：生产代码禁裸崩溃（测试豁免）

/// 不可失败（架构治理 task-5 分类 A：不可失败，保留并注明理由）。
#[allow(clippy::expect_used)]
fn main() {
    print!(
        "{}",
        engram_api::routes::openapi()
            .to_pretty_json()
            .expect("序列化")
    );
}

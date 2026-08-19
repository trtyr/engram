//! dump OpenAPI 到 stdout（CI 类型生成用）。
//! 用法：cargo run -p agent-memory-api --bin openapi-dump > openapi.json

fn main() {
    print!(
        "{}",
        agent_memory_api::routes::openapi()
            .to_pretty_json()
            .expect("序列化")
    );
}

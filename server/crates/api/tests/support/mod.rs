//! 测试基建：本机 PG 每测试一库（即建即删），不再依赖 Docker/testcontainers。
//!
//! 复用本机 Homebrew PostgreSQL（默认 127.0.0.1:5432，当前系统用户 trust 认证），
//! 每个测试创建唯一命名的临时数据库，Drop 时后台 DROP DATABASE（WITH FORCE）。
//! 隔离性与 testcontainers 等价；可用 AM_TEST_PG_URL 覆盖管理库连接串。

use sqlx::PgPool;
use std::sync::atomic::{AtomicU64, Ordering};

/// 管理库连接串（建库/删库用；指向已存在的库，通常 postgres）。
fn admin_url() -> String {
    std::env::var("AM_TEST_PG_URL").unwrap_or_else(|_| "postgres://127.0.0.1:5432/postgres".into())
}

static SEQ: AtomicU64 = AtomicU64::new(0);

/// 测试库守卫：Drop 时后台删库（防累积——残留库可手动 `DROP DATABASE am_test_*`）。
pub struct TestPg {
    pub db_name: String,
}

impl Drop for TestPg {
    fn drop(&mut self) {
        let sql = format!("DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)", self.db_name);
        let _ = std::process::Command::new("psql")
            .arg("-d")
            .arg(admin_url())
            .arg("-c")
            .arg(&sql)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

/// 建一个干净的测试库（扩展由 run_migrations 的 0001/0009 自建）。
/// 保留旧名以兼容既有调用点；语义 = "启动一个带 pgvector 的 PG 实例"。
pub async fn start_pgvector() -> anyhow::Result<TestPg> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let db_name = format!("am_test_{nanos}_{seq}");

    let admin = sqlx::PgPool::connect(&admin_url()).await?;
    // TEMPLATE template0：并发建库不与 template1 访问冲突（CREATE 会串行取模板锁）
    sqlx::query(&format!("CREATE DATABASE \"{db_name}\" TEMPLATE template0"))
        .execute(&admin)
        .await?;
    admin.close().await;
    Ok(TestPg { db_name })
}

pub async fn connection_url(t: &TestPg) -> anyhow::Result<String> {
    // 与 admin_url 同主机；库名唯一保证测试间隔离
    let admin = admin_url();
    let base = admin.trim_end_matches('/');
    let base = base
        .rsplit_once('/')
        .map(|(host, _)| host.to_string())
        .unwrap_or_else(|| base.to_string());
    Ok(format!("{base}/{}", t.db_name))
}

pub async fn connect_with_retry(url: &str) -> anyhow::Result<PgPool> {
    // 库已建好，正常一次即连；保留重试壳兼容旧签名
    for _ in 0..10 {
        if let Ok(pool) =
            engram_storage::connect_pool(&engram_storage::PoolConfig::new(url)).await
            && sqlx::query("SELECT 1").execute(&pool).await.is_ok()
        {
            return Ok(pool);
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    Err(anyhow::anyhow!("测试库连接超时: {url}"))
}

// ---------- MCP JSON-RPC 测试辅助（mcp_test / project_mcp_test 共用） ----------

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use engram_api::routes;
use engram_api::state::AppState;
use serde_json::{Value, json};
use tower::util::ServiceExt;

/// 完整测试 app：真 PG + 迁移 + 完整 router（含 Bearer 与 MCP 层）。
pub async fn app() -> (Router, TestPg) {
    let container = start_pgvector().await.expect("测试库");
    let url = connection_url(&container).await.unwrap();
    let pool = connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    let state = AppState::new(pool)
        .with_admin_password(Some("test-admin-pw".into()))
        .with_master_key(Some("ab".repeat(32)));
    (routes::router(state), container)
}

/// 管理员登录拿会话 token。
pub async fn login_token(app: &Router) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"password":"test-admin-pw"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    v["token"].as_str().unwrap().to_string()
}

/// 签发 API key（明文只出现这一次）。
pub async fn create_key(app: &Router, token: &str, scopes: &[&str]) -> String {
    let scopes_json = serde_json::json!(scopes);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/settings/api-keys")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(format!(
                    r#"{{"name":"mcp-test","scopes":{scopes_json}}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "签发 key 应成功");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    v["key"].as_str().unwrap().to_string()
}

/// 向 /mcp 发一条 JSON-RPC 请求，返回 HTTP 状态 + 响应 JSON（无状态模式：纯 JSON 响应）。
pub async fn mcp_rpc(app: &Router, auth: &str, payload: Value) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("host", "localhost")
                .header("authorization", format!("Bearer {auth}"))
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&body)
        .unwrap_or_else(|e| panic!("响应应为 JSON：{e}\n{}", String::from_utf8_lossy(&body)));
    (status, v)
}

pub fn rpc(id: i64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

/// 从 JSON-RPC 响应取 result（报 panic 带上下文）。
pub fn expect_result(v: &Value, what: &str) -> Value {
    assert!(
        v.get("error").is_none(),
        "{what} 不应报错：{}",
        v.get("error").unwrap_or(&Value::Null)
    );
    v["result"].clone()
}

/// initialize 握手（无状态模式下每条请求独立，但客户端仍按规范先 initialize）。
pub async fn mcp_initialize(app: &Router, auth: &str) -> Value {
    let (status, v) = mcp_rpc(
        app,
        auth,
        rpc(
            1,
            "initialize",
            json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "0.1.0"}
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    expect_result(&v, "initialize")
}

/// tools/call 快捷方式：断言成功并反序列化 content[0].text 的 JSON。
pub async fn mcp_call_json(app: &Router, auth: &str, name: &str, arguments: Value) -> Value {
    let (_, v) = mcp_rpc(
        app,
        auth,
        rpc(2, "tools/call", json!({"name": name, "arguments": arguments})),
    )
    .await;
    let out = expect_result(&v, &format!("tools/call {name}"));
    assert!(
        !out["isError"].as_bool().unwrap_or(false),
        "{name} 不应报错：{out}"
    );
    let text = out["content"][0]["text"].as_str().unwrap_or_else(|| {
        panic!("{name} 应返回文本内容：{out}");
    });
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{name} 返回应是 JSON：{e}\n{text}"))
}

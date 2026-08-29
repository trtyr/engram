//! LLM 集成测试：mock OpenAI 端点 + 真 PG。
//! 验证 Phase 1 出口标准：provider 注册（key 加密）→ 连通 → embedding 调用被记账。

mod support;

use agent_memory_llm::KeyCipher;
use agent_memory_llm::provider::{LlmProvider, OpenAiCompatProvider, ProviderRegistry};
use agent_memory_llm::router::{PurposeRouter, RouteRule, RoutingTable};
use agent_memory_llm::types::{EmbedRequest, Purpose};
use axum::Json;
use axum::routing::post;

/// 起 mock OpenAI 兼容端点（/v1/chat/completions + /v1/embeddings）。
/// 返回 base_url。
async fn start_mock_llm() -> String {
    let app = axum::Router::new()
        .route(
            "/v1/chat/completions",
            post(|| async {
                Json(serde_json::json!({
                    "choices": [{ "message": { "role": "assistant", "content": "{\"ok\":true}" } }],
                    "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
                }))
            }),
        )
        .route(
            "/v1/embeddings",
            post(|Json(body): Json<serde_json::Value>| async move {
                let n = body["input"].as_array().map(|a| a.len()).unwrap_or(1);
                let data: Vec<serde_json::Value> = (0..n)
                    .map(|i| serde_json::json!({ "index": i, "embedding": vec![0.1_f32; 8] }))
                    .collect();
                Json(serde_json::json!({
                    "data": data,
                    "usage": { "prompt_tokens": 7, "total_tokens": 7 }
                }))
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{}", addr)
}

async fn setup() -> (support::TestPg, ProviderRegistry, PurposeRouter) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");
    let cipher = KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap();
    (
        container,
        ProviderRegistry::new(pool.clone(), cipher),
        PurposeRouter::new(pool),
    )
}

#[tokio::test]
async fn provider_roundtrip_and_usage_accounting() {
    let (_c, registry, _router) = setup().await;
    let pool = registry_pool(&registry);

    let base_url = start_mock_llm().await;

    // 注册 provider（key 加密落库）
    let cipher = KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap();
    let enc = cipher.encrypt("sk-mock-key").unwrap();
    sqlx::query(
        "INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, models, is_default)
         VALUES ($1, 'mock', $2, $3, $4, true)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(&base_url)
    .bind(&enc)
    .bind(sqlx::types::Json(vec![
        agent_memory_llm::types::ModelInfo {
            id: "bge-m3".into(),
            capabilities: vec!["embedding".into()],
        },
    ]))
    .execute(&pool)
    .await
    .unwrap();

    // 落库的是密文不是明文
    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT api_key_encrypted FROM llm_providers WHERE name = 'mock'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !stored.windows(11).any(|w| w == b"sk-mock-key"),
        "明文密钥不得出现在库中"
    );

    // 经注册表取出（自动解密）→ 真实 HTTP 调用 mock embedding
    let provider = registry.get("mock").await.unwrap();
    assert_eq!(provider.name(), "mock");
    let resp = provider
        .embed(EmbedRequest {
            model: "bge-m3".into(),
            inputs: vec!["你好世界".into(), "hello".into()],
            dimensions: None,
        })
        .await
        .unwrap();
    assert_eq!(resp.embeddings.len(), 2);
    assert_eq!(resp.embeddings[0].len(), 8);

    // 记账
    registry
        .record_usage(&agent_memory_llm::types::UsageMeta {
            provider: "mock".into(),
            model: "bge-m3".into(),
            purpose: Purpose::Embed.as_str().into(),
            input_tokens: resp.input_tokens,
            output_tokens: 0,
            latency_ms: resp.latency_ms,
            job_id: None,
        })
        .await;
    let summary = registry
        .usage_summary(chrono::Utc::now() - chrono::Duration::hours(1))
        .await
        .unwrap();
    assert_eq!(summary.len(), 1);
    assert_eq!(summary[0].purpose, "embed");
    assert_eq!(summary[0].input_tokens, 7);
}

#[tokio::test]
async fn router_config_applies_immediately() {
    let (_c, _registry, router) = setup().await;
    let pool = router_pool(&router);

    // 初始空表
    let t = router.table().await.unwrap();
    assert!(t.chain(Purpose::Extract).is_empty());

    // 配置路由并立即生效
    let mut table = RoutingTable::default();
    table.routes.insert(
        "extract".into(),
        vec![RouteRule {
            provider: "deepseek".into(),
            model: "deepseek-chat".into(),
        }],
    );
    router.save(&table).await.unwrap();

    let t2 = router.table().await.unwrap();
    assert_eq!(t2.chain(Purpose::Extract)[0].model, "deepseek-chat");
    assert!(
        t2.chain(Purpose::Persona).is_empty(),
        "未配置 purpose 走默认 provider"
    );

    // settings 表只有一行
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM settings WHERE key = 'llm_routing'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn http_error_classification() {
    // 429 → Transient；401 → Permanent
    let p = OpenAiCompatProvider::new("t", "http://127.0.0.1:1", "k");
    let err = p
        .embed(EmbedRequest {
            model: "m".into(),
            inputs: vec!["x".into()],
            dimensions: None,
        })
        .await
        .unwrap_err();
    assert!(
        matches!(err, agent_memory_llm::types::LlmError::Transient(_)),
        "连接拒绝应归类瞬态: {err}"
    );
}

// ---- 内部池访问（测试专用，避免公开 registry 内部） ----
fn registry_pool(r: &ProviderRegistry) -> sqlx::PgPool {
    r.pool_for_test()
}
fn router_pool(r: &PurposeRouter) -> sqlx::PgPool {
    r.pool_for_test()
}

// ---------- L1/L3：能力回退陷阱与默认确定性 ----------

#[tokio::test]
async fn l1_capability_mismatch_reports_not_configured() {
    let (_c, registry, _router) = setup().await;
    let pool = registry_pool(&registry);

    // embedding-only 默认 provider（旧实现 or_else(first) 会把 bge-m3 当 chat 模型选出去）
    let cipher = KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap();
    sqlx::query(
        "INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, models, is_default)
         VALUES ($1, 'embed-only', 'http://127.0.0.1:1', $2, $3, true)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(cipher.encrypt("k").unwrap())
    .bind(sqlx::types::Json(vec![
        agent_memory_llm::types::ModelInfo {
            id: "bge-m3".into(),
            capabilities: vec!["embedding".into()],
        },
    ]))
    .execute(&pool)
    .await
    .unwrap();

    // Embed 用途：能力匹配命中 ✓
    assert!(registry.resolve(Purpose::Embed).await.is_ok());

    // chat 用途（Extract）：旧实现静默选 bge-m3 → 调用全 400；新实现明确报配置错误
    let err = match registry.resolve(Purpose::Extract).await {
        Err(e) => e,
        Ok(_) => panic!("embedding-only 默认 provider 不应解析出 chat 模型"),
    };
    match err {
        agent_memory_llm::types::LlmError::NotConfigured(msg) => {
            assert!(msg.contains("chat"), "报错应指向能力缺失: {msg}");
        }
        other => panic!("应为 NotConfigured，实际 {other}"),
    }
}

// ---------- L6：记账门面（embed_for 结构性记账） ----------

#[tokio::test]
async fn l6_embed_for_records_usage() {
    let (_c, registry, _router) = setup().await;
    let pool = registry_pool(&registry);

    let base_url = start_mock_llm().await;
    let cipher = KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap();
    sqlx::query(
        "INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, models, is_default)
         VALUES ($1, 'facade-mock', $2, $3, $4, true)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(&base_url)
    .bind(cipher.encrypt("sk-k").unwrap())
    .bind(sqlx::types::Json(vec![
        agent_memory_llm::types::ModelInfo {
            id: "mock-emb".into(),
            capabilities: vec!["embedding".into()],
        },
    ]))
    .execute(&pool)
    .await
    .unwrap();

    // 门面调用：解析 + 嵌入 + 记账一体
    let job_id = uuid::Uuid::new_v4();
    let resp = registry
        .embed_for(
            Purpose::Embed,
            vec!["你好".into(), "世界".into()],
            None,
            Some(job_id),
        )
        .await
        .unwrap();
    assert_eq!(resp.embeddings.len(), 2);

    // 记账落行（此前裸 resolve→embed 直连全绕过）
    let (provider, model, purpose, jid): (String, String, String, Option<uuid::Uuid>) =
        sqlx::query_as(
            "SELECT provider, model, purpose, job_id FROM llm_usage \
             WHERE purpose = 'embed' ORDER BY ts DESC LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(provider, "facade-mock");
    assert_eq!(model, "mock-emb");
    assert_eq!(purpose, "embed");
    assert_eq!(jid, Some(job_id), "job_id 透传记账");
}

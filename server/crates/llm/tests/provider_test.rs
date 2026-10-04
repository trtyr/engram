//! LLM 集成测试：mock OpenAI 端点 + 真 PG。
//! 验证 Phase 1 出口标准：provider 注册（key 加密）→ 连通 → embedding 调用被记账。

mod support;

use axum::Json;
use axum::routing::post;
use engram_llm::KeyCipher;
use engram_llm::provider::{LlmProvider, OpenAiCompatProvider, ProviderRegistry};
use engram_llm::types::{EmbedRequest, Purpose};
use tracing_subscriber::prelude::*;

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

async fn setup() -> (support::TestPg, ProviderRegistry) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    let cipher = KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap();
    (container, ProviderRegistry::new(pool, cipher))
}

#[tokio::test]
async fn provider_roundtrip_and_usage_accounting() {
    let (_c, registry) = setup().await;
    let pool = registry_pool(&registry);

    let base_url = start_mock_llm().await;

    // 注册 provider（key 加密落库）
    let cipher = KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap();
    let enc = cipher.encrypt("sk-mock-key").unwrap();
    sqlx::query(
        "INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, model_id, capability, is_default)
         VALUES ($1, 'mock', $2, $3, 'bge-m3', 'embedding', true)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(&base_url)
    .bind(&enc)
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
    let (provider, model) = registry.get("mock").await.unwrap();
    assert_eq!(provider.name(), "mock");
    assert_eq!(model, "bge-m3", "get 应返回 provider 的 model_id");
    let resp = provider
        .embed(EmbedRequest {
            model,
            inputs: vec!["你好世界".into(), "hello".into()],
            dimensions: None,
        })
        .await
        .unwrap();
    assert_eq!(resp.embeddings.len(), 2);
    assert_eq!(resp.embeddings[0].len(), 8);

    // 记账
    registry
        .record_usage(&engram_llm::types::UsageMeta {
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
        matches!(err, engram_llm::types::LlmError::Transient(_)),
        "连接拒绝应归类瞬态: {err}"
    );
}

// ---- 内部池访问（测试专用，避免公开 registry 内部） ----
fn registry_pool(r: &ProviderRegistry) -> sqlx::PgPool {
    r.pool_for_test()
}
// ---------- L1/L3：能力回退陷阱与默认确定性 ----------

#[tokio::test]
async fn l1_capability_mismatch_reports_not_configured() {
    let (_c, registry) = setup().await;
    let pool = registry_pool(&registry);

    // embedding-only 默认 provider（旧实现 or_else(first) 会把 bge-m3 当 chat 模型选出去）
    let cipher = KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap();
    sqlx::query(
        "INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, model_id, capability, is_default)
         VALUES ($1, 'embed-only', 'http://127.0.0.1:1', $2, 'bge-m3', 'embedding', true)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(cipher.encrypt("k").unwrap())
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
        engram_llm::types::LlmError::NotConfigured(msg) => {
            assert!(msg.contains("chat"), "报错应指向能力缺失: {msg}");
        }
        other => panic!("应为 NotConfigured，实际 {other}"),
    }
}

// ---------- L6：记账门面（embed_for 结构性记账） ----------

#[tokio::test]
async fn l6_embed_for_records_usage() {
    let (_c, registry) = setup().await;
    let pool = registry_pool(&registry);

    let base_url = start_mock_llm().await;
    let cipher = KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap();
    sqlx::query(
        "INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, model_id, capability, is_default)
         VALUES ($1, 'facade-mock', $2, $3, 'mock-emb', 'embedding', true)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(&base_url)
    .bind(cipher.encrypt("sk-k").unwrap())
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

// ---------- P005-T003：record_usage 结构化事件（成功调用全量日志） ----------

struct CollectLayer(std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>);

struct Collector {
    message: String,
    fields: serde_json::Map<String, serde_json::Value>,
}

impl tracing::field::Visit for Collector {
    fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
        if f.name() == "message" {
            self.message = format!("{v:?}");
        } else {
            self.fields
                .insert(f.name().into(), serde_json::json!(format!("{v:?}")));
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CollectLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut v = Collector {
            message: String::new(),
            fields: serde_json::Map::new(),
        };
        event.record(&mut v);
        self.0.lock().unwrap().push(serde_json::json!({
            "level": event.metadata().level().to_string(),
            "message": v.message,
            "fields": v.fields,
        }));
    }
}

/// record_usage 必发「LLM 调用」info! 事件（provider/model/purpose/token/latency 齐全）——
/// 生产经 api 层 PgLogLayer 落 logs 表。
#[tokio::test]
async fn record_usage_emits_structured_log_event() {
    let collected = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let layer = CollectLayer(collected.clone());
    let (_c, registry) = setup().await;

    // 全局注册（spawn 跨线程也生效；本文件其他测试的事件同入 collected，无碍断言）
    tracing::subscriber::set_global_default(tracing_subscriber::registry().with(layer)).unwrap();

    let handle = tokio::spawn(async move {
        registry
            .record_usage(&engram_llm::types::UsageMeta {
                provider: "mock".into(),
                model: "bge-m3".into(),
                purpose: "embed".into(),
                input_tokens: 7,
                output_tokens: 0,
                latency_ms: 42,
                job_id: None,
            })
            .await;
    });
    handle.await.unwrap();

    let events = collected.lock().unwrap();
    let hit = events
        .iter()
        .find(|e| e["message"] == "LLM 调用")
        .expect("record_usage 应发「LLM 调用」事件");
    assert_eq!(hit["level"], "INFO");
    assert_eq!(hit["fields"]["provider"], "mock");
    assert_eq!(hit["fields"]["purpose"], "embed");
    assert!(
        hit["fields"].get("input_tokens").is_some(),
        "token 字段应在"
    );
    assert!(hit["fields"].get("latency_ms").is_some(), "耗时字段应在");
}

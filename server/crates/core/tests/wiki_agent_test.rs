//! wiki Agent Harness 集成测试（P004-T010）：mock LLM 多轮工具循环 + 破坏性工具审计 + 预算降级。

mod support;

use tracing_subscriber::prelude::*;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use engram_core::wiki_agent::{run_agent, AgentBudget, AgentDeps, AgentTask};
use engram_llm::provider::LlmProvider;
use engram_llm::types::{ChatMessage, ChatRequest, ChatResponse, EmbedRequest, EmbedResponse, LlmError, ToolCall};

// ---------- mock provider（预置响应序列，耗尽后回 final） ----------

struct MockProvider {
    seq: Mutex<VecDeque<ChatResponse>>,
}

impl MockProvider {
    fn new(resps: Vec<ChatResponse>) -> Self {
        Self {
            seq: Mutex::new(resps.into()),
        }
    }
}

impl LlmProvider for MockProvider {
    fn chat(
        &self,
        _req: ChatRequest,
    ) -> impl std::future::Future<Output = Result<ChatResponse, LlmError>> + Send {
        let next = self.seq.lock().unwrap().pop_front();
        async move {
            Ok(next.unwrap_or(ChatResponse {
                content: "final 总结".into(),
                tool_calls: None,
                input_tokens: 1,
                output_tokens: 1,
                model: "mock-model".into(),
                latency_ms: 1,
            }))
        }
    }
    fn embed(
        &self,
        _req: EmbedRequest,
    ) -> impl std::future::Future<Output = Result<EmbedResponse, LlmError>> + Send {
        async { Err(LlmError::Permanent("mock 无嵌入".into())) }
    }
    fn name(&self) -> &str {
        "mock"
    }
}

fn resp(content: &str, calls: Vec<ToolCall>) -> ChatResponse {
    ChatResponse {
        content: content.into(),
        tool_calls: if calls.is_empty() { None } else { Some(calls) },
        input_tokens: 1,
        output_tokens: 1,
        model: "mock-model".into(),
        latency_ms: 1,
    }
}

fn tc(id: &str, name: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: args.to_string(),
    }
}

#[allow(clippy::type_complexity)]
async fn setup_deps() -> (AgentDeps, uuid::Uuid, tempfile::TempDir, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    let dir = tempfile::tempdir().unwrap();
    let registry = engram_llm::ProviderRegistry::new(
        pool.clone(),
        engram_llm::KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
    );
    let lib: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM wiki_libraries WHERE slug = 'main'")
            .fetch_one(&pool)
            .await
            .unwrap();
    (
        AgentDeps {
            pool,
            registry,
            data_dir: dir.path().to_path_buf(),
        },
        lib,
        dir,
        container,
    )
}

// ---------- 测试 ----------

#[tokio::test]
async fn harness_two_round_tool_loop_writes_page() {
    let (deps, lib, _dir, _pg) = setup_deps().await;
    let provider = Arc::new(MockProvider::new(vec![resp(
        "先查再写",
        vec![tc(
            "c1",
            "write_page",
            serde_json::json!({
                "slug": "agent-test-page",
                "title": "Agent 测试页",
                "content": "# Agent 测试页\n\n正文。参见 [[other]]"
            }),
        )],
    )]));

    let task = AgentTask {
        lib,
        instruction: "建一页".into(),
        source_url: None,
        source_text: Some("原料文本".into()),
        source_name: Some("原料".into()),
    };
    let report = run_agent(
        provider,
        "mock-model".into(),
        &deps,
        &task,
        AgentBudget::default(),
        None,
    )
    .await
    .expect("harness 应成功");


    assert_eq!(report.rounds, 2, "第一轮工具+第二轮 final");
    assert_eq!(report.tool_calls, 1);
    assert_eq!(report.pages_touched, vec!["agent-test-page"]);
    assert!(!report.degraded);
    assert!(report.summary.contains("final"));
    // 页面真的存在
    let exists: i64 = sqlx::query_scalar("SELECT count(*) FROM wiki_pages WHERE slug = 'agent-test-page' AND library_id = $1")
        .bind(lib)
        .fetch_one(&deps.pool)
        .await
        .unwrap();
    assert_eq!(exists, 1, "write_page 工具应真的建页");
}

/// 简易 tracing 捕获层（断言 audit 事件字段）。
struct CollectLayer(Arc<Mutex<Vec<serde_json::Value>>>);

impl<S> tracing_subscriber::Layer<S> for CollectLayer
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut map = serde_json::Map::new();
        let mut visitor = Collector(&mut map);
        event.record(&mut visitor);
        map.insert(
            "level".into(),
            event.metadata().level().to_string().into(),
        );
        self.0.lock().unwrap().push(serde_json::Value::Object(map));
    }
}

struct Collector<'a>(&'a mut serde_json::Map<String, serde_json::Value>);

impl tracing::field::Visit for Collector<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}").into());
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0
            .insert(field.name().to_string(), value.to_string().into());
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0.insert(field.name().to_string(), value.into());
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0.insert(field.name().to_string(), value.into());
    }
}

#[tokio::test]
async fn harness_destructive_tool_is_audited() {
    let (deps, lib, _dir, _pg) = setup_deps().await;
    // 先建一页供删除
    deps.wiki()
        .put_page(lib, "doomed", "待删页", "内容", None, None)
        .await
        .expect("建页");

    let events: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let layer = CollectLayer(events.clone());
    let _guard = tracing::subscriber::set_default(
        tracing_subscriber::registry().with(layer),
    );

    let provider = Arc::new(MockProvider::new(vec![resp(
        "删它",
        vec![tc("c1", "delete_page", serde_json::json!({"slug": "doomed"}))],
    )]));
    let task = AgentTask {
        lib,
        instruction: "删除 doomed 页".into(),
        source_url: None,
        source_text: None,
        source_name: None,
    };
    let report = run_agent(provider, "mock".into(), &deps, &task, AgentBudget::default(), None)
        .await
        .expect("harness 应成功");
    assert_eq!(report.pages_touched, Vec::<String>::new(), "删除后触页清空");

    let evs = events.lock().unwrap();
    let audit: Vec<&serde_json::Value> = evs
        .iter()
        .filter(|e| e.get("audit").and_then(|v| v.as_bool()) == Some(true))
        .collect();
    assert_eq!(audit.len(), 1, "破坏性工具应有且仅有一条审计: {evs:?}");
    let a = audit[0];
    assert_eq!(a["tool"], "delete_page");
    assert_eq!(a["destructive"], serde_json::json!(true));
    assert_eq!(a["ok"], serde_json::json!(true));
}

#[tokio::test]
async fn harness_budget_exhaustion_degrades_not_errors() {
    let (deps, lib, _dir, _pg) = setup_deps().await;
    // 永远要求调工具（耗尽后 mock 回 final 也算非 degraded 的兜底——所以用 budget=1 截断第一轮）
    let provider = Arc::new(MockProvider::new(vec![resp(
        "还要调",
        vec![tc("c1", "wiki_search", serde_json::json!({"query": "x"}))],
    )]));
    let task = AgentTask {
        lib,
        instruction: "无限循环任务".into(),
        source_url: None,
        source_text: None,
        source_name: None,
    };
    let report = run_agent(
        provider,
        "mock".into(),
        &deps,
        &task,
        AgentBudget {
            max_rounds: 1,
            max_llm_calls: 60,
            timeout: std::time::Duration::from_secs(60),
        },
        None,
    )
    .await
    .expect("预算耗尽应降级返回而非 Err");
    assert!(report.degraded, "达 max_rounds 应标记降级");
    assert!(report.degraded_reason.is_some());
}

#[tokio::test]
async fn harness_tool_error_feeds_back_not_aborts() {
    let (deps, lib, _dir, _pg) = setup_deps().await;
    // 第一轮调不存在的工具（报错回填），第二轮改调 get_page 读不存在的页（报错回填），第三轮 final
    let provider = Arc::new(MockProvider::new(vec![
        resp("试错1", vec![tc("c1", "no_such_tool", serde_json::json!({}))]),
        resp("试错2", vec![tc("c2", "get_page", serde_json::json!({"slug": "missing-page"}))]),
    ]));
    let task = AgentTask {
        lib,
        instruction: "容错路径".into(),
        source_url: None,
        source_text: None,
        source_name: None,
    };
    let report = run_agent(provider, "mock".into(), &deps, &task, AgentBudget::default(), None)
        .await
        .expect("工具失败不应终止循环");
    assert_eq!(report.rounds, 3);
    assert_eq!(report.tool_calls, 2);
    assert!(!report.degraded);
    let _ = ChatMessage::system("引用防未用告警");
}

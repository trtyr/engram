//! wiki Agent Harness 集成测试（P004-T010）：mock LLM 多轮工具循环 + 破坏性工具审计 + 预算降级。
//! 真实 demo（demo_real_url）已于 2026-10-02 跑通：LangChain 链接→web-reader 抓取→缓存原件
//! →LLM 分段读→建 3 页互链 6 条（118s，MiniMax-M3 经 newapi）。

mod support;

use tracing_subscriber::prelude::*;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use engram_core::wiki_agent::{AgentBudget, AgentDeps, AgentTask, run_agent};
use engram_llm::provider::LlmProvider;
use engram_llm::types::{
    ChatMessage, ChatRequest, ChatResponse, EmbedRequest, EmbedResponse, LlmError, ToolCall,
};

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
    async fn embed(&self, _req: EmbedRequest) -> Result<EmbedResponse, LlmError> {
        Err(LlmError::Permanent("mock 无嵌入".into()))
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
    let lib: uuid::Uuid = sqlx::query_scalar("SELECT id FROM wiki_libraries WHERE slug = 'main'")
        .fetch_one(&pool)
        .await
        .unwrap();
    (
        AgentDeps {
            purpose_text: None,
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
    let exists: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM wiki_pages WHERE slug = 'agent-test-page' AND library_id = $1",
    )
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
        map.insert("level".into(), event.metadata().level().to_string().into());
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
    // 进程级订阅（tokio 捕获纪律：set_default per-thread 在 await 竞态下丢事件）
    let _ = tracing::subscriber::set_global_default(
        tracing_subscriber::registry().with(CollectLayer(events.clone())),
    );

    let provider = Arc::new(MockProvider::new(vec![resp(
        "删它",
        vec![tc(
            "c1",
            "delete_page",
            serde_json::json!({"slug": "doomed"}),
        )],
    )]));
    let task = AgentTask {
        lib,
        instruction: "删除 doomed 页".into(),
        source_url: None,
        source_text: None,
        source_name: None,
    };
    let report = run_agent(
        provider,
        "mock".into(),
        &deps,
        &task,
        AgentBudget::default(),
        None,
    )
    .await
    .expect("harness 应成功");
    assert_eq!(report.pages_touched, Vec::<String>::new(), "删除后触页清空");

    let evs = events.lock().unwrap();
    let audit: Vec<&serde_json::Value> = evs
        .iter()
        .filter(|e| e.get("audit").and_then(|v| v.as_bool()) == Some(true))
        .collect();
    let a = audit.iter().find(|e| e["tool"] == "delete_page").expect(
        "delete_page 审计事件应存在（进程级订阅下混入他测事件只影响计数不影响存在性）: {evs:?}",
    );
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
        resp(
            "试错1",
            vec![tc("c1", "no_such_tool", serde_json::json!({}))],
        ),
        resp(
            "试错2",
            vec![tc(
                "c2",
                "get_page",
                serde_json::json!({"slug": "missing-page"}),
            )],
        ),
    ]));
    let task = AgentTask {
        lib,
        instruction: "容错路径".into(),
        source_url: None,
        source_text: None,
        source_name: None,
    };
    let report = run_agent(
        provider,
        "mock".into(),
        &deps,
        &task,
        AgentBudget::default(),
        None,
    )
    .await
    .expect("工具失败不应终止循环");
    assert_eq!(report.rounds, 3);
    assert_eq!(report.tool_calls, 2);
    assert!(!report.degraded);
    let _ = ChatMessage::system("引用防未用告警");
}

// ---------- 真实 demo（#[ignore]：ENGRAM_DEMO_LLM_KEY + ENGRAM_DEMO_READER_KEY 就绪时跑） ----------

/// 真实链接 → harness 自主抓取 → 建页 → 互链（newapi LLM + 智谱 web-reader）。
#[tokio::test]
#[ignore = "真实网络 demo：ENGRAM_DEMO_LLM_KEY/ENGRAM_DEMO_READER_KEY 环境变量就绪时运行"]
async fn demo_real_url_ingest_builds_pages() {
    let llm_key = std::env::var("ENGRAM_DEMO_LLM_KEY").expect("需要 ENGRAM_DEMO_LLM_KEY");
    let reader_key = std::env::var("ENGRAM_DEMO_READER_KEY").expect("需要 ENGRAM_DEMO_READER_KEY");
    unsafe { std::env::set_var("AGENT_MEMORY_MASTER_KEY", "ab".repeat(32)) };
    // 让 LLM 失败 warn（含响应 body）可见
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let (deps, lib, _dir, _pg) = setup_deps().await;

    // 1. chat provider：newapi 网关（is_default → WikiAgent resolve 回退默认 chat）
    sqlx::query(
        "INSERT INTO llm_providers (id, name, base_url, api_key_encrypted, model_id, capability, is_default) \
         VALUES ($1, 'newapi-demo', $4, $2, $3, 'chat', true)",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(
        engram_llm::KeyCipher::from_hex_master(&"ab".repeat(32))
            .unwrap()
            .encrypt(&llm_key)
            .unwrap(),
    )
    .bind(std::env::var("ENGRAM_DEMO_LLM_MODEL").unwrap_or_else(|_| "MiniMax-M3".into()))
    .bind(std::env::var("ENGRAM_DEMO_BASE_URL").unwrap_or_else(|_| "https://newapi.trtyr.top".into()))
    .execute(&deps.pool)
    .await
    .expect("provider 配置");

    // 2. web-reader 凭据（harness web_reader 工具读取）
    let creds = engram_core::credentials::CredentialsService::new(
        deps.pool.clone(),
        engram_llm::KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
    );
    creds
        .put(
            "zhipu/web_reader_key",
            &reader_key,
            None,
            "manual",
            &[],
            None,
            None,
            None,
        )
        .await
        .expect("凭据写入");

    // 3. 起 runner（wiki_docs 管道 + agent harness）
    let handle = {
        let pool = deps.pool.clone();
        let registry = deps.registry.clone();
        let dir = deps.data_dir.clone();
        let runner = engram_core::wiki_docs::register_handlers(
            engram_jobs::Runner::new(
                pool.clone(),
                engram_jobs::RunnerConfig {
                    worker_id: "demo".into(),
                    concurrency: 2,
                    poll_interval: std::time::Duration::from_millis(50),
                    batch_size: 10,
                    reap_interval: std::time::Duration::from_secs(3600),
                    per_kind_concurrency: Default::default(),
                    cipher: None,
                },
            ),
            registry.clone(),
        );
        engram_core::wiki_agent::register_agent_handler(runner, registry, dir).start()
    };

    // 4. 提交真实任务：真实链接 → 自主抓取建页
    let queue = engram_jobs::JobQueue::new(deps.pool.clone());
    let task = AgentTask {
        lib,
        instruction:
            "阅读这篇 RAG 文字切分教程，为 wiki 沉淀 2-3 页知识页（概念页 + 与库内已有页互链；无已有页则至少两页互链）。"
                .into(),
        source_url: Some(
            "https://python.langchain.com/docs/concepts/text_splitters/".into(),
        ),
        source_text: None,
        source_name: Some("LangChain Text Splitters".into()),
    };
    let job_id = engram_core::wiki_agent::enqueue_agent_task(&queue, &task)
        .await
        .expect("入队");

    // 5. 轮询 job 终态（demo 上限 8 分钟）
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(480);
    let mut final_status = String::new();
    let mut report = serde_json::Value::Null;
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(3000)).await;
        let (status, err): (String, Option<String>) =
            sqlx::query_as("SELECT status, error FROM jobs WHERE id = $1")
                .bind(job_id)
                .fetch_one(&deps.pool)
                .await
                .unwrap();
        if matches!(status.as_str(), "succeeded" | "failed" | "dead") {
            final_status = status;
            report = serde_json::json!({"error": err});
            break;
        }
    }
    eprintln!("DEMO job {job_id} 终态 = {final_status}\nDEMO report = {report}");
    let _ = handle
        .shutdown_and_wait(std::time::Duration::from_secs(10))
        .await;

    assert_eq!(final_status, "succeeded", "harness job 应成功: {report}");
    let page_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM wiki_pages WHERE library_id = $1")
            .bind(lib)
            .fetch_one(&deps.pool)
            .await
            .unwrap();
    let link_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM wiki_links WHERE library_id = $1")
            .bind(lib)
            .fetch_one(&deps.pool)
            .await
            .unwrap();
    eprintln!("DEMO pages = {page_count}, links = {link_count}");
    assert!(page_count >= 2, "至少建 2 页");
    assert!(link_count >= 1, "至少 1 条互链");
}

/// 终极对照：同进程 reqwest Client 发「抓包文件字节」到真网关（诊断 reqwest 500 元凶）。
#[tokio::test]
#[ignore = "诊断探针：ENGRAM_DEMO_LLM_KEY + /tmp/req_dump.json 存在时运行"]
async fn probe_reqwest_raw_dump_body() {
    let key = std::env::var("ENGRAM_DEMO_LLM_KEY").unwrap();
    let body = std::fs::read("/tmp/req_dump.json").unwrap();
    let client = reqwest::Client::builder()
        .http1_only()
        .user_agent(concat!("engram-llm/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap();
    // 1. 纯 reqwest + dump body
    let r1 = client
        .post("https://newapi.trtyr.top/v1/chat/completions")
        .bearer_auth(&key)
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .unwrap();
    eprintln!(
        "PROBE reqwest+dump-body: HTTP {} len={}",
        r1.status(),
        body.len()
    );
    if r1.status().as_u16() != 200 {
        eprintln!(
            "PROBE body-snippet: {}",
            r1.text()
                .await
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect::<String>()
        );
    }
    // 2. reqwest 手动 json! 构造（模拟 chat.rs 的 json! 宏路径）
    let val: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let r2 = client
        .post("https://newapi.trtyr.top/v1/chat/completions")
        .bearer_auth(&key)
        .json(&val)
        .send()
        .await
        .unwrap();
    eprintln!("PROBE reqwest+.json(Value): HTTP {}", r2.status());
    if r2.status().as_u16() != 200 {
        eprintln!(
            "PROBE body-snippet: {}",
            r2.text()
                .await
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect::<String>()
        );
    }
}

#[tokio::test]
async fn harness_study_coordination_updates_item() {
    // P007-T010 学习协同：harness 经 study_list/study_update_item 工具更新学习路线图状态
    let (deps, lib, _dir, _pg) = setup_deps().await;

    // 先建「RAG 入门」track + 一个知识点
    let svc = engram_core::study::StudyService::new(deps.pool.clone());
    let topic = svc
        .topic_create("RAG 入门", "能设计切分管线")
        .await
        .unwrap();
    let item = svc.item_add(topic, "基础流程", Some(10)).await.unwrap();

    // 两轮：① study_list 定位 ② study_update_item 标 learned
    let mock = Arc::new(MockProvider::new(vec![
        resp(
            "查学习路线图",
            vec![tc("c1", "study_list", serde_json::json!({}))],
        ),
        resp(
            "标记已学",
            vec![tc(
                "c2",
                "study_update_item",
                serde_json::json!({"item_id": item.to_string(), "status": "learned"}),
            )],
        ),
    ]));
    let report = run_agent(
        mock.clone(),
        "mock-model".into(),
        &deps,
        &AgentTask {
            lib,
            instruction: "把「基础流程」知识点标为已学，并挂 wiki 页 rag-basic-pipeline".into(),
            source_url: None,
            source_text: None,
            source_name: None,
        },
        AgentBudget::default(),
        None,
    )
    .await
    .expect("run_agent");

    assert_eq!(report.tool_calls, 2, "两次工具调用");
    assert!(report.pages_touched.is_empty());

    // DB 断言：状态真的变了 + learned_at 落了
    let full = svc.topic_get(topic).await.unwrap().unwrap();
    let it = full.items.iter().find(|i| i.id == item).unwrap();
    assert_eq!(it.status, "learned", "study_update_item 应生效");
    assert!(it.learned_at.is_some());
}

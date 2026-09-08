//! Wiki 域集成测试：两步 ingest 全链路（mock LLM）。
//! 多库（0037）：所有 svc 调用带 lib 参数——用 resolve(pool, None) 取 main 主库 id。

mod support;

use engram_distill::llm_port::MockLlm;
use engram_jobs::{Runner, RunnerConfig};
use engram_wiki_engine::{LlmRef, WikiService};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

async fn setup(
    chats: Vec<serde_json::Value>,
) -> (
    sqlx::PgPool,
    WikiService,
    engram_jobs::RunnerHandle,
    support::TestPg,
    uuid::Uuid, // lib：main 主库 id
) {
    let llm: LlmRef = Arc::new(MockLlm::with_raw_chats(
        chats
            .into_iter()
            .map(|c| match c {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            })
            .collect(),
    ));
    setup_llm(llm).await
}

/// W3：可注入定制 MockLlm（如 embed_fail=true）的 setup。
#[allow(clippy::type_complexity)]
async fn setup_llm(
    llm: LlmRef,
) -> (
    sqlx::PgPool,
    WikiService,
    engram_jobs::RunnerHandle,
    support::TestPg,
    uuid::Uuid, // lib：main 主库 id
) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    let lib = engram_wiki_engine::libraries::resolve(&pool, None)
        .await
        .expect("main 主库应存在");

    let l1 = llm.clone();
    let runner = engram_wiki_engine::ingest::register_handlers(
        Runner::new(
            pool.clone(),
            RunnerConfig {
                worker_id: "test".into(),
                concurrency: 4,
                poll_interval: Duration::from_millis(20),
                batch_size: 10,
                reap_interval: Duration::from_secs(3600),
            },
        ),
        l1,
    );
    // embed 也需要 handler 之外的 Llm —— WikiService 不直接调 LLM（嵌入在 job 内）
    let handle = runner.start();
    let registry = engram_llm::ProviderRegistry::new(
        pool.clone(),
        engram_llm::KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
    );
    (
        pool.clone(),
        WikiService::new(pool, registry),
        handle,
        container,
        lib,
    )
}

async fn wait_jobs(pool: &sqlx::PgPool, kinds: &[&str]) {
    for _ in 0..300 {
        let jobs: Vec<(String, String)> =
            sqlx::query_as("SELECT kind, status::text FROM jobs WHERE kind = ANY($1)")
                .bind(kinds)
                .fetch_all(pool)
                .await
                .unwrap();
        let all_done = jobs.iter().all(|(k, s)| {
            matches!(s.as_str(), "succeeded" | "failed" | "dead") || {
                let _ = k;
                false
            }
        });
        if all_done && !jobs.is_empty() {
            // 还要求至少每个 kind 一个
            if kinds.iter().all(|k| jobs.iter().any(|(jk, _)| jk == k)) {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("wiki jobs 超时: {kinds:?}");
}

/// 核心链路：两篇相关文档先后 ingest → 互链 + 第二篇更新既有页不重复建。
#[tokio::test]
async fn two_docs_interlinked_no_duplicate() {
    let (pool, wiki, handle, _pg, lib) = setup(vec![
        // 文档1 分析
        json!({"entities": ["张三"], "concepts": ["向量检索"], "links": [], "conflicts": [], "source_title": "文档一"}),
        // 文档1 生成：新建 3 页 + synthesis（跨源综合）+ comparison（对比）
        json!({"pages": [
            {"slug": "张三", "page_type": "entity", "title": "张三", "content": "# 张三\n\n张三是向量检索研究者，参见 [[向量检索]]。"},
            {"slug": "向量检索", "page_type": "concept", "title": "向量检索", "content": "# 向量检索\n\n向量检索是 [[张三]] 的研究方向。"},
            {"slug": "文档一", "page_type": "source", "title": "文档一", "content": "# 文档一\n\n来源摘要，涉及 [[张三]] 与 [[向量检索]]。"},
            {"slug": "检索方法综合", "page_type": "synthesis", "title": "检索方法综合", "content": "# 检索方法综合\n\n综合 [[向量检索]] 的多源观点：张三的量化方法与其他流派在精度/速度上各有取舍。"},
            {"slug": "量化方法对比", "page_type": "comparison", "title": "量化方法对比", "content": "# 量化方法对比\n\n[[张三]] 的量化方法 vs 传统方法：精度相当但速度快 3 倍。"},
        ]}),
        // 文档2 分析：发现与既有页关联
        json!({"entities": [], "concepts": ["近似搜索"], "links": [{"slug": "向量检索", "reason": "近似搜索是向量检索的子方向"}], "conflicts": [], "source_title": "文档二"}),
        // 文档2 生成：新建 concept 近似搜索 + source；更新既有「向量检索」（不重建）
        json!({"pages": [
            {"slug": "近似搜索", "page_type": "concept", "title": "近似搜索", "content": "# 近似搜索\n\n近似搜索属于 [[向量检索]] 的加速技术。"},
            {"slug": "向量检索", "page_type": "concept", "title": "向量检索", "content": "# 向量检索\n\n向量检索是 [[张三]] 的研究方向，其加速依赖 [[近似搜索]]。"},
            {"slug": "文档二", "page_type": "source", "title": "文档二", "content": "# 文档二\n\n扩展了 [[向量检索]]，引入 [[近似搜索]]。"},
        ]}),
    ])
    .await;

    // 文档 1
    let skipped = wiki
        .ingest(
            lib,
            "文档一",
            "张三研究向量检索。向量检索是一种在高维空间寻找近邻的技术。",
        )
        .await
        .unwrap();
    assert!(matches!(
        skipped,
        engram_wiki_engine::ingest::IngestOutcome::Enqueued(_, _)
    ));
    wait_jobs(&pool, &["wiki_analyze", "wiki_generate"]).await;

    let pages = wiki.list_pages(lib, None, 50, None).await.unwrap();
    // 3 内容页 + synthesis + comparison + index
    assert!(
        pages.len() >= 6,
        "应含 synthesis/comparison: {}",
        pages.len()
    );
    let synth = wiki.get_page(lib, "检索方法综合").await.unwrap();
    assert_eq!(synth.page_type, "synthesis", "synthesis 页型应真实产出");
    assert!(synth.content.contains("[[向量检索]]"), "综合页应互链");
    let comp = wiki.get_page(lib, "量化方法对比").await.unwrap();
    assert_eq!(comp.page_type, "comparison", "comparison 页型应真实产出");
    let vec_page = wiki.get_page(lib, "向量检索").await.unwrap();
    assert_eq!(vec_page.version, 1);

    // 文档 2（相关内容）
    let skipped2 = wiki
        .ingest(lib, "文档二", "近似搜索是向量检索的加速子方向，如 HNSW。")
        .await
        .unwrap();
    assert!(matches!(
        skipped2,
        engram_wiki_engine::ingest::IngestOutcome::Enqueued(_, _)
    ));
    wait_jobs(&pool, &["wiki_analyze", "wiki_generate"]).await;

    // 不重复建页：向量检索 v2（更新）而非新 slug；张三仍 1 版
    let vec_page = wiki.get_page(lib, "向量检索").await.unwrap();
    assert_eq!(vec_page.version, 2, "第二次 ingest 应更新既有页（版本 2）");
    assert!(
        vec_page.content.contains("近似搜索"),
        "更新内容应并入: {}",
        vec_page.content
    );
    let zhang = wiki.get_page(lib, "张三").await.unwrap();
    assert_eq!(zhang.version, 1, "未涉及的页不动");

    // 互链：图上有边
    let graph = wiki.graph(lib).await.unwrap();
    assert!(
        graph
            .edges
            .iter()
            .any(|e| e.from_slug == "张三" && e.to_slug == "向量检索")
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|e| e.from_slug == "向量检索" && e.to_slug == "张三")
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|e| e.from_slug == "近似搜索" && e.to_slug == "向量检索")
    );

    // sha 幂等：同内容再次 ingest 秒跳过
    let skipped3 = wiki
        .ingest(
            lib,
            "文档一",
            "张三研究向量检索。向量检索是一种在高维空间寻找近邻的技术。",
        )
        .await
        .unwrap();
    assert!(
        matches!(
            skipped3,
            engram_wiki_engine::ingest::IngestOutcome::AlreadyReady(_)
        ),
        "同 sha 已就绪应跳过"
    );

    // queries 存档闭环：archive_query → wiki_analyze 入队（自动再摄取产页）
    let skipped_q = wiki
        .archive_query(
            lib,
            "向量检索问答",
            "什么是向量检索？",
            "向量检索是在高维空间寻找最近邻的技术。",
        )
        .await
        .unwrap();
    assert!(!skipped_q, "queries 存档应触发摄取");
    // queries 页型直接落库（不依赖 LLM 生成）
    let qpage = wiki.get_page(lib, "query-向量检索问答").await.unwrap();
    assert_eq!(qpage.page_type, "queries", "存档应产 queries 页型");
    assert!(qpage.content.contains("**问**"), "queries 页应含问答结构");
    wait_jobs(&pool, &["wiki_analyze"]).await;

    handle.shutdown();
    handle.join().await;
}

/// 人写页面不被 LLM 覆盖 → proposal。
#[tokio::test]
async fn human_page_produces_proposal_not_overwrite() {
    let (pool, wiki, handle, _pg, lib) = setup(vec![
        // 分析
        json!({"entities": ["张三"], "concepts": [], "links": [], "conflicts": [], "source_title": "文档"}),
        // 生成：试图写「张三」页（人写页）→ 提案
        json!({"pages": [
            {"slug": "张三", "page_type": "entity", "title": "张三", "content": "# 张三\n\nLLM 版内容"},
        ]}),
    ])
    .await;

    // 人先写页面
    wiki.put_page(
        lib,
        "张三",
        "张三",
        "# 张三\n\n人工编写的内容。",
        None,
        None,
    )
    .await
    .unwrap();
    let before = wiki.get_page(lib, "张三").await.unwrap();
    assert_eq!(before.origin, "human");
    assert_eq!(before.version, 1);

    wiki.ingest(lib, "文档", "张三的信息。").await.unwrap();
    wait_jobs(&pool, &["wiki_analyze", "wiki_generate"]).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let after = wiki.get_page(lib, "张三").await.unwrap();
    assert_eq!(after.version, 1, "人写页不被覆盖");
    assert!(after.content.contains("人工编写"), "内容保持人工版");

    // proposal 事件存在
    let events: Vec<String> =
        sqlx::query_scalar("SELECT message FROM job_events WHERE message LIKE '%提案%'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(!events.is_empty(), "应产生提案事件");
    let data: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT data FROM job_events WHERE message LIKE '%提案%' LIMIT 1")
            .fetch_optional(&pool)
            .await
            .unwrap()
            .flatten();
    let data = data.unwrap();
    assert_eq!(data["page_slug"], "张三");
    assert!(
        data["proposal_content"]
            .as_str()
            .unwrap()
            .contains("LLM 版内容")
    );

    // 人审合入
    let merged = wiki
        .apply_proposal(
            lib,
            "张三",
            data["proposal_content"].as_str().unwrap(),
            "张三",
            None,
        )
        .await
        .unwrap();
    assert_eq!(merged.version, 2);
    assert!(merged.content.contains("LLM 版内容"));

    handle.shutdown();
    handle.join().await;
}

/// 文档织入双路（2026-09-04）：upload 文档（raw_path）走原文件；URL 文档
/// （raw_path=NULL）用 chunks 拼接兜底——此前 URL 文档 --doc-id 会 404 且自动织入静默跳过。
#[tokio::test]
async fn ingest_document_url_fallback_uses_chunks() {
    let (pool, wiki, handle, _pg, lib) = setup(vec![]).await;

    // URL 式文档：raw_path NULL + 已分块文本（挂 main 库——wiki_documents/chunks 均带 library_id）
    let doc_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO wiki_documents (id, library_id, title, source_uri, status) \
         VALUES ($1, $2, 'URL 摄取的文档', 'https://example.com/article', 'ready')",
    )
    .bind(doc_id)
    .bind(lib)
    .execute(&pool)
    .await
    .unwrap();
    for (seq, content) in [
        (1, "第一段：异步运行时的选型考量。"),
        (2, "第二段：tokio 与 async-std 的取舍。"),
    ] {
        sqlx::query(
            "INSERT INTO wiki_chunks (id, library_id, document_id, seq, content) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(uuid::Uuid::now_v7())
        .bind(lib)
        .bind(doc_id)
        .bind(seq)
        .bind(content)
        .execute(&pool)
        .await
        .unwrap();
    }

    let skipped = wiki.ingest_document(lib, doc_id).await.unwrap();
    assert!(
        matches!(
            skipped,
            engram_wiki_engine::ingest::IngestOutcome::Enqueued(_, _)
        ),
        "首次织入不应跳过"
    );
    // wiki_sources 已入队（sha 按 title+拼接文本计算，两段都进文本）
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM wiki_sources")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "URL 文档应织入 wiki_sources");
    // 幂等（pending 语义）：无 LLM 跑完 analysis 前，源停在 pending——重复织入
    // 不新建源（sha 去重，重置重跑），sources 仍 1 条；skipped=true 要等源 ready 才成立
    let _ = wiki.ingest_document(lib, doc_id).await.unwrap();
    let n2: i64 = sqlx::query_scalar("SELECT count(*) FROM wiki_sources")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n2, 1, "重复织入不得新建源（sha 去重）");

    // 无 raw_path 且无 chunks → 可行动 400
    let empty_doc = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO wiki_documents (id, library_id, title, source_uri, status) VALUES ($1, $2, '空文档', 'https://x', 'pending')",
    )
    .bind(empty_doc)
    .bind(lib)
    .execute(&pool)
    .await
    .unwrap();
    let err = wiki.ingest_document(lib, empty_doc).await;
    assert!(
        matches!(&err, Err(engram_wiki_engine::WikiError::BadRequest(m)) if m.contains("无可织入的分块")),
        "无文件无分块应 400 且文案可行动：{err:?}"
    );

    // 不存在 → 404
    let ghost = wiki.ingest_document(lib, uuid::Uuid::now_v7()).await;
    assert!(matches!(
        &ghost,
        Err(engram_wiki_engine::WikiError::NotFound(_))
    ));

    handle.shutdown();
    handle.join().await;
}

/// S-7：via 执行者标记落 frontmatter——AI 代执行（"ai"）与真人编辑可区分；
/// 后续不带 via 的更新不清除已有标记（merge 块为空对象时保持原值）。
#[tokio::test]
async fn put_page_via_lands_in_frontmatter() {
    let (_pool, wiki, handle, _pg, lib) = setup(vec![]).await;

    let p = wiki
        .put_page(lib, "via-page", "V", "# V 内容", None, Some("ai"))
        .await
        .unwrap();
    assert_eq!(
        p.frontmatter.get("via").and_then(|v| v.as_str()),
        Some("ai"),
        "新建带 via 应落 frontmatter.via"
    );

    let p2 = wiki
        .put_page(lib, "via-page", "V", "# V 内容 v2", None, None)
        .await
        .unwrap();
    assert_eq!(
        p2.frontmatter.get("via").and_then(|v| v.as_str()),
        Some("ai"),
        "无 via 的更新不得清除已有执行者标记"
    );

    handle.shutdown();
    handle.join().await;
}

/// lint：注入死链 + 孤儿页 → 全部报出。
#[tokio::test]
async fn lint_reports_dead_links_and_orphans() {
    let (pool, wiki, handle, _pg, lib) = setup(vec![]).await;

    // 正常互链两页 + 一个死链 + 一个孤儿
    wiki.put_page(lib, "正常页A", "A", "内容链接 [[正常页B]]。", None, None)
        .await
        .unwrap();
    wiki.put_page(lib, "正常页B", "B", "回链 [[正常页A]]。", None, None)
        .await
        .unwrap();
    wiki.put_page(
        lib,
        "带死链",
        "D",
        "这里有个 [[不存在的页面]]。",
        None,
        None,
    )
    .await
    .unwrap();
    wiki.put_page(lib, "孤儿页", "O", "没有任何入链。", None, None)
        .await
        .unwrap();
    // W-1：大小写变体链接（[[Engram]] vs slug=engram）——不判死链，报 case_mismatch
    wiki.put_page(lib, "engram", "Engram", "实体页。", None, None)
        .await
        .unwrap();
    wiki.put_page(lib, "引用页", "R", "产品是 [[Engram]]。", None, None)
        .await
        .unwrap();
    // 手动建链接表（put_page 现已自动重算本页 wikilinks；此处补齐测试所需的其他边，
    // ON CONFLICT 跳过与自动重算重叠的边；边挂库）
    sqlx::query("INSERT INTO wiki_links (library_id, from_slug, to_slug, weight) VALUES ($1,'正常页A','正常页B',3.0), ($1,'正常页B','正常页A',3.0), ($1,'带死链','不存在的页面',3.0) ON CONFLICT (library_id, from_slug, to_slug) DO NOTHING")
        .bind(lib)
        .execute(&pool)
        .await
        .unwrap();

    let report = wiki.lint(lib).await.unwrap();
    let dead: Vec<_> = report
        .issues
        .iter()
        .filter(|i| i.rule == "dead_link")
        .collect();
    let orphan: Vec<_> = report
        .issues
        .iter()
        .filter(|i| i.rule == "orphan")
        .collect();

    assert!(
        dead.iter().any(|i| i.slug == "带死链"),
        "死链应报出: {dead:?}"
    );
    // W-1：[[Engram]] 只差大小写——不得进 dead_link，应进 case_mismatch
    assert!(
        !dead.iter().any(|i| i.slug == "引用页"),
        "大小写变体不是死链: {dead:?}"
    );
    let case: Vec<_> = report
        .issues
        .iter()
        .filter(|i| i.rule == "case_mismatch")
        .collect();
    assert!(
        case.iter()
            .any(|i| i.slug == "引用页" && i.detail.contains("[[engram]]")),
        "大小写变体应报 case_mismatch 并给出正确 slug: {case:?}"
    );
    // 孤儿：带死链/孤儿页 无入链（正常页有互链）
    assert!(
        orphan.iter().any(|i| i.slug == "孤儿页"),
        "孤儿应报出: {orphan:?}"
    );
    assert!(
        orphan.iter().any(|i| i.slug == "带死链"),
        "无入链页也是孤儿: {orphan:?}"
    );
    // 正常页不误报
    assert!(!dead.iter().any(|i| i.slug == "正常页A"));
    assert!(!orphan.iter().any(|i| i.slug == "正常页A"));

    handle.shutdown();
    handle.join().await;
}

/// lint 第 5 步（过时源）：id 曾以 text 形式回查 wiki_sources.id（uuid 列），
/// 曾因 uuid = text 类型错误 503——回归：带已 ingest 无引用 source 时 lint 正常完成并报 stale_source。
#[tokio::test]
async fn lint_with_ingested_source_reports_stale_without_type_error() {
    let (pool, wiki, handle, _pg, lib) = setup(vec![]).await;

    let sid = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO wiki_sources (id, library_id, sha256, raw_path, title, status, last_ingested_at) \
         VALUES ($1, $2, 'lint-stale-sha', '/tmp/lint-stale.md', 'lint-stale', 'ready', now())",
    )
    .bind(sid)
    .bind(lib)
    .execute(&pool)
    .await
    .unwrap();

    // 该 source 无任何页面引用 → 走 cnt 回查分支（修复前此处 uuid = text 直接 503）
    let report = wiki.lint(lib).await.unwrap();
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.rule == "stale_source" && i.slug == sid.to_string()),
        "已 ingest 无引用的 source 应报 stale_source: {:?}",
        report.issues
    );

    handle.shutdown();
    handle.join().await;
}

/// review resolve 未命中（不存在或已处理）应按 NotFound 语义返回，
/// 修复前 JobError::Permanent 被 From 统一映射成 Storage → 接口层 503。
#[tokio::test]
async fn review_resolve_miss_returns_not_found() {
    let (pool, wiki, handle, _pg, _lib) = setup(vec![]).await;

    let miss = uuid::Uuid::now_v7();
    let err = wiki.review_resolve(miss, None, true).await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("不存在或已处理"),
        "未命中应报不存在或已处理: {msg}"
    );

    handle.shutdown();
    handle.join().await;
    let _ = pool;
}

// ---------- W1：generate 永久失败后重提交自愈（死锁解除） ----------

#[tokio::test]
async fn w1_generate_failure_resubmit_recovers() {
    let analysis = json!({"entities": ["W1实体"], "concepts": [], "links": [], "conflicts": [], "source_title": "W1文档"});
    let pages_ok = json!({"pages": [
        {"slug": "w1-page", "page_type": "concept", "title": "W1页", "content": "# W1页\n\nW1 内容词可检索。"}
    ]});
    let bad = serde_json::Value::String("{not json".into());
    // analyze ✓ → generate 两连坏 JSON 永久失败 → 重提交后新 generate ✓
    let (pool, wiki, handle, _pg, lib) = setup(vec![analysis, bad.clone(), bad, pages_ok]).await;

    let text = "# W1 死锁恢复测试\n这是独一无二的内容 w1-unique-123。";
    let skipped = wiki.ingest(lib, "W1文档", text).await.unwrap();
    assert!(matches!(
        skipped,
        engram_wiki_engine::ingest::IngestOutcome::Enqueued(_, _)
    ));
    wait_jobs(&pool, &["wiki_analyze", "wiki_generate"]).await;

    // 第一轮：generate 永久失败，source 未 ready
    let failed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE kind='wiki_generate' AND status='failed'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(failed, 1, "第一轮 generate 应 failed");
    let src_status: String = sqlx::query_scalar("SELECT status FROM wiki_sources")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(src_status, "ready");

    // 重提交同 sha → 状态感知：从失败 job payload 取 analysis 直发新 generate
    // （旧逻辑：幂等键墙返回终态 job，链断死锁）
    let skipped2 = wiki.ingest(lib, "W1文档", text).await.unwrap();
    assert!(
        matches!(
            skipped2,
            engram_wiki_engine::ingest::IngestOutcome::Enqueued(_, _)
        ),
        "恢复路径应真正重跑而非秒跳过"
    );

    for _ in 0..300 {
        let done: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM jobs WHERE kind='wiki_generate' AND status IN ('succeeded','failed','dead')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        if done >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let succ: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE kind='wiki_generate' AND status='succeeded'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(succ, 1, "第二轮 generate 应成功（死锁解除）");

    let ready: String = sqlx::query_scalar("SELECT status FROM wiki_sources")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ready, "ready");
    let page = wiki.get_page(lib, "w1-page").await.unwrap();
    assert!(page.content.contains("W1 内容词"));

    handle.shutdown();
    handle.join().await;
}

// ---------- W2/W3：内容词检索 + 嵌入失败不丢索引 ----------

/// W2：LLM 生成页可按内容词（非 slug 词）FTS 命中（旧实现 tsv 只嵌 slug）。
#[tokio::test]
async fn w2_content_word_search_hits_llm_pages() {
    let analysis = json!({"entities": [], "concepts": [], "links": [], "conflicts": [], "source_title": "W2文档"});
    let pages = json!({"pages": [
        {"slug": "w2-knowledge-graph", "page_type": "concept", "title": "知识图谱页",
         "content": "# 知识图谱页\n\n这里讨论分布式系统的一致性哈希与数据分片策略。"}
    ]});
    let (pool, wiki, handle, _pg, lib) = setup(vec![analysis, pages]).await;

    wiki.ingest(lib, "W2文档", "# W2 测试内容\n独一无二 w2-unique。")
        .await
        .unwrap();
    wait_jobs(&pool, &["wiki_analyze", "wiki_generate"]).await;

    // 内容词命中（slug 里完全没有这些词）
    let hits = wiki.search(lib, "一致性哈希 数据分片", 10).await.unwrap();
    assert!(!hits.is_empty(), "内容词应命中 LLM 生成页");
    assert_eq!(hits[0].slug, "w2-knowledge-graph");

    // 纯 FTS 语义也成立：tsv 非 NULL 且含内容 token
    let tsv_len: i32 = sqlx::query_scalar(
        "SELECT COALESCE(length(tsv::text), 0) FROM wiki_pages WHERE slug = 'w2-knowledge-graph'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(tsv_len > 0, "tsv 应已写入（旧实现只嵌 slug）");

    handle.shutdown();
    handle.join().await;
}

/// W3：嵌入整体失败——页面仍入库、tsv 仍写、内容词仍可检索（不再从检索消失）。
#[tokio::test]
async fn w3_embed_failure_keeps_fts_searchable() {
    // 真注入：MockLlm embed_fail=true（generate 内 llm.embed 直接 Err）
    let mut mock = MockLlm::with_raw_chats(vec![
        json!({"entities": [], "concepts": [], "links": [], "conflicts": [], "source_title": "W3文档"}).to_string(),
        json!({"pages": [
            {"slug": "w3-resilient", "page_type": "concept", "title": "韧性页",
             "content": "# 韧性页\n\n探讨故障恢复与降级策略的工程实践。"}
        ]}).to_string(),
    ]);
    mock.embed_fail = true;
    let (pool, wiki, handle, _pg, lib) = setup_llm(Arc::new(mock)).await;

    wiki.ingest(lib, "W3文档", "# W3 嵌入失败\nw3-unique-777。")
        .await
        .unwrap();
    wait_jobs(&pool, &["wiki_analyze", "wiki_generate"]).await;

    let src: String = sqlx::query_scalar("SELECT status FROM wiki_sources")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(src, "ready", "嵌入失败不阻塞 ready");

    // 向量缺失 + tsv 已写（W3 解耦的直接证据）
    let (no_vec, tsv_null): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE embedding IS NULL), count(*) FILTER (WHERE tsv IS NULL) \
         FROM wiki_pages WHERE slug = 'w3-resilient'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(no_vec, 1, "嵌入失败 → 向量缺失");
    assert_eq!(tsv_null, 0, "tsv 必须已写（W3 解耦）");

    let hits = wiki.search(lib, "故障恢复 降级", 10).await.unwrap();
    assert!(!hits.is_empty(), "嵌入失败后内容词仍可检索");

    // 失败事件留痕（可观测）
    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM job_events WHERE message LIKE '%嵌入失败%'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(events >= 1, "嵌入失败应有事件");

    handle.shutdown();
    handle.join().await;
}

// ---------- W4：Permanent 失败标 failed + error 落列（'failed' 不再是幽灵态） ----------

#[tokio::test]
async fn w4_permanent_failure_marks_source_failed() {
    let analysis = json!({"entities": [], "concepts": [], "links": [], "conflicts": [], "source_title": "W4文档"});
    let bad = serde_json::Value::String("{broken".into());
    let pages_ok = json!({"pages": [
        {"slug": "w4-page", "page_type": "concept", "title": "W4页", "content": "# W4\n\n恢复后的页面。"}
    ]});
    // analyze ✓ → generate 坏 JSON ×2 → Permanent → W4 标 failed → 重提交自愈 → ready
    let (pool, wiki, handle, _pg, lib) = setup(vec![analysis, bad.clone(), bad, pages_ok]).await;

    let text = "# W4 失败标记测试\nw4-unique-42。";
    wiki.ingest(lib, "W4文档", text).await.unwrap();
    wait_jobs(&pool, &["wiki_analyze", "wiki_generate"]).await;

    // W4 核心：generate Permanent 失败 → source='failed' + error 非空（旧实现永卡 processing）
    let (status, error): (String, Option<String>) =
        sqlx::query_as("SELECT status, error FROM wiki_sources")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "failed", "Permanent 失败应标 failed");
    assert!(
        error.as_deref().unwrap_or("").contains("解析") || error.is_some(),
        "error 应落列: {error:?}"
    );

    // 重提交同 sha → 自愈（W1 路径 + W4 状态重置）→ 最终 ready
    wiki.ingest(lib, "W4文档", text).await.unwrap();
    for _ in 0..300 {
        let (s, e): (String, Option<String>) =
            sqlx::query_as("SELECT status, error FROM wiki_sources")
                .fetch_one(&pool)
                .await
                .unwrap();
        if s == "ready" && e.is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let (s, e): (String, Option<String>) = sqlx::query_as("SELECT status, error FROM wiki_sources")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(s, "ready", "自愈后应 ready");
    assert!(e.is_none(), "自愈应清 error");

    handle.shutdown();
    handle.join().await;
}

// ---------- W6：UPSERT 原子化（human 保护 + 并发不撞 UNIQUE） ----------

#[tokio::test]
async fn w6_upsert_protects_human_and_concurrent_safe() {
    let analysis = json!({"entities": [], "concepts": [], "links": [], "conflicts": [], "source_title": "W6文档"});
    let pages = json!({"pages": [
        {"slug": "w6-page", "page_type": "concept", "title": "W6页", "content": "# W6页\n\nLLM 生成的原始内容。"}
    ]});
    let (pool, wiki, handle, _pg, lib) = setup(vec![analysis, pages]).await;
    wiki.ingest(lib, "W6文档", "# W6 首轮\nw6-unique-a")
        .await
        .unwrap();
    wait_jobs(&pool, &["wiki_analyze", "wiki_generate"]).await;

    // 人工接管该页
    let human = wiki
        .put_page(
            lib,
            "w6-page",
            "W6页",
            "# W6页\n\n人工内容，不许覆盖。",
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(human.origin, "human");
    let v_before = human.version;

    // 第二轮 generate 同 slug：human 保护 → 提案，内容/版本不动
    let pages2 = json!({"pages": [
        {"slug": "w6-page", "page_type": "concept", "title": "W6页", "content": "# W6页\n\nLLM 想改成自己的版本。"},
        {"slug": "w6-llm", "page_type": "concept", "title": "W6二号", "content": "# W6二号\n\n新页面内容。"}
    ]});
    // setup 只支持一次注入——直接向队列再手动入链（复用 svc 的 ingest 会耗尽 mock）。
    drop((wiki, handle)); // 保留 pool；runner 停掉避免抢跑
    let _ = pages2;

    // 直接构造 generate job 的 UPSERT 语义验证：并发两次同 slug UPSERT（模拟两个 generate 竞态）
    // 多库：冲突目标 (library_id, slug)
    let fm = serde_json::json!({"title": "W6页", "page_type": "concept", "sources": []});
    let upsert = || {
        sqlx::query_scalar::<_, bool>(
            "INSERT INTO wiki_pages (id, library_id, slug, title, page_type, content, frontmatter, origin, version) \
             VALUES ($1, $2, 'w6-race', 'W6页', 'concept', $3, $4::jsonb, 'llm', 1) \
             ON CONFLICT (library_id, slug) DO UPDATE SET content = $3, version = wiki_pages.version + 1, updated_at = now() \
             WHERE wiki_pages.origin = 'llm' \
             RETURNING (xmax = 0)",
        )
    };
    // 新 slug：两个并发 UPSERT —— 一个插入，一个合并，绝不 UNIQUE 报错
    let (a, b) = tokio::join!(
        async {
            upsert()
                .bind(uuid::Uuid::now_v7())
                .bind(lib)
                .bind("并发写入A")
                .bind(sqlx::types::Json(&fm))
                .fetch_optional(&pool)
                .await
                .unwrap()
        },
        async {
            upsert()
                .bind(uuid::Uuid::now_v7())
                .bind(lib)
                .bind("并发写入B")
                .bind(sqlx::types::Json(&fm))
                .fetch_optional(&pool)
                .await
                .unwrap()
        }
    );
    assert_eq!(
        (a.is_some(), b.is_some()),
        (true, true),
        "并发 UPSERT 都成功（一插一合）"
    );
    let (content, version): (String, i32) =
        sqlx::query_as("SELECT content, version FROM wiki_pages WHERE slug = 'w6-race'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(version == 1 || version == 2, "合并方 version+1: {version}");
    assert!(content == "并发写入A" || content == "并发写入B");

    // human 页：UPSERT 不生效（RETURNING None），内容与版本不动
    let blocked = sqlx::query_scalar::<_, bool>(
        "INSERT INTO wiki_pages (id, library_id, slug, title, page_type, content, frontmatter, origin, version) \
         VALUES ($1, $2, 'w6-page', 'W6页', 'concept', 'LLM 想覆盖', $3::jsonb, 'llm', 1) \
         ON CONFLICT (library_id, slug) DO UPDATE SET content = 'LLM 想覆盖', version = wiki_pages.version + 1 \
         WHERE wiki_pages.origin = 'llm' \
         RETURNING (xmax = 0)",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(lib)
    .bind(sqlx::types::Json(&fm))
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(blocked.is_none(), "human 页 UPSERT 应无行返回（保护生效）");
    let (content, version): (String, i32) =
        sqlx::query_as("SELECT content, version FROM wiki_pages WHERE slug = 'w6-page'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(content.contains("人工内容"), "human 内容未被动");
    assert_eq!(version, v_before, "human 页版本未动");
}

// ---------- 契约：graph 端点 sparse 旗标与 insights 同口径 ----------

#[tokio::test]
async fn graph_community_sparse_flag_matches_insights_threshold() {
    let env = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&env).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    let registry = engram_llm::ProviderRegistry::new(
        pool.clone(),
        engram_llm::KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
    );
    let svc = engram_wiki_engine::WikiService::new(pool.clone(), registry);
    let lib = engram_wiki_engine::libraries::resolve(&pool, None)
        .await
        .expect("main 主库应存在");

    // 3 页同一社区：仅靠下限权重边（无 wikilink、无共享 source）→ 应判稀疏（页面挂库）
    for slug in ["sp-a", "sp-b", "sp-c"] {
        sqlx::query(
            "INSERT INTO wiki_pages (id, library_id, slug, title, content, page_type, origin, frontmatter)
             VALUES ($3, $2, $1, $1, 'x', 'concept', 'llm', '{}')",
        )
        .bind(slug)
        .bind(lib)
        .bind(uuid::Uuid::now_v7())
        .execute(&pool)
        .await
        .unwrap();
    }
    // 手工放三条下限权重边（0.1）——Louvain 会聚成一个社区，cohesion = 0.3/3 = 0.1 < 0.15（边挂库）
    for (f, t) in [("sp-a", "sp-b"), ("sp-b", "sp-c"), ("sp-a", "sp-c")] {
        sqlx::query(
            "INSERT INTO wiki_links (library_id, from_slug, to_slug, weight) VALUES ($3, $1, $2, 0.1)",
        )
        .bind(f)
        .bind(t)
        .bind(lib)
        .execute(&pool)
        .await
        .unwrap();
    }

    let g = svc.graph(lib).await.unwrap();
    assert!(
        g.communities.iter().any(|c| c.sparse && c.size >= 3),
        "下限权重边社区应判 sparse（cohesion=0.1<0.15）：{:?}",
        g.communities
    );
    // 全部社区 size/top_slug/sparse 字段名实相符
    for c in &g.communities {
        assert_eq!(c.top_slug.is_empty(), c.size == 0, "top_slug 与 size 一致");
    }
    drop(env);
}

/// W-13（2026-09-04）：query-archive 重复存档同 title → 幂等 skipped，
/// 不再落页 version+1 + 再摄取烧 LLM。
#[tokio::test]
async fn archive_query_is_idempotent_by_title() {
    let (pool, wiki, handle, _pg, lib) = setup(vec![]).await;

    let first = wiki
        .archive_query(lib, "幂等存档", "问", "答")
        .await
        .unwrap();
    assert!(!first, "首次存档 skipped=false");
    let second = wiki
        .archive_query(lib, "幂等存档", "问", "答")
        .await
        .unwrap();
    assert!(second, "重复存档应 skipped=true");

    let (n, ver): (i64, i32) = sqlx::query_as(
        "SELECT count(*), COALESCE(max(version), 0) FROM wiki_pages WHERE slug = 'query-幂等存档'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1, "同 title 只落一条 queries 页");
    assert_eq!(ver, 1, "重复存档不应 version+1");

    handle.shutdown();
    handle.join().await;
}

/// W-7③（2026-09-04）：单次织入建页量软上限——generate 返回 21 页，截断到 20 + 告警事件。
#[tokio::test]
async fn generate_page_cap_truncates_and_warns() {
    let pages: Vec<serde_json::Value> = (0..21)
        .map(|i| {
            serde_json::json!({
                "slug": format!("页{i}"),
                "page_type": "concept",
                "title": format!("页{i}"),
                "content": format!("# 页{i}\n\n第 {i} 页内容。"),
            })
        })
        .collect();
    let (pool, wiki, handle, _pg, lib) = setup(vec![
        serde_json::json!({"entities": [], "concepts": [], "links": [], "conflicts": [], "source_title": "批量源"}),
        serde_json::json!({"pages": pages}),
    ])
    .await;

    let skipped = wiki
        .ingest(lib, "批量源", "一篇覆盖大量主题的文档。")
        .await
        .unwrap();
    assert!(matches!(
        skipped,
        engram_wiki_engine::ingest::IngestOutcome::Enqueued(_, _)
    ));
    wait_jobs(&pool, &["wiki_analyze", "wiki_generate"]).await;

    // 截断到 20（21 页 concept 只建 20）
    let cnt: i64 =
        sqlx::query_scalar("SELECT count(*) FROM wiki_pages WHERE page_type = 'concept'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(cnt, 20, "建页量应被软上限截断到 20");

    // 告警事件落 job_events
    let warned: i64 =
        sqlx::query_scalar("SELECT count(*) FROM job_events WHERE message LIKE '%建页量超软上限%'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(warned, 1, "应有告警事件标记截断");

    handle.shutdown();
    handle.join().await;
}

/// R9/D28：list_pages keyset 游标翻页——无重复、无丢失、垃圾游标响亮拒。
#[tokio::test]
async fn list_pages_cursor_pagination_walks_all() {
    let (pool, wiki, _runner, _pg, lib) = setup(vec![]).await;
    for i in 0..5 {
        wiki.put_page(
            lib,
            &format!("d28-页-{i}"),
            &format!("D28 页 {i}"),
            &format!("第 {i} 页"),
            None,
            None,
        )
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = wiki
            .list_pages(lib, None, 2, cursor.as_deref())
            .await
            .unwrap();
        assert!(page.len() <= 2);
        if page.is_empty() {
            break;
        }
        seen.extend(page.iter().map(|p| p.slug.clone()));
        let last = page.last().unwrap();
        cursor = Some(format!("{}|{}", last.updated_at.to_rfc3339(), last.id));
        if seen.len() > 5 {
            panic!("游标翻页越走越多");
        }
    }
    assert_eq!(seen.len(), 5, "应恰好走全 5 页：{seen:?}");
    assert_eq!(
        seen.iter().collect::<std::collections::HashSet<_>>().len(),
        5,
        "不得重复"
    );
    // 垃圾游标响亮拒
    let err = wiki
        .list_pages(lib, None, 2, Some("garbage"))
        .await
        .expect_err("垃圾游标应被拒");
    assert!(err.to_string().contains("cursor"), "{err}");
    let _ = pool;
}

/// R7/D23+D24：log 系统页不进 graph/lint 口径；ingest 空入参响亮拒绝。
#[tokio::test]
async fn log_page_excluded_from_graph_and_lint_and_empty_ingest_rejected() {
    let (pool, svc, _runner, _pg, lib) = setup(vec![]).await;
    svc.put_page(lib, "普通页", "普通页", "正文 [[普通页]] 自链", None, None)
        .await
        .unwrap();
    // 直插一条系统 log 页（list_pages 不可见；挂库）
    sqlx::query(
        "INSERT INTO wiki_pages (id, library_id, slug, title, page_type, folder, content, frontmatter, origin, version, tsv) \
         VALUES ($1, $2, 'log', '审计日志', 'log', '系统', '日志内容', '{}', 'llm', 1, '')",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(lib)
    .execute(&pool)
    .await
    .unwrap();

    // D23：list_pages / lint / graph 三口径一致（都不含 log）
    assert_eq!(svc.list_pages(lib, None, 100, None).await.unwrap().len(), 1);
    let lint = svc.lint(lib).await.unwrap();
    assert_eq!(lint.checked_pages, 1, "log 页不应计入 lint：{lint:?}");
    let graph = svc.graph(lib).await.unwrap();
    assert_eq!(graph.nodes.len(), 1, "log 页不应进图：{:?}", graph.nodes);

    // D24：空标题 / 空文本 / 纯空白，全部响亮拒绝不入队
    for (title, text) in [("", "正文"), ("标题", ""), ("  ", "  ")] {
        let err = svc
            .ingest(lib, title, text)
            .await
            .expect_err("空入参应被拒");
        assert!(
            err.to_string().contains("不能为空"),
            "({title:?}, {text:?}) 应报不能为空：{err}"
        );
    }
}

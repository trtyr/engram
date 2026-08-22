//! Wiki 域集成测试：两步 ingest 全链路（mock LLM）。

mod support;

use agent_memory_distill::llm_port::MockLlm;
use agent_memory_jobs::{Runner, RunnerConfig};
use agent_memory_wiki_engine::{LlmRef, WikiService};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

async fn setup(
    chats: Vec<serde_json::Value>,
) -> (
    sqlx::PgPool,
    WikiService,
    agent_memory_jobs::RunnerHandle,
    support::TestPg,
) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    agent_memory_storage::run_migrations(&pool)
        .await
        .expect("迁移");

    let llm: LlmRef = Arc::new(MockLlm::with_raw_chats(
        chats
            .into_iter()
            .map(|c| match c {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            })
            .collect(),
    ));
    let l1 = llm.clone();
    let runner = agent_memory_wiki_engine::ingest::register_handlers(
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
    (pool.clone(), WikiService::new(pool), handle, container)
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
    let (pool, wiki, handle, _pg) = setup(vec![
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
            "文档一",
            "张三研究向量检索。向量检索是一种在高维空间寻找近邻的技术。",
        )
        .await
        .unwrap();
    assert!(!skipped);
    wait_jobs(&pool, &["wiki_analyze", "wiki_generate"]).await;

    let pages = wiki.list_pages(None, 50).await.unwrap();
    // 3 内容页 + synthesis + comparison + index
    assert!(
        pages.len() >= 6,
        "应含 synthesis/comparison: {}",
        pages.len()
    );
    let synth = wiki.get_page("检索方法综合").await.unwrap();
    assert_eq!(synth.page_type, "synthesis", "synthesis 页型应真实产出");
    assert!(synth.content.contains("[[向量检索]]"), "综合页应互链");
    let comp = wiki.get_page("量化方法对比").await.unwrap();
    assert_eq!(comp.page_type, "comparison", "comparison 页型应真实产出");
    let vec_page = wiki.get_page("向量检索").await.unwrap();
    assert_eq!(vec_page.version, 1);

    // 文档 2（相关内容）
    let skipped2 = wiki
        .ingest("文档二", "近似搜索是向量检索的加速子方向，如 HNSW。")
        .await
        .unwrap();
    assert!(!skipped2);
    wait_jobs(&pool, &["wiki_analyze", "wiki_generate"]).await;

    // 不重复建页：向量检索 v2（更新）而非新 slug；张三仍 1 版
    let vec_page = wiki.get_page("向量检索").await.unwrap();
    assert_eq!(vec_page.version, 2, "第二次 ingest 应更新既有页（版本 2）");
    assert!(
        vec_page.content.contains("近似搜索"),
        "更新内容应并入: {}",
        vec_page.content
    );
    let zhang = wiki.get_page("张三").await.unwrap();
    assert_eq!(zhang.version, 1, "未涉及的页不动");

    // 互链：图上有边
    let graph = wiki.graph().await.unwrap();
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
            "文档一",
            "张三研究向量检索。向量检索是一种在高维空间寻找近邻的技术。",
        )
        .await
        .unwrap();
    assert!(skipped3, "同 sha 应跳过");

    // queries 存档闭环：archive_query → wiki_analyze 入队（自动再摄取产页）
    let skipped_q = wiki
        .archive_query(
            "向量检索问答",
            "什么是向量检索？",
            "向量检索是在高维空间寻找最近邻的技术。",
        )
        .await
        .unwrap();
    assert!(!skipped_q, "queries 存档应触发摄取");
    // queries 页型直接落库（不依赖 LLM 生成）
    let qpage = wiki.get_page("query-向量检索问答").await.unwrap();
    assert_eq!(qpage.page_type, "queries", "存档应产 queries 页型");
    assert!(qpage.content.contains("**问**"), "queries 页应含问答结构");
    wait_jobs(&pool, &["wiki_analyze"]).await;

    handle.shutdown();
    handle.join().await;
}

/// 人写页面不被 LLM 覆盖 → proposal。
#[tokio::test]
async fn human_page_produces_proposal_not_overwrite() {
    let (pool, wiki, handle, _pg) = setup(vec![
        // 分析
        json!({"entities": ["张三"], "concepts": [], "links": [], "conflicts": [], "source_title": "文档"}),
        // 生成：试图写「张三」页（人写页）→ 提案
        json!({"pages": [
            {"slug": "张三", "page_type": "entity", "title": "张三", "content": "# 张三\n\nLLM 版内容"},
        ]}),
    ])
    .await;

    // 人先写页面
    wiki.put_page("张三", "张三", "# 张三\n\n人工编写的内容。")
        .await
        .unwrap();
    let before = wiki.get_page("张三").await.unwrap();
    assert_eq!(before.origin, "human");
    assert_eq!(before.version, 1);

    wiki.ingest("文档", "张三的信息。").await.unwrap();
    wait_jobs(&pool, &["wiki_analyze", "wiki_generate"]).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let after = wiki.get_page("张三").await.unwrap();
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
        .apply_proposal("张三", data["proposal_content"].as_str().unwrap(), "张三")
        .await
        .unwrap();
    assert_eq!(merged.version, 2);
    assert!(merged.content.contains("LLM 版内容"));

    handle.shutdown();
    handle.join().await;
}

/// lint：注入死链 + 孤儿页 → 全部报出。
#[tokio::test]
async fn lint_reports_dead_links_and_orphans() {
    let (pool, wiki, handle, _pg) = setup(vec![]).await;

    // 正常互链两页 + 一个死链 + 一个孤儿
    wiki.put_page("正常页A", "A", "内容链接 [[正常页B]]。")
        .await
        .unwrap();
    wiki.put_page("正常页B", "B", "回链 [[正常页A]]。")
        .await
        .unwrap();
    wiki.put_page("带死链", "D", "这里有个 [[不存在的页面]]。")
        .await
        .unwrap();
    wiki.put_page("孤儿页", "O", "没有任何入链。")
        .await
        .unwrap();
    // 手动建链接表（put_page 不自动建边——模拟 ingest 后状态）
    sqlx::query("INSERT INTO wiki_links (from_slug, to_slug, weight) VALUES ('正常页A','正常页B',3.0), ('正常页B','正常页A',3.0), ('带死链','不存在的页面',3.0)")
        .execute(&pool)
        .await
        .unwrap();

    let report = wiki.lint().await.unwrap();
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

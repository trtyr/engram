//! P004-T007：wiki MCP 工具面全量对齐回归——13 个新 action（repair_review/维护/降级恢复面）
//! 从 HTTP 独有能力补进 MCP 单一入口，agent-first 维护（Karpathy 循环）的工具前提。
//! 每个新 action 至少一条断言；读写分类护栏在 dispatch.rs is_write/is_read_action 内登记。

mod support;

use serde_json::{Value, json};
use support::{app, create_key, login_token, mcp_initialize, mcp_rpc, rpc};

struct Ctx {
    app: axum::Router,
    token: String,
    /// 测试库守卫：随 Ctx 活到测试结束（提前 drop 会中途 FORCE 删库）
    _pg: support::TestPg,
}

impl Ctx {
    async fn new() -> Self {
        let (app, _pg) = app().await;
        let token = login_token(&app).await;
        Self { app, token, _pg }
    }
}

fn wiki_call(action: &str, args: Value) -> Value {
    let mut arguments = serde_json::Map::new();
    arguments.insert("action".into(), json!(action));
    if let Value::Object(m) = args {
        for (k, v) in m {
            arguments.insert(k, v);
        }
    }
    rpc(
        2,
        "tools/call",
        json!({ "name": "wiki", "arguments": arguments }),
    )
}

/// 空库可通的无参/轻参 action：结构断言（不炸 + 关键字段在）。
#[tokio::test]
async fn wiki_curation_read_actions_reachable() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["wiki"]).await;
    mcp_initialize(&ctx.app, &key).await;

    // duplicates：空库 → candidates 空数组
    let (_, v) = mcp_rpc(&ctx.app, &key, wiki_call("duplicates", json!({}))).await;
    assert!(v.get("error").is_none(), "duplicates 应可达: {v}");
    assert!(v["result"]["content"][0]["text"].is_string());

    // folders：空库 → 数组
    let (_, v) = mcp_rpc(&ctx.app, &key, wiki_call("folders", json!({}))).await;
    assert!(v.get("error").is_none(), "folders 应可达: {v}");

    // query_gaps：空库 → 数组/对象
    let (_, v) = mcp_rpc(&ctx.app, &key, wiki_call("query_gaps", json!({}))).await;
    assert!(v.get("error").is_none(), "query_gaps 应可达: {v}");

    // proposals：无提案 → 空数组
    let (_, v) = mcp_rpc(&ctx.app, &key, wiki_call("proposals", json!({}))).await;
    assert!(v.get("error").is_none(), "proposals 应可达: {v}");

    // rebuild_links / rebuild_tsv：空库幂等 → 计数字段
    for action in ["rebuild_links", "rebuild_tsv"] {
        let (_, v) = mcp_rpc(&ctx.app, &key, wiki_call(action, json!({}))).await;
        assert!(v.get("error").is_none(), "{action} 应可达: {v}");
    }

    // insight_reset：无 dismissed 也应成功
    let (_, v) = mcp_rpc(&ctx.app, &key, wiki_call("insight_reset", json!({}))).await;
    assert!(v.get("error").is_none(), "insight_reset 应可达: {v}");

    // repair：空库确定性修复 → 报告结构
    let (_, v) = mcp_rpc(&ctx.app, &key, wiki_call("repair", json!({}))).await;
    assert!(v.get("error").is_none(), "repair 应可达: {v}");
}

/// 写路径 action：purpose_set 写读一致 / proposal_apply 落页 / insight_dismiss 幂等 /
/// repair_async 出 job_id / reembed 对不存在文档报 NotFound。
#[tokio::test]
async fn wiki_curation_write_actions_roundtrip() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["wiki"]).await;
    mcp_initialize(&ctx.app, &key).await;

    // purpose_set → purpose 读回 configured=true
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        wiki_call(
            "purpose_set",
            json!({ "goals": ["沉淀 RAG 学习知识"], "thesis": "检索层与知识层分离" }),
        ),
    )
    .await;
    assert!(v.get("error").is_none(), "purpose_set 应成功: {v}");
    let (_, v) = mcp_rpc(&ctx.app, &key, wiki_call("purpose", json!({}))).await;
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        text.contains("configured"),
        "purpose 读回应带 configured 标记: {text}"
    );
    assert!(text.contains("沉淀 RAG 学习知识"), "写读应一致: {text}");

    // proposal_apply → get_page 落页验证
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        wiki_call(
            "proposal_apply",
            json!({
                "slug": "t007-proposal-page",
                "title": "T007 提案页",
                "content": "# T007 提案页\n\n人审合入的内容。",
                "via": "ai"
            }),
        ),
    )
    .await;
    assert!(v.get("error").is_none(), "proposal_apply 应成功: {v}");
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        wiki_call("get_page", json!({ "slug": "t007-proposal-page" })),
    )
    .await;
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(text.contains("人审合入的内容"), "提案应已落页: {text}");

    // insight_dismiss：任意 key 幂等 ok
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        wiki_call("insight_dismiss", json!({ "key": "some-insight-key" })),
    )
    .await;
    assert!(v.get("error").is_none(), "insight_dismiss 应幂等成功: {v}");

    // repair_async：返回 job_id
    let (_, v) = mcp_rpc(&ctx.app, &key, wiki_call("repair_async", json!({}))).await;
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        text.contains("job_id"),
        "repair_async 应返回 job_id: {text}"
    );

    // reembed：不存在文档 → NotFound（错误可见，不静默）
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        wiki_call("reembed", json!({ "doc_id": "01a00000000000000000000000" })),
    )
    .await;
    assert!(v.get("error").is_some(), "reembed 不存在文档应报错: {v}");
}

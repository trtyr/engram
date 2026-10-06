//! wiki MCP 工具面回归——P016 运维收敛后的存活面（lint/repair/duplicates/insights 等运维
//! action 已随 maintain_wiki 定期巡逻退役，AI 也不再手动触发）。
//! 读写分类护栏在 dispatch.rs is_write/is_read_action 内登记。

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

    // folders：空库 → 数组
    let (_, v) = mcp_rpc(&ctx.app, &key, wiki_call("folders", json!({}))).await;
    assert!(v.get("error").is_none(), "folders 应可达: {v}");
}

/// 写路径 action：purpose_set 写读一致（运维触发类 action 已退役——巡逻 Agent 承担）。
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
}

/// 退役 action 不可再调用（unknown_action 明确报错，不静默）。
#[tokio::test]
async fn wiki_retired_ops_actions_rejected() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["wiki"]).await;
    mcp_initialize(&ctx.app, &key).await;
    for action in [
        "lint",
        "lint_deep",
        "merge",
        "duplicates",
        "query_gaps",
        "repair",
        "repair_async",
        "insights",
        "insight_dismiss",
        "insight_reset",
        "rebuild_links",
        "rebuild_tsv",
        "reembed",
    ] {
        let (_, v) = mcp_rpc(&ctx.app, &key, wiki_call(action, json!({}))).await;
        assert!(
            v.get("error").is_some(),
            "退役 action {action} 应报 unknown_action: {v}"
        );
    }
}

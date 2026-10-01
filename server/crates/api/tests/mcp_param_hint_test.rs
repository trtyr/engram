//! dispatch from_args 参数名指路回归（EN-11，P002-T005，拍板 B）：
//! 传错主键名（相邻域直觉名）→ INVALID_PARAMS 报错点名未识别键，打回即学会；
//! 合法参数不触发提示；宽容解析（D7）原有行为不受影响。

mod support;

use serde_json::{Value, json};
use support::{app, create_key, login_token, mcp_initialize, mcp_rpc, rpc};
use tower::util::ServiceExt;

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

fn domain_call(domain: &str, action: &str, args: Value) -> Value {
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
        json!({ "name": domain, "arguments": arguments }),
    )
}

#[tokio::test]
async fn wrong_key_name_error_names_the_unknown_key() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["memory"]).await;
    mcp_initialize(&ctx.app, &key).await;

    // EN-11 原始实例①：jobs.get 传 job_id（正确为 id）；jobs 域 scope 兜底映射 memory
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        domain_call(
            "jobs",
            "get",
            json!({ "job_id": "01a00000000000000000000000" }),
        ),
    )
    .await;
    assert!(v.get("error").is_some(), "错名参数应被打回：{v}");
    let msg = v["error"]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("missing field"),
        "报错应含 missing field 主语：{msg}"
    );
    assert!(
        msg.contains("job_id"),
        "报错应点名调用方传入的未识别键 job_id：{msg}"
    );
    assert!(msg.contains("本操作不认识"), "报错应含指路话术：{msg}");
}

#[tokio::test]
async fn valid_args_no_hint_and_still_work() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["todos"]).await;
    mcp_initialize(&ctx.app, &key).await;

    // 合法参数：正常执行，回执不含指路话术（tickets 与 todos 同 scope 同底座）
    let (_, v) = mcp_rpc(&ctx.app, &key, domain_call("tickets", "list", json!({}))).await;
    assert!(v.get("error").is_none(), "tickets list 不应报错：{v}");
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        !text.contains("本操作不认识"),
        "合法调用不得带指路话术：{text}"
    );

    // 宽容解析（D7）不受影响：数字字段收到字符串仍被强转
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        domain_call("todos", "list", json!({ "limit": "5" })),
    )
    .await;
    assert!(
        v.get("error").is_none(),
        "string→int 宽容解析应继续生效：{v}"
    );
}

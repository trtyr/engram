//! codegraph 产物上传（公网多Agent P001 步骤4）：
//! upload 型条目 db+HEAD 上传、声明式新鲜度、坏产物/错型报错可行动。
//! 测试不依赖 codegraph CLI——上传校验只看 SQLite 魔数与 head 形态。

mod support;

use serde_json::{Value, json};
use support::{app, create_key, login_token, mcp_call_json, mcp_initialize, mcp_rpc, rpc};

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

fn mcp_call(action: &str, args: Value) -> Value {
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
        json!({"name": "codegraph", "arguments": arguments}),
    )
}

/// "SQLite format 3\0"（16 字节 SQLite 魔数，上传校验的最小合法 db）
const GOOD_DB_B64: &str = "U1FMaXRlIGZvcm1hdCAzAA==";
/// "not-sqlite"（非 SQLite 内容，应被魔数校验拒绝）
const BAD_DB_B64: &str = "bm90LXNxbGl0ZQ==";

#[tokio::test]
async fn upload_creates_updates_and_reflects() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["codegraph"]).await;
    mcp_initialize(&ctx.app, &key).await;

    // 首传：新建 upload 型条目，status 直接 ready
    let out = mcp_call_json(
        &ctx.app,
        &key,
        "codegraph",
        json!({"action": "upload", "name": "up-proj", "head": "a1b2c3d4e5", "db_b64": GOOD_DB_B64}),
    )
    .await;
    assert_eq!(out["status"], "ready", "{out}");
    assert_eq!(out["source_kind"], "upload", "{out}");
    assert_eq!(out["head"], "a1b2c3d4e5", "{out}");
    assert_eq!(out["db_bytes"], 16, "{out}");
    assert!(out["uploaded_at"].is_string(), "{out}");

    // list 反映新产物与 head（声明式新鲜度对外可见）
    let list = mcp_call_json(&ctx.app, &key, "codegraph", json!({"action": "list"})).await;
    let row = list
        .as_array()
        .and_then(|a| a.iter().find(|r| r["name"] == "up-proj"))
        .expect("list 应含 up-proj")
        .clone();
    assert_eq!(row["head"], "a1b2c3d4e5", "{row}");
    assert_eq!(row["source_kind"], "upload", "{row}");
    assert_eq!(row["status"], "ready", "{row}");
    assert_eq!(row["usable"], true, "产物在盘即 usable：{row}");

    // 产物落在 query 消费路径上（path/.codegraph/codegraph.db——CLI/直读 db 的唯一出处）
    let path = row["path"].as_str().expect("list 应带 path");
    assert!(
        std::path::Path::new(path)
            .join(".codegraph")
            .join("codegraph.db")
            .is_file(),
        "产物应在 {path}/.codegraph/codegraph.db"
    );

    // 覆盖上传：同名同 id，head 更新
    let id = out["id"].as_str().unwrap().to_string();
    let out2 = mcp_call_json(
        &ctx.app,
        &key,
        "codegraph",
        json!({"action": "upload", "name": "up-proj", "head": "b2c3d4e5f6", "db_b64": GOOD_DB_B64}),
    )
    .await;
    assert_eq!(out2["id"], out["id"], "同名覆盖应命中同一条目");
    assert_eq!(out2["head"], "b2c3d4e5f6", "{out2}");
    let _ = id;
}

#[tokio::test]
async fn upload_rejects_bad_artifacts_and_repo_type() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["codegraph"]).await;
    mcp_initialize(&ctx.app, &key).await;

    // 坏 head：非 hex
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        mcp_call(
            "upload",
            json!({"name": "bad-head", "head": "zz-not-hex", "db_b64": GOOD_DB_B64}),
        ),
    )
    .await;
    let msg = v["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("commit hash"), "坏 head 报错应可行动：{msg}");

    // 坏 db：非 SQLite
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        mcp_call(
            "upload",
            json!({"name": "bad-db", "head": "a1b2c3d4e5", "db_b64": BAD_DB_B64}),
        ),
    )
    .await;
    let msg = v["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("SQLite"), "坏 db 报错应可行动：{msg}");

    // repo 型冲突：先注册一个本地路径条目，再 upload 同名
    let tmp = std::env::temp_dir().join(format!("cg-repo-type-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&tmp).unwrap();
    let _ = mcp_call_json(
        &ctx.app,
        &key,
        "codegraph",
        json!({
            "action": "register",
            "name": "repo-type-proj",
            "source_uri": tmp.to_string_lossy()
        }),
    )
    .await;
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        mcp_call(
            "upload",
            json!({"name": "repo-type-proj", "head": "a1b2c3d4e5", "db_b64": GOOD_DB_B64}),
        ),
    )
    .await;
    let msg = v["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("repo 型"), "repo 型冲突应明确拒绝：{msg}");

    let _ = std::fs::remove_dir_all(&tmp);
}

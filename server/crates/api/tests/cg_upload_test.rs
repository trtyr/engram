//! codegraph 产物上传（公网多Agent P001 步骤4）：
//! upload 型条目 db+HEAD 上传、声明式新鲜度、坏产物/错型报错可行动。
//! 测试不依赖 codegraph CLI——上传校验只看 SQLite 魔数与 head 形态。

mod support;

use serde_json::{Value, json};
use support::{app, create_key, login_token, mcp_call_json, mcp_initialize, mcp_rpc, rpc};

use axum::http::{Request, StatusCode};

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

/// "not-sqlite"（非 SQLite 内容，应被魔数校验拒绝）
const BAD_DB_B64: &str = "bm90LXNxbGl0ZQ==";

/// 造一份「最小合法 codegraph 产物」：真 SQLite 文件 + `project_metadata`（R3 校验要读它）。
/// `version`/`extraction` 传 `None` = 不写该键（用于「缺版本键」的负例）。
fn artifact_db_b64(version: Option<&str>, extraction: Option<&str>) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("codegraph.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "CREATE TABLE project_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL, \
             updated_at INTEGER NOT NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO project_metadata (key, value, updated_at) \
             VALUES ('index_state','complete',0)",
            [],
        )
        .unwrap();
        for (k, v) in [
            ("indexed_with_version", version),
            ("indexed_with_extraction_version", extraction),
        ] {
            if let Some(v) = v {
                conn.execute(
                    "INSERT INTO project_metadata (key, value, updated_at) VALUES (?1, ?2, 0)",
                    rusqlite::params![k, v],
                )
                .unwrap();
            }
        }
    }
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(std::fs::read(&path).unwrap())
}

#[tokio::test]
async fn upload_creates_updates_and_reflects() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["codegraph"]).await;
    mcp_initialize(&ctx.app, &key).await;

    // 首传：新建条目（来源标注 client_upload），status 直接 ready，产物元数据落库
    let db = artifact_db_b64(Some("1.5.0"), Some("24"));
    let out = mcp_call_json(
        &ctx.app,
        &key,
        "codegraph",
        json!({"action": "upload", "name": "up-proj", "head": "a1b2c3d4e5", "db_b64": db.clone()}),
    )
    .await;
    assert_eq!(out["status"], "ready", "{out}");
    assert_eq!(out["source_kind"], "client_upload", "{out}");
    assert_eq!(out["head"], "a1b2c3d4e5", "{out}");
    assert!(out["db_bytes"].as_u64().unwrap() > 0, "{out}");
    assert!(out["uploaded_at"].is_string(), "{out}");
    assert!(out["produced_at"].is_string(), "产物产出时刻应落库：{out}");
    assert_eq!(
        out["built_with_version"], "1.5.0",
        "R3：产物内 CLI 版本应读出并落库（可追溯）：{out}"
    );
    assert!(
        out["last_producer"]
            .as_str()
            .unwrap_or_default()
            .starts_with("client:"),
        "投递者应可追溯（client:<key 名>）：{out}"
    );

    // list 反映新产物与 head（声明式新鲜度对外可见）
    let list = mcp_call_json(&ctx.app, &key, "codegraph", json!({"action": "list"})).await;
    let row = list
        .as_array()
        .and_then(|a| a.iter().find(|r| r["name"] == "up-proj"))
        .expect("list 应含 up-proj")
        .clone();
    assert_eq!(row["head"], "a1b2c3d4e5", "{row}");
    assert_eq!(row["source_kind"], "client_upload", "{row}");
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

    // 覆盖上传：同名同 id，head 更新，**上一份产物压进 stats.previous 留痕**（R1 后到者覆盖 + 完整留痕）
    let out2 = mcp_call_json(
        &ctx.app,
        &key,
        "codegraph",
        json!({"action": "upload", "name": "up-proj", "head": "b2c3d4e5f6", "db_b64": db}),
    )
    .await;
    assert_eq!(out2["id"], out["id"], "同名覆盖应命中同一条目");
    assert_eq!(out2["head"], "b2c3d4e5f6", "{out2}");
    assert_eq!(
        out2["stats"]["previous"]["head"], "a1b2c3d4e5",
        "被覆盖的上一份产物应留痕：{out2}"
    );
}

#[tokio::test]
async fn upload_rejects_bad_artifacts_and_overwrites_cloud_entry() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["codegraph"]).await;
    mcp_initialize(&ctx.app, &key).await;
    let db = artifact_db_b64(Some("1.5.0"), Some("24"));

    // 坏 head：非 hex
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        mcp_call(
            "upload",
            json!({"name": "bad-head", "head": "zz-not-hex", "db_b64": db.clone()}),
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

    // 云自建型（cloud_index）可被上传覆盖（0055 起撤销「本机索引不归上传通道管」的旧规则——R1「后到者覆盖」）。
    // 注册入口已收敛为「只收 git 地址 + 注册即 clone」（2026-09-21）：夹具用本地 file:// 真仓库，不联网。
    let (_src, url) = git_repo_url("cloud-entry-repo");
    let reg = mcp_call_json(
        &ctx.app,
        &key,
        "codegraph",
        json!({
            "action": "register",
            "name": "cloud-entry-proj",
            "source_uri": url
        }),
    )
    .await;
    assert_eq!(
        reg["source_kind"], "cloud_index",
        "register 默认来源标注应为 cloud_index：{reg}"
    );

    let up = mcp_call_json(
        &ctx.app,
        &key,
        "codegraph",
        json!({"action": "upload", "name": "cloud-entry-proj", "head": "a1b2c3d4e5", "db_b64": db}),
    )
    .await;
    assert_eq!(up["id"], reg["id"], "同名应命中同一条目（不新建）：{up}");
    assert_eq!(
        up["source_kind"], "client_upload",
        "被上传刷新后来源标注应改判：{up}"
    );
    assert_eq!(up["head"], "a1b2c3d4e5", "{up}");
}

/// R3 产物版本校验（2026-09-21 已定口径：CLI 版本**硬拒** / extraction 版本**仅告警**）。
#[tokio::test]
async fn upload_enforces_artifact_version_policy() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["codegraph"]).await;
    mcp_initialize(&ctx.app, &key).await;

    // 非 pin 版本 → 拒收；文案含实际值 / pin / 可行动指引
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        mcp_call(
            "upload",
            json!({
                "name": "ver-mismatch",
                "head": "a1b2c3d4e5",
                "db_b64": artifact_db_b64(Some("1.4.2"), Some("24")),
            }),
        ),
    )
    .await;
    let msg = v["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("产物版本不符"), "非 pin 版本应明确拒收：{msg}");
    assert!(
        msg.contains("1.4.2") && msg.contains("1.5.0"),
        "文案应含实际值与本服务 pin：{msg}"
    );
    assert!(
        msg.contains("codegraph upgrade"),
        "文案应含可行动指引：{msg}"
    );

    // 缺 indexed_with_version 键 → 拒收（无法校验即不收：宁缺勿脏）
    let (_, v) = mcp_rpc(
        &ctx.app,
        &key,
        mcp_call(
            "upload",
            json!({
                "name": "ver-missing",
                "head": "a1b2c3d4e5",
                "db_b64": artifact_db_b64(None, Some("24")),
            }),
        ),
    )
    .await;
    let msg = v["error"]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("project_metadata.indexed_with_version"),
        "缺版本键应拒收并说明原因：{msg}"
    );

    // pin 版本但 extraction 口径不同 → 入库 + stats.extraction_warning（不阻断）
    let out = mcp_call_json(
        &ctx.app,
        &key,
        "codegraph",
        json!({
            "action": "upload",
            "name": "ver-extraction-drift",
            "head": "a1b2c3d4e5",
            "db_b64": artifact_db_b64(Some("1.5.0"), Some("22")),
        }),
    )
    .await;
    assert_eq!(out["status"], "ready", "extraction 漂移不应阻断入库：{out}");
    assert_eq!(out["built_with_version"], "1.5.0", "{out}");
    let warn = out["stats"]["extraction_warning"].as_str().unwrap_or("");
    assert!(
        warn.contains("22") && warn.contains("24"),
        "应写入 extraction 告警（含实际值与实测口径）：{out}"
    );

    // pin 一致 → 无告警
    let out2 = mcp_call_json(
        &ctx.app,
        &key,
        "codegraph",
        json!({
            "action": "upload",
            "name": "ver-clean",
            "head": "a1b2c3d4e5",
            "db_b64": artifact_db_b64(Some("1.5.0"), Some("24")),
        }),
    )
    .await;
    assert_eq!(out2["status"], "ready", "{out2}");
    assert!(
        out2["stats"]["extraction_warning"].is_null(),
        "口径一致时不应有告警：{out2}"
    );
}

/// R2 拉取侧：产物下载端点（`GET /codegraph/projects/{id}/artifact`）——
/// 返回原始字节 + 元数据响应头（head / 来源标注等，供 pull 端复用声明）；无 codegraph scope 的 key 403。
#[tokio::test]
async fn artifact_endpoint_exports_bytes_with_metadata() {
    use axum::body::Body;
    use tower::ServiceExt;

    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["codegraph"]).await;
    mcp_initialize(&ctx.app, &key).await;
    let db = artifact_db_b64(Some("1.5.0"), Some("24"));
    let out = mcp_call_json(
        &ctx.app,
        &key,
        "codegraph",
        json!({"action": "upload", "name": "pull-proj", "head": "a1b2c3d4e5", "db_b64": db}),
    )
    .await;
    let id = out["id"].as_str().expect("upload 应回 id").to_string();

    // 无 codegraph scope → 403（与其它 codegraph 端点同口径）
    let mem_key = create_key(&ctx.app, &ctx.token, &["memory"]).await;
    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/codegraph/projects/{id}/artifact"))
                .header("authorization", format!("Bearer {mem_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "memory-only key 拿产物应 403"
    );

    // 有 scope → 200：原始字节 + 元数据头齐全
    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/codegraph/projects/{id}/artifact"))
                .header("authorization", format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("x-codegraph-head")
            .and_then(|v| v.to_str().ok()),
        Some("a1b2c3d4e5"),
        "应带 head 声明（pull 端据此原样投给本地）"
    );
    assert_eq!(
        resp.headers()
            .get("x-codegraph-source-kind")
            .and_then(|v| v.to_str().ok()),
        Some("client_upload"),
        "应带来源标注"
    );
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(bytes.len() > 16, "应回产物字节本体（{} 字节）", bytes.len());
    assert_eq!(&bytes[..16], b"SQLite format 3\0", "应是原始 SQLite 本体");
}

/// 造一份「大产物」：真 SQLite + `project_metadata` + 填充表（令 base64 body 越 4MB）。
fn big_artifact_db_b64(pad_bytes: usize) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("codegraph.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "CREATE TABLE project_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL, \
             updated_at INTEGER NOT NULL)",
            [],
        )
        .unwrap();
        for (k, v) in [
            ("index_state", "complete"),
            ("indexed_with_version", "1.5.0"),
            ("indexed_with_extraction_version", "24"),
        ] {
            conn.execute(
                "INSERT INTO project_metadata (key, value, updated_at) VALUES (?1, ?2, 0)",
                rusqlite::params![k, v],
            )
            .unwrap();
        }
        conn.execute("CREATE TABLE pad (id INTEGER PRIMARY KEY, blob BLOB)", [])
            .unwrap();
        conn.execute(
            "INSERT INTO pad (id, blob) VALUES (1, ?1)",
            rusqlite::params![vec![7u8; pad_bytes]],
        )
        .unwrap();
    }
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(std::fs::read(&path).unwrap())
}

/// 大产物上传必须成功：越 4MB 的 body 不得被 MCP 层掐断。
///
/// 回归锁（2026-09-21 活体实测抓出）：rmcp 的 `max_request_body_bytes` 默认 4MB，
/// 真实 codegraph 产物（24.7MB → 32MB body）经 MCP 上传必被 413/连接重置掐断，
/// push 通道实际不可用。修复后 >4MB body 必须正常落库。
#[tokio::test]
async fn upload_accepts_body_over_rmcp_default_limit() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["codegraph"]).await;
    mcp_initialize(&ctx.app, &key).await;
    // 3.3MB 产物 → base64 ≈ 4.4MB body（刚好越过 rmcp 默认 4MB）
    let db = big_artifact_db_b64(3_300_000);
    assert!(db.len() > 4 * 1024 * 1024, "夹具本身要越线：{}", db.len());
    let out = mcp_call_json(
        &ctx.app,
        &key,
        "codegraph",
        json!({"action": "upload", "name": "big-proj", "head": "f00dbabe01", "db_b64": db}),
    )
    .await;
    assert_eq!(out["status"], "ready", "{out}");
    assert_eq!(out["source_kind"], "client_upload", "{out}");
}

/// 造一份「可查」的产物：`project_metadata`（过 R3 校验）+ `nodes`（explore 直读要用的列）。
fn queryable_artifact_db_b64() -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("codegraph.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "CREATE TABLE project_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL, \
             updated_at INTEGER NOT NULL)",
            [],
        )
        .unwrap();
        for (k, v) in [
            ("index_state", "complete"),
            ("indexed_with_version", "1.5.0"),
            ("indexed_with_extraction_version", "24"),
        ] {
            conn.execute(
                "INSERT INTO project_metadata (key, value, updated_at) VALUES (?1, ?2, 0)",
                rusqlite::params![k, v],
            )
            .unwrap();
        }
        // nodes 列取真实 schema 的必需子集（explore_outline_query 读 name/kind/file_path/
        // start_line/end_line/signature）
        conn.execute(
            "CREATE TABLE nodes (id TEXT PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, \
             qualified_name TEXT NOT NULL, file_path TEXT NOT NULL, language TEXT NOT NULL, \
             start_line INTEGER NOT NULL, end_line INTEGER NOT NULL, start_column INTEGER NOT NULL, \
             end_column INTEGER NOT NULL, docstring TEXT, signature TEXT)",
            [],
        )
        .unwrap();
        for (id, kind, name, sig) in [
            (
                "n1",
                "function",
                "OutlineProbeFn",
                "fn OutlineProbeFn(x: i32) -> i32",
            ),
            ("n2", "function", "OtherProbeFn", "fn OtherProbeFn()"),
        ] {
            conn.execute(
                "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language, \
                 start_line, end_line, start_column, end_column, docstring, signature) \
                 VALUES (?1, ?2, ?3, ?4, 'src/probe.rs', 'rust', 3, 9, 1, 2, NULL, ?5)",
                rusqlite::params![id, kind, name, format!("crate::{name}"), sig],
            )
            .unwrap();
        }
    }
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(std::fs::read(&path).unwrap())
}

/// 本机 codegraph CLI 是否可用且 pin（1.5.0）匹配——决定 CLI 路径能否真跑。
fn cli_version_matches_pin() -> bool {
    std::process::Command::new("codegraph")
        .arg("version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            let s = String::from_utf8_lossy(&o.stdout);
            s.split_whitespace().last().map(str::to_string).as_deref() == Some("1.5.0")
        })
        .unwrap_or(false)
}

/// R7（task-6）：upload 型 + 查询端到端——上传的产物**真的能查一次**。
///
/// ① `explore`（db 直读）：**必验**——测试自己用 rusqlite 只读打开产物，证明「产物落在 query
///    消费路径上且查得到符号」（这步不依赖本机 CLI）；MCP 侧 `query()` 先过 `ensure_version`
///    （R5 事实），故本机 CLI 缺失/版本不符时 MCP 调用会被版本门拒——此时**显式打印 skip 原因**，
///    不静默通过。
/// ② `search`（CLI 路径）：本机有 pin 版 CLI 时验证「确实已进 CLI 路径（不再被版本门拦）」；
///    缺失时同样显式 skip 打印。
#[tokio::test]
async fn upload_then_query_end_to_end() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["codegraph"]).await;
    mcp_initialize(&ctx.app, &key).await;

    let db = queryable_artifact_db_b64();
    let out = mcp_call_json(
        &ctx.app,
        &key,
        "codegraph",
        json!({"action": "upload", "name": "query-proj", "head": "c0ffee1234", "db_b64": db}),
    )
    .await;
    let path = out["path"].as_str().expect("upload 应回 path").to_string();
    let artifact = std::path::Path::new(&path)
        .join(".codegraph")
        .join("codegraph.db");
    assert!(
        artifact.is_file(),
        "产物应落在 query 消费路径上：{}",
        artifact.display()
    );

    // ①-a db 直读必验（不依赖本机 CLI）：产物本体查得到符号
    {
        let conn = rusqlite::Connection::open_with_flags(
            &artifact,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("产物应能以只读方式打开");
        let hits: i64 = conn
            .query_row(
                "SELECT count(*) FROM nodes WHERE name LIKE '%OutlineProbeFn%'",
                [],
                |r| r.get(0),
            )
            .expect("nodes 表应可查");
        assert!(hits > 0, "产物本体应含 OutlineProbeFn（命中 {hits}）");
    }

    let cli_ok = cli_version_matches_pin();
    let call = |kind: &str, target: &str| {
        let app = ctx.app.clone();
        let key = key.clone();
        let body = json!({"name": "codegraph", "arguments": {
            "action": "query", "project": "query-proj", "kind": kind, "target": target
        }});
        async move { mcp_rpc(&app, &key, rpc(2, "tools/call", body)).await }
    };

    // ①-b explore（db 直读数据路径）
    let (_, v) = call("explore", "OutlineProbeFn").await;
    if let Some(err) = v.get("error") {
        assert!(
            !cli_ok,
            "本机 CLI 可用且 pin 匹配时 explore 不应被拦：{err}"
        );
        println!(
            "[skip] explore 未跑：本机 codegraph CLI 缺失或版本不符（query 先过版本门）——{}",
            err["message"].as_str().unwrap_or_default()
        );
    } else {
        let text = v["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default();
        let payload: serde_json::Value = serde_json::from_str(text).expect("explore 应回 JSON");
        let symbols = payload["symbols"].as_array().cloned().unwrap_or_default();
        assert!(!symbols.is_empty(), "explore 应有非空符号大纲：{payload}");
        println!(
            "[ok] explore 命中 {} 个符号（db 直读，产物真能查）",
            symbols.len()
        );
    }

    // ② search（CLI 路径）
    let (_, v2) = call("search", "OutlineProbeFn").await;
    if let Some(err) = v2.get("error") {
        let msg = err["message"].as_str().unwrap_or_default().to_string();
        if cli_ok {
            assert!(
                !msg.contains("CLI 不可用") && !msg.contains("版本不匹配"),
                "CLI 已装且 pin 匹配时不应在版本门被拦：{msg}"
            );
            println!("[ok] search 已进 CLI 路径（CLI 返回业务错误，非版本门拦截）：{msg}");
        } else {
            println!("[skip] search 未跑：本机 codegraph CLI 缺失或版本不符——{msg}");
        }
    } else {
        println!("[ok] search 走通 CLI 路径（返回归一结果）");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// HTTP multipart 上传入口（2026-09-21 入口收敛）：与 MCP 通道同桥同校验。
// 手搓 multipart 体——测试里不必为一个请求引 multipart 客户端依赖。
// ─────────────────────────────────────────────────────────────────────────────

/// 原始字节版产物（multipart 要二进制本体，不是 base64）。
fn artifact_db_bytes(version: Option<&str>, extraction: Option<&str>) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(artifact_db_b64(version, extraction))
        .unwrap()
}

/// 造一个真 git 仓库并给出 `file://` URL（注册入口只收 URI 形态，且注册即真 clone）。
/// 不联网：git 原生支持 file:// 传输，`--depth 1` 同样生效。
fn git_repo_url(name: &str) -> (tempfile::TempDir, String) {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join(name);
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "init"]);
    let url = format!("file://{}", repo.to_string_lossy().replace('\\', "/"));
    (tmp, url)
}

const MP_BOUNDARY: &str = "----engramtestboundary";

/// 拼 multipart/form-data 体：文本字段 + 可选文件字段。
fn multipart_body(text_fields: &[(&str, &str)], file: Option<(&str, &[u8])>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut field = |name: &str, filename: Option<&str>, value: &[u8]| {
        out.extend_from_slice(format!("--{MP_BOUNDARY}\r\n").as_bytes());
        let disp = match filename {
            Some(f) => format!(
                "Content-Disposition: form-data; name=\"{name}\"; filename=\"{f}\"\r\n\
                 Content-Type: application/octet-stream\r\n\r\n"
            ),
            None => format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n"),
        };
        out.extend_from_slice(disp.as_bytes());
        out.extend_from_slice(value);
        out.extend_from_slice(b"\r\n");
    };
    for (k, v) in text_fields {
        field(k, None, v.as_bytes());
    }
    if let Some((k, v)) = file {
        field(k, Some("codegraph.db"), v);
    }
    out.extend_from_slice(format!("--{MP_BOUNDARY}--\r\n").as_bytes());
    out
}

/// POST /codegraph/artifacts（multipart）→ (状态码, JSON 体)。
async fn post_multipart(app: &axum::Router, key: &str, body: Vec<u8>) -> (StatusCode, Value) {
    use axum::body::Body;
    use tower::util::ServiceExt;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/codegraph/artifacts")
                .header("authorization", format!("Bearer {key}"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={MP_BOUNDARY}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, v)
}

/// 上传成功 → status=ready / source_kind=client_upload；同名再传 → 覆盖 + `stats.previous` 留痕。
#[tokio::test]
async fn multipart_upload_creates_then_overwrites_with_previous_trace() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["codegraph"]).await;

    let db = artifact_db_bytes(Some("1.5.0"), Some("24"));
    let body = multipart_body(
        &[("name", "mp-proj"), ("head", "a1b2c3d4e5")],
        Some(("file", &db)),
    );
    let (status, v) = post_multipart(&ctx.app, &key, body).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let p = &v["project"];
    assert_eq!(p["name"], "mp-proj");
    assert_eq!(p["status"], "ready", "上传即就绪：{v}");
    assert_eq!(p["source_kind"], "client_upload", "{v}");
    assert_eq!(p["head"], "a1b2c3d4e5", "{v}");
    assert_eq!(v["db_bytes"].as_u64().unwrap(), db.len() as u64, "{v}");
    let first_id = p["id"].as_str().unwrap().to_string();

    // 同名再传（新 head）→ 同一 id、head 更新、上一份压进 stats.previous
    let db2 = artifact_db_bytes(Some("1.5.0"), Some("24"));
    let body2 = multipart_body(
        &[("name", "mp-proj"), ("head", "fffffffff")],
        Some(("file", &db2)),
    );
    let (status, v2) = post_multipart(&ctx.app, &key, body2).await;
    assert_eq!(status, StatusCode::CREATED, "{v2}");
    assert_eq!(
        v2["project"]["id"].as_str().unwrap(),
        first_id,
        "同名应覆盖同一条目而不是新建：{v2}"
    );
    assert_eq!(v2["project"]["head"], "fffffffff", "{v2}");
    assert!(
        v2["project"]["stats"]["previous"].is_object(),
        "上一份产物元数据应留痕在 stats.previous：{v2}"
    );
}

/// `head` 字段可留空 = 未声明（Web 入口只选文件）：落 NULL，不编造、也不拒收。
#[tokio::test]
async fn multipart_upload_without_head_stores_undeclared() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["codegraph"]).await;

    let db = artifact_db_bytes(Some("1.5.0"), Some("24"));
    let body = multipart_body(&[("name", "mp-nohead")], Some(("file", &db)));
    let (status, v) = post_multipart(&ctx.app, &key, body).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let p = &v["project"];
    assert!(p["head"].is_null(), "未声明 head 应落 NULL：{v}");
    assert_eq!(p["status"], "ready", "{v}");
    assert_eq!(p["source_kind"], "client_upload", "{v}");

    // 列表里新鲜度如实：无声明 head → 不虚报 stale
    use axum::body::Body;
    use tower::util::ServiceExt;
    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/codegraph/projects")
                .header("authorization", format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let rows: Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let item = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "mp-nohead")
        .expect("mp-nohead 应在列");
    assert!(
        item["freshness"]["hint"].is_string(),
        "无 head 应给出提示（不虚报）：{item}"
    );
}

/// 四态分型之三：坏产物（缺 SQLite 魔数）/ 版本不符（非 pin）/ 缺字段——全部 400 且文案可行动。
#[tokio::test]
async fn multipart_upload_rejects_bad_artifact_version_and_missing_fields() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["codegraph"]).await;

    // ① 坏产物：不是 SQLite
    let body = multipart_body(
        &[("name", "mp-bad")],
        Some(("file", b"not-a-sqlite-file".as_slice())),
    );
    let (status, v) = post_multipart(&ctx.app, &key, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("SQLite"), "应点明缺魔数：{msg}");

    // ② 版本不符（非 pin）：文案要给 pin + 升级指引
    let db = artifact_db_bytes(Some("9.9.9"), Some("24"));
    let body = multipart_body(&[("name", "mp-ver")], Some(("file", &db)));
    let (status, v) = post_multipart(&ctx.app, &key, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("1.5.0"), "应给出本服务 pin：{msg}");
    assert!(msg.contains("upgrade"), "应给出升级指引：{msg}");

    // ③ 缺 file 字段
    let body = multipart_body(&[("name", "mp-nofile")], None);
    let (status, v) = post_multipart(&ctx.app, &key, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("file"), "应指明缺 file：{msg}");

    // ④ 缺 name 字段
    let db = artifact_db_bytes(Some("1.5.0"), Some("24"));
    let body = multipart_body(&[], Some(("file", &db)));
    let (status, v) = post_multipart(&ctx.app, &key, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("name"), "应指明缺 name：{msg}");
}

/// 四态分型之四：认证/授权——没有 codegraph scope 的 key 一律 403（不落到上传逻辑）。
#[tokio::test]
async fn multipart_upload_requires_codegraph_scope() {
    let ctx = Ctx::new().await;
    let key = create_key(&ctx.app, &ctx.token, &["memory"]).await;

    let db = artifact_db_bytes(Some("1.5.0"), Some("24"));
    let body = multipart_body(&[("name", "mp-scope")], Some(("file", &db)));
    let (status, v) = post_multipart(&ctx.app, &key, body).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
}

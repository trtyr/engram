//! R 测试报告（macOS 四轮全量）落地项的集成测试：
//! remember 快捷写入、写操作瘦身（P0-1）、void 级联含 superseded（D2）与 restore 撤销、
//! wiki 版本快照/回滚/删除重建（建议 #5）、search 片段化（P0-2）、title 寻址（P1-11）、
//! sources 通道（E7）、skills 版本回滚（建议 #5）、search_all 跨域检索（P1-8）。

mod support;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use support::{app, create_key, expect_result, login_token, mcp_rpc, rpc};
use tower::util::ServiceExt;

/// 域工具调用：action + 平铺参数（渐进式发现语法；与 mcp_test 同款助手）。
fn call(id: i64, tool: &str, action: &str, args: Value) -> Value {
    let mut arguments = serde_json::Map::new();
    arguments.insert("action".into(), json!(action));
    if let Value::Object(map) = args {
        for (k, v) in map {
            arguments.insert(k, v);
        }
    }
    rpc(
        id,
        "tools/call",
        json!({"name": tool, "arguments": arguments}),
    )
}

/// 解析 tools/call 返回的 JSON 文本载荷。
fn out_json(v: &Value, what: &str) -> Value {
    let out = expect_result(v, what);
    serde_json::from_str(out["content"][0]["text"].as_str().expect("文本内容")).expect("JSON")
}

#[tokio::test]
async fn memory_remember_and_slim_write_responses() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["memory"]).await;

    // remember：一句话记忆（P1-9）——等价单轮 write_session
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            1,
            "memory",
            "remember",
            json!({"text": "用户的猫叫墨鱼，喜欢趴键盘上睡觉"}),
        ),
    )
    .await;
    let s = out_json(&v, "remember");
    assert_eq!(s["turns"], 1, "remember 应生成单轮会话：{s}");
    assert!(s.get("content").is_none(), "P0-1：写操作不应回显正文：{s}");
    assert!(s["id"].is_string() && s["agent"] == "mcp-test", "{s}");

    // 空文本响亮拒绝
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(2, "memory", "remember", json!({"text": "  "})),
    )
    .await;
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("text 不能为空"),
        "{v}"
    );

    // write_session 同样瘦身：turns 换轮次数，无 content 正文
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            3,
            "memory",
            "write_session",
            json!({"distill": "off", "turns": [{"speaker": "user", "text": "瘦身校验"}]}),
        ),
    )
    .await;
    let s = out_json(&v, "write_session slim");
    assert_eq!(s["turns"], 1);
    assert!(s.get("content").is_none(), "不应回显 turns 全文：{s}");
}

/// D2 + 恢复：void 级联归档 active **和 superseded** 原子；restore 撤销作废并按
/// superseded_by 还原各自状态。
#[tokio::test]
async fn memory_void_cascades_superseded_and_restore() {
    let (app, pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["memory"]).await;

    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            1,
            "memory",
            "write_session",
            json!({"distill": "off", "turns": [{"speaker": "user", "text": "级联测试源会话"}]}),
        ),
    )
    .await;
    let sid = out_json(&v, "write_session")["id"]
        .as_str()
        .unwrap()
        .to_string();

    let pool = sqlx::PgPool::connect(&support::connection_url(&pg).await.unwrap())
        .await
        .unwrap();
    // 造数为已蒸馏（done）会话——与真实「蒸馏后含 active + superseded 原子」场景一致；
    // pending 会话正常不会有原子，void 的级联只对 done 会话触发
    sqlx::query("UPDATE raw_sessions SET distill_status = 'done' WHERE id = $1")
        .bind(sid.parse::<sqlx::types::Uuid>().unwrap())
        .execute(&pool)
        .await
        .unwrap();
    let atom_a = uuid::Uuid::now_v7();
    let atom_b = uuid::Uuid::now_v7();
    for (id, status, sup) in [
        (atom_a, "active", Option::<uuid::Uuid>::None),
        (atom_b, "superseded", Some(atom_a)),
    ] {
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, status, superseded_by, source_refs) \
             VALUES ($1, 'fact', $2, $3, $4, $5::jsonb)",
        )
        .bind(id)
        .bind(format!("事实-{status}"))
        .bind(status)
        .bind(sup)
        .bind(format!(r#"[{{"session_id":"{sid}"}}]"#))
        .execute(&pool)
        .await
        .unwrap();
    }

    // void → 两个原子都归档（修复前 superseded 不动）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            2,
            "memory",
            "forget",
            json!({"session_id": sid, "mode": "void"}),
        ),
    )
    .await;
    expect_result(&v, "forget void");
    let (archived,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM atoms WHERE status = 'archived'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(archived, 2, "D2：superseded 原子应一并归档");

    // list_atoms 默认 active → 空；status=all → 2 条 archived
    let (_, v) = mcp_rpc(&app, &key, call(3, "memory", "list_atoms", json!({}))).await;
    let rows = out_json(&v, "list_atoms default");
    assert_eq!(rows.as_array().unwrap().len(), 0, "默认只查 active（P1-6）");
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(4, "memory", "list_atoms", json!({"status": "all"})),
    )
    .await;
    assert_eq!(
        out_json(&v, "list_atoms all").as_array().unwrap().len(),
        2,
        "all 出口应看到归档历史"
    );

    // restore → 会话回 pending（off 标记保留在 metadata），原子按 superseded_by 各归其位
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            5,
            "memory",
            "forget",
            json!({"session_id": sid, "mode": "restore"}),
        ),
    )
    .await;
    let r = out_json(&v, "forget restore");
    assert_eq!(r["restored_atoms"], 2, "{r}");
    let mut statuses: Vec<String> = sqlx::query_as("SELECT status FROM atoms ORDER BY content")
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|s: (String,)| s.0)
        .collect();
    statuses.sort();
    assert_eq!(statuses, vec!["active", "superseded"], "恢复应区分两类状态");
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(6, "memory", "get_session", json!({"session_id": sid})),
    )
    .await;
    let s = out_json(&v, "get_session after restore");
    assert_eq!(s["distill_status"], "done", "恢复到作废前状态：{s}");
}

/// 版本通道：write_page 覆盖自动留快照；删除页可从快照重建；title 寻址；search 片段化。
#[tokio::test]
async fn wiki_versions_restore_snippets_and_title_addressing() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["wiki"]).await;

    // v1 写入 → 覆盖 v2（快照记录 v1）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            1,
            "wiki",
            "write_page",
            json!({"slug": "zz-vtest", "title": "版本测试页", "content": "第一版内容"}),
        ),
    )
    .await;
    let page = out_json(&v, "write_page v1");
    assert!(
        page["content_chars"].is_i64() && page["content_omitted"] == json!(true),
        "P0-1：写页面回元数据：{page}"
    );
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            2,
            "wiki",
            "write_page",
            json!({"slug": "zz-vtest", "title": "版本测试页", "content": "第二版内容 beta UniqueMark"}),
        ),
    )
    .await;
    expect_result(&v, "write_page v2");

    // title 寻址（P1-11）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(3, "wiki", "get_page", json!({"slug": "版本测试页"})),
    )
    .await;
    let page = out_json(&v, "get_page by title");
    assert_eq!(page["slug"], "zz-vtest");

    // list_pages 带 content_chars（P1-7，验收方空库未确认项）
    let (_, v) = mcp_rpc(&app, &key, call(31, "wiki", "list_pages", json!({}))).await;
    let rows = out_json(&v, "list_pages").as_array().unwrap().clone();
    let row = rows
        .iter()
        .find(|r| r["slug"] == "zz-vtest")
        .expect("列表含测试页");
    assert!(
        row["content_chars"].is_i64() && row["content_omitted"] == json!(true),
        "list_pages 行应带 content_chars：{row}"
    );

    // search 片段化（P0-2）：命中带片段与 content_chars，不再拖全文
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(4, "wiki", "search", json!({"query": "beta"})),
    )
    .await;
    let r = out_json(&v, "wiki search");
    let pages = r["pages"].as_array().expect("pages");
    let hit = pages
        .iter()
        .find(|p| p["slug"] == "zz-vtest")
        .expect("命中测试页");
    assert_eq!(hit["content_omitted"], json!(true));
    assert!(
        hit["content"].as_str().unwrap().contains("beta"),
        "片段应含命中词：{hit}"
    );
    assert!(
        !hit["content"].as_str().unwrap().contains("第一版"),
        "片段不应是全文：{hit}"
    );

    // versions：覆盖产生 v1 快照（列表不带正文）
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(5, "wiki", "versions", json!({"slug": "zz-vtest"})),
    )
    .await;
    let rows = out_json(&v, "versions").as_array().unwrap().clone();
    assert_eq!(rows.len(), 1, "覆盖一次应有一条快照");
    assert_eq!(rows[0]["version"], 1);
    assert!(rows[0].get("content").is_none(), "列表不带正文");
    assert!(rows[0]["content_chars"].is_i64());

    // version_content 预览 → delete → 快照保底 → restore 重建
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            6,
            "wiki",
            "version_content",
            json!({"slug": "zz-vtest", "version": 1}),
        ),
    )
    .await;
    let c = out_json(&v, "version_content");
    assert!(c["content"].as_str().unwrap().contains("第一版"));
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(7, "wiki", "delete_page", json!({"slug": "zz-vtest"})),
    )
    .await;
    expect_result(&v, "delete_page");
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(8, "wiki", "versions", json!({"slug": "zz-vtest"})),
    )
    .await;
    let rows = out_json(&v, "versions after delete")
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(rows.len(), 2, "删除前最后状态也留快照");
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            9,
            "wiki",
            "restore_version",
            json!({"slug": "zz-vtest", "version": 1}),
        ),
    )
    .await;
    let page = out_json(&v, "restore_version");
    assert_eq!(page["slug"], "zz-vtest");
    assert_eq!(page["version"], 3, "重建版本号接续快照史：{page}");
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(10, "wiki", "get_page", json!({"slug": "zz-vtest"})),
    )
    .await;
    let page = out_json(&v, "get_page after restore");
    assert!(
        page["content"].as_str().unwrap().contains("第一版"),
        "恢复到 v1 内容：{page}"
    );

    // sources 通道（E7）：操作可达（空库返回空数组）
    let (_, v) = mcp_rpc(&app, &key, call(11, "wiki", "sources", json!({}))).await;
    let rows = out_json(&v, "sources");
    assert!(rows.as_array().is_some(), "sources 应返回数组：{rows}");
}

/// 技能版本通道：update 留快照 → versions 列表 → restore 回滚。
#[tokio::test]
async fn skills_versions_and_restore_journey() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["skills"]).await;

    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            1,
            "skills",
            "create",
            json!({"name": "rev-journey", "slug": "rev-journey", "content": "v1 正文"}),
        ),
    )
    .await;
    let s = out_json(&v, "skills create");
    assert!(
        s["content_chars"].is_i64() && s.get("content").is_none(),
        "写操作瘦身：{s}"
    );

    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            2,
            "skills",
            "update",
            json!({"slug": "rev-journey", "content": "v2 正文"}),
        ),
    )
    .await;
    expect_result(&v, "skills update");

    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(3, "skills", "versions", json!({"slug": "rev-journey"})),
    )
    .await;
    let rows = out_json(&v, "versions").as_array().unwrap().clone();
    assert_eq!(rows.len(), 2, "create+update 各一条快照");
    assert!(rows[0].get("content").is_none(), "列表不带正文");
    let create_rev = rows
        .iter()
        .find(|r| r["origin"] == "create")
        .expect("create 快照");
    let rid = create_rev["id"].as_str().unwrap().to_string();

    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            4,
            "skills",
            "restore",
            json!({"slug": "rev-journey", "revision_id": rid}),
        ),
    )
    .await;
    expect_result(&v, "restore");
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(5, "skills", "get", json!({"slug": "rev-journey"})),
    )
    .await;
    let s = out_json(&v, "get after restore");
    assert!(
        s["content"].as_str().unwrap().contains("v1 正文"),
        "应回滚到 create 版本：{s}"
    );
}

/// search_all（P1-8）：一次调用跨域命中（memory/wiki/todos 三 scope）。
#[tokio::test]
async fn search_all_fans_out_across_scoped_domains() {
    let (app, _pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["memory", "wiki", "todos"]).await;

    // 各域铺一条可检索目标
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(1, "todos", "add", json!({"title": "墨鱼全局检索目标"})),
    )
    .await;
    expect_result(&v, "todo add");
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            2,
            "wiki",
            "write_page",
            json!({"slug": "zz-searchall", "title": "墨鱼页", "content": "墨鱼 squid 标记正文"}),
        ),
    )
    .await;
    expect_result(&v, "write_page");

    // search_all 是独立工具（非域工具）：直接 tools/call，无 action 信封
    let (_, v) = mcp_rpc(
        &app,
        &key,
        rpc(
            3,
            "tools/call",
            json!({"name": "search_all", "arguments": {"query": "墨鱼"}}),
        ),
    )
    .await;
    let r = out_json(&v, "search_all");
    assert!(
        r.get("memory").is_some(),
        "memory 域应出现（scope 内）：{r}"
    );
    let wiki_hits = r["wiki"].as_array().expect("wiki 数组");
    assert!(
        wiki_hits.iter().any(|p| p["slug"] == "zz-searchall"),
        "wiki 应命中：{r}"
    );
    let todos = r["todos"].as_array().expect("todos 数组");
    assert!(
        todos.iter().any(|t| t["title"] == "墨鱼全局检索目标"),
        "todos 应命中：{r}"
    );
    assert!(
        r.get("projects").is_none(),
        "无 project scope 的域不出现：{r}"
    );
}

/// 验收遗留 #1：memory search 的 L3 画像命中默认不带 evidence_refs（与 context 同口径）；
/// include_evidence=true 显式开启。
#[tokio::test]
async fn memory_search_strips_l3_evidence_refs_by_default() {
    let (app, pg) = app().await;
    let key = create_key(&app, &login_token(&app).await, &["memory"]).await;

    // 造一条画像分面（content 含检索词 zebra3k），evidence_refs 非空
    let pool = sqlx::PgPool::connect(&support::connection_url(&pg).await.unwrap())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO persona_aspects (id, aspect, content, evidence_refs, version)          VALUES ($1, 'preferences', '用户偏好 zebra3k 主题的一切内容', '[{\"session_id\": \"00000000-0000-0000-0000-000000000000\"}]'::jsonb, 1)",
    )
    .bind(uuid::Uuid::now_v7())
    .execute(&pool)
    .await
    .unwrap();

    // 默认：L3 命中不带 evidence_refs
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(1, "memory", "search", json!({"query": "zebra3k"})),
    )
    .await;
    let r = out_json(&v, "search default");
    let l3 = r["l3"].as_array().expect("l3 数组");
    assert!(!l3.is_empty(), "L3 应命中画像分面：{r}");
    assert!(
        l3[0].get("evidence_refs").is_none(),
        "默认不应携带 evidence_refs：{}",
        l3[0]
    );

    // include_evidence=true：显式携带
    let (_, v) = mcp_rpc(
        &app,
        &key,
        call(
            2,
            "memory",
            "search",
            json!({"query": "zebra3k", "include_evidence": true}),
        ),
    )
    .await;
    let r = out_json(&v, "search include_evidence");
    let l3 = r["l3"].as_array().expect("l3 数组");
    assert!(
        l3[0].get("evidence_refs").is_some(),
        "显式开启应携带溯源：{}",
        l3[0]
    );
}

/// Web 前端「恢复」按钮的 HTTP 通道：POST /memory/sessions/{id}/restore——
/// void 后恢复回原状态、级联归档的原子还原；非 void 会话恢复 400。
#[tokio::test]
async fn http_restore_endpoint_reverts_void() {
    let (app, pg) = app().await;
    let admin = login_token(&app).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/memory/sessions")
                .header("authorization", format!("Bearer {admin}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"distill": "off", "turns": [{"speaker": "user", "text": "http 恢复通道"}]})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let session: Value = serde_json::from_slice(&body).unwrap();
    let sid = session["id"].as_str().unwrap().to_string();

    let pool = sqlx::PgPool::connect(&support::connection_url(&pg).await.unwrap())
        .await
        .unwrap();
    // done + 一条源自该会话的 active 原子（void 级联才有东西可归档/恢复）
    sqlx::query("UPDATE raw_sessions SET distill_status = 'done' WHERE id = $1")
        .bind(sid.parse::<sqlx::types::Uuid>().unwrap())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, status, source_refs)          VALUES ($1, 'fact', 'http-恢复-靶原子', 'active', $2::jsonb)",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(format!(r#"[{{"session_id":"{sid}"}}]"#))
    .execute(&pool)
    .await
    .unwrap();

    let post = |app: &Router, path: String| {
        let app = app.clone();
        let admin = admin.clone();
        async move {
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("authorization", format!("Bearer {admin}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };
    let resp = post(&app, format!("/memory/sessions/{sid}/void")).await;
    assert_eq!(resp.status(), StatusCode::OK, "void 应 200");
    let (archived,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM atoms WHERE status = 'archived'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(archived, 1, "void 应级联归档");

    let resp = post(&app, format!("/memory/sessions/{sid}/restore")).await;
    assert_eq!(resp.status(), StatusCode::OK, "restore 应 200");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let r: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        r["session"]["distill_status"], "done",
        "恢复到作废前状态：{r}"
    );
    assert_eq!(r["restored_atoms"], 1, "{r}");
    let (active,): (i64,) = sqlx::query_as("SELECT count(*) FROM atoms WHERE status = 'active'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(active, 1, "原子应回到 active");

    // 重复恢复：非 void 状态 → 400
    let resp = post(&app, format!("/memory/sessions/{sid}/restore")).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "重复恢复应 400");
}

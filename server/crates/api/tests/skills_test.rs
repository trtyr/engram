//! 技能域集成测试：CRUD、slug 唯一/校验、搜索过滤、批量导入（frontmatter 解析 + 冲突）、
//! 导出、版本快照与回滚、scope 门控。

mod support;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use engram_api::routes;
use engram_api::state::AppState;
use serde_json::{Value, json};
use tower::util::ServiceExt;

async fn app() -> (Router, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    let state = AppState::new(pool)
        .with_admin_password(Some("test-admin-pw".into()))
        .with_master_key(Some("ab".repeat(32)));
    (routes::router(state), container)
}

async fn login_token(app: &Router) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"password":"test-admin-pw"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    v["token"].as_str().unwrap().to_string()
}

async fn create_key(app: &Router, token: &str, scopes: &[&str]) -> String {
    let scopes_json = json!(scopes);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/settings/api-keys")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(format!(
                    r#"{{"name":"skills-test","scopes":{scopes_json}}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    v["key"].as_str().unwrap().to_string()
}

/// 发 JSON 请求（body 为 None 时无请求体），返回 (status, json)。
async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    let body = match body {
        Some(b) => {
            builder = builder.header("content-type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let resp = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}

fn sample_create(slug: &str) -> Value {
    json!({
        "slug": slug,
        "name": "示例技能",
        "description": "一段示例描述",
        "content": "# 步骤\n1. 做 A\n2. 做 B",
        "tags": ["demo", "rust"]
    })
}

#[tokio::test]
async fn skills_crud_flow() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    // 新建（201，回读全字段）
    let (st, v) = send(
        &app,
        "POST",
        "/skills",
        &admin,
        Some(sample_create("review-pr")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(v["slug"], "review-pr");
    assert_eq!(v["enabled"], json!(true));
    assert_eq!(v["source"], "manual");
    assert_eq!(v["tags"], json!(["demo", "rust"]));
    assert!(v["content"].as_str().unwrap().contains("做 A"));
    let id = v["id"].as_str().unwrap().to_string();

    // 详情（含正文）
    let (st, v) = send(&app, "GET", "/skills/review-pr", &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["id"], json!(id));

    // 列表是摘要（不含正文键）
    let (st, v) = send(&app, "GET", "/skills", &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].get("content").is_none(), "列表不应带正文：{rows:?}");

    // 更新（改正文 + 描述）
    let (st, v) = send(
        &app,
        "PUT",
        "/skills/review-pr",
        &admin,
        Some(json!({"description": "更新后的描述", "content": "# 新步骤"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["description"], "更新后的描述");
    assert!(v["content"].as_str().unwrap().contains("新步骤"));

    // 删除 → 204 → 404
    let (st, v) = send(&app, "DELETE", "/skills/review-pr", &admin, None).await;
    assert_eq!(st, StatusCode::NO_CONTENT, "{v}");
    let (st, v) = send(&app, "GET", "/skills/review-pr", &admin, None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert!(v["error"]["message"].as_str().unwrap().contains("不存在"));
}

#[tokio::test]
async fn skills_slug_conflict_and_validation() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    send(
        &app,
        "POST",
        "/skills",
        &admin,
        Some(sample_create("taken")),
    )
    .await;

    // 同 slug → 409
    let (st, v) = send(
        &app,
        "POST",
        "/skills",
        &admin,
        Some(sample_create("taken")),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("已被占用"));

    // 非法 slug → 400（大写/空格/超长）
    for bad in ["Has Space", "-lead", "中文", &"a".repeat(81)] {
        let mut body = sample_create("ok-slug");
        body["slug"] = json!(bad);
        let (st, v) = send(&app, "POST", "/skills", &admin, Some(body)).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "slug {bad:?} 应 400：{v}");
    }

    // 空名字 → 400
    let (st, _) = send(&app, "POST", "/skills", &admin, Some(json!({"name": "  "}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // 不传 slug：ASCII 名自动推导
    let (st, v) = send(
        &app,
        "POST",
        "/skills",
        &admin,
        Some(json!({"name": "Auto Slug One"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(v["slug"], "auto-slug-one");

    // 不传 slug：中文名推导失败 → 400 且提示显式传 slug
    let (st, v) = send(
        &app,
        "POST",
        "/skills",
        &admin,
        Some(json!({"name": "中文技能"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("slug"));

    // D25：混 ASCII 前缀的中文名不再静默坍缩到前缀 slug（曾把 "ZZTEST 中文" 推导成 "zztest"）
    let (st, v) = send(
        &app,
        "POST",
        "/skills",
        &admin,
        Some(json!({"name": "ZZTEST 中文名技能", "content": "x"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "混 ASCII 中文名应被拒：{v}");
    assert!(
        v["error"]["message"].as_str().unwrap().contains("非 ASCII"),
        "报错应说明非 ASCII 原因：{v}"
    );

    // 404：改/删不存在的技能
    let (st, _) = send(
        &app,
        "PUT",
        "/skills/ghost",
        &admin,
        Some(json!({"name": "x"})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = send(&app, "DELETE", "/skills/ghost", &admin, None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn skills_search_filter_and_enabled() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    send(
        &app,
        "POST",
        "/skills",
        &admin,
        Some(sample_create("alpha-one")),
    )
    .await;
    let mut beta = sample_create("beta-two");
    beta["name"] = json!("审查技能");
    beta["description"] = json!("审查 Rust PR 用");
    beta["tags"] = json!(["review"]);
    send(&app, "POST", "/skills", &admin, Some(beta)).await;

    // q 搜描述
    let (st, v) = send(&app, "GET", "/skills?q=示例", &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["slug"], "alpha-one");

    // q 搜中文名
    let (st, v) = send(&app, "GET", "/skills?q=审查", &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["slug"], "beta-two");

    // tag 过滤
    let (st, v) = send(&app, "GET", "/skills?tag=review", &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["slug"], "beta-two");

    // 停用 beta-two 后 enabled=true 查不到、enabled=false 查得到
    let (st, _) = send(
        &app,
        "PUT",
        "/skills/beta-two",
        &admin,
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, v) = send(&app, "GET", "/skills?enabled=true", &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        v.as_array().unwrap().len(),
        1,
        "停用后 enabled=true 只剩 alpha：{v}"
    );
    let (st, v) = send(&app, "GET", "/skills?enabled=false", &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v[0]["slug"], "beta-two");
}

#[tokio::test]
async fn skills_import_batch_and_conflicts() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    // 先占一个 slug（测冲突条目）
    send(
        &app,
        "POST",
        "/skills",
        &admin,
        Some(sample_create("existing")),
    )
    .await;

    let docs = json!({
        "documents": [
            {
                "filename": "deploy-check.md",
                "content": "---\nname: Deploy Check\ndescription: 部署前检查\ntags: ops, deploy\n---\n# 清单\n- 健康检查"
            },
            {
                "filename": "existing.md",
                "content": "---\nname: Existing\n---\n# 覆盖前的内容"
            },
            {
                "content": "# 无 frontmatter 也无 filename——名字兜底缺失，该条失败"
            },
            {
                "filename": "only-body.md",
                "content": "# 只有正文\n用 filename 兜底命名"
            }
        ],
        "overwrite": false
    });
    let (st, v) = send(&app, "POST", "/skills/import", &admin, Some(docs)).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["imported"], json!(2));
    assert_eq!(v["failed"], json!(2));
    assert_eq!(v["updated"], json!(0));

    let items = v["items"].as_array().unwrap();
    assert_eq!(items[0]["status"], "imported");
    assert_eq!(items[0]["slug"], "deploy-check");
    assert_eq!(items[1]["status"], "failed");
    assert!(
        items[1]["error"].as_str().unwrap().contains("已存在"),
        "overwrite=false 冲突应提示已存在：{items:?}"
    );
    assert_eq!(items[2]["status"], "failed");
    assert!(items[2]["error"].as_str().unwrap().contains("名字"));
    // 第 4 条：filename 兜底命名 → slug only-body
    assert_eq!(items[3]["status"], "imported");
    assert_eq!(items[3]["slug"], "only-body");

    // 导入的技能正文不含 frontmatter，元数据来自 frontmatter
    let (st, v) = send(&app, "GET", "/skills/deploy-check", &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["name"], "Deploy Check");
    assert_eq!(v["tags"], json!(["ops", "deploy"]));
    assert_eq!(v["source"], "import");
    assert!(!v["content"].as_str().unwrap().contains("---"));

    // overwrite=true：同 slug 再导 → updated，且旧版本进快照
    let docs = json!({
        "documents": [
            {"filename": "existing.md", "content": "---\nname: Existing\ndescription: 覆盖版\n---\n# 覆盖后的内容"}
        ],
        "overwrite": true
    });
    let (st, v) = send(&app, "POST", "/skills/import", &admin, Some(docs)).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["updated"], json!(1));
    let (st, v) = send(&app, "GET", "/skills/existing", &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(v["content"].as_str().unwrap().contains("覆盖后的内容"));

    // 空批次
    let (st, v) = send(
        &app,
        "POST",
        "/skills/import",
        &admin,
        Some(json!({"documents": []})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["imported"], json!(0));
}

#[tokio::test]
async fn skills_export_contains_full_content() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    send(
        &app,
        "POST",
        "/skills",
        &admin,
        Some(sample_create("exp-b")),
    )
    .await;
    send(
        &app,
        "POST",
        "/skills",
        &admin,
        Some(sample_create("exp-a")),
    )
    .await;

    let (st, v) = send(&app, "GET", "/skills/export", &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    // 按 slug 排序
    assert_eq!(rows[0]["slug"], "exp-a");
    assert_eq!(rows[1]["slug"], "exp-b");
    // 导出含正文（与列表摘要的区别）
    assert!(rows[0]["content"].as_str().unwrap().contains("做 A"));
}

#[tokio::test]
async fn skills_revisions_snapshot_and_restore() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    // 创建 → rev1（origin=create，初始态）
    send(
        &app,
        "POST",
        "/skills",
        &admin,
        Some(sample_create("rev-test")),
    )
    .await;
    let (st, v) = send(&app, "GET", "/skills/rev-test/revisions", &admin, None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let revs = v.as_array().unwrap();
    assert_eq!(revs.len(), 1);
    assert_eq!(revs[0]["rev"], json!(1));
    assert_eq!(revs[0]["origin"], "create");
    assert!(revs[0]["content"].as_str().unwrap().contains("做 A"));
    let rev1 = revs[0]["id"].as_str().unwrap().to_string();

    // 第一次更新 → rev2 = 更新前快照
    send(
        &app,
        "PUT",
        "/skills/rev-test",
        &admin,
        Some(json!({"content": "# 第二版"})),
    )
    .await;
    // 第二次更新 → rev3 = 第一版状态
    send(
        &app,
        "PUT",
        "/skills/rev-test",
        &admin,
        Some(json!({"content": "# 第三版"})),
    )
    .await;

    // enabled-only 更新不产生新快照
    send(
        &app,
        "PUT",
        "/skills/rev-test",
        &admin,
        Some(json!({"enabled": false})),
    )
    .await;
    let (st, v) = send(&app, "GET", "/skills/rev-test/revisions", &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    let revs = v.as_array().unwrap();
    assert_eq!(revs.len(), 3, "enabled-only 不留版本：{v}");
    assert_eq!(revs[0]["rev"], json!(3), "新→旧排序");

    // 回滚到 rev1（origin=create）→ 正文回初始态，且回滚前现状先留 restore 快照
    let (st, v) = send(
        &app,
        "POST",
        &format!("/skills/rev-test/revisions/{rev1}/restore"),
        &admin,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(
        v["content"].as_str().unwrap().contains("做 A"),
        "应回到初始正文：{v}"
    );

    let (st, v) = send(&app, "GET", "/skills/rev-test/revisions", &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    let revs = v.as_array().unwrap();
    assert_eq!(revs.len(), 4);
    assert_eq!(revs[0]["origin"], "restore", "回滚前现状应先快照");

    // 回滚不存在的版本 → 404
    let (st, _) = send(
        &app,
        "POST",
        &format!(
            "/skills/rev-test/revisions/{}/restore",
            uuid::Uuid::now_v7()
        ),
        &admin,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // 删除技能 → 版本级联消失（表清空）
    send(&app, "DELETE", "/skills/rev-test", &admin, None).await;
    // 重建同名 slug 后旧版本不应复活
    let (st, v) = send(
        &app,
        "POST",
        "/skills",
        &admin,
        Some(sample_create("rev-test")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let (st, v) = send(&app, "GET", "/skills/rev-test/revisions", &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 1, "级联删除后旧版本不应复活");
}

#[tokio::test]
async fn skills_scope_required() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    // 无 skills scope 的 key → 403
    let mem_key = create_key(&app, &admin, &["memory"]).await;
    let (st, v) = send(&app, "GET", "/skills", &mem_key, None).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("skills"));

    // skills scope 的 key → 200 且能写
    let skills_key = create_key(&app, &admin, &["skills"]).await;
    let (st, _) = send(&app, "GET", "/skills", &skills_key, None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = send(
        &app,
        "POST",
        "/skills",
        &skills_key,
        Some(sample_create("key-made")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // 未知 scope 的 key 无法签发
    let (st, _) = send(
        &app,
        "POST",
        "/settings/api-keys",
        &admin,
        Some(json!({"name": "bad", "scopes": ["skillz"]})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn skills_spa_nav_serves_html() {
    let (app, _pg) = app().await;

    // 浏览器导航（Accept: text/html）到 /skills → SPA 页（不 401/404）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/skills")
                .header("accept", "text/html")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "SPA 导航应回页");
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        content_type.contains("text/html"),
        "应回 HTML：{content_type}"
    );
}

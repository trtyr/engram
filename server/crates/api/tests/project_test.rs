//! 项目记忆域集成测试：三表 CRUD、类型筛选/模板、批量删除、多主机位置、分类文档、scope 门控。

mod support;

use engram_api::routes;
use engram_api::state::AppState;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;

async fn app() -> (Router, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool)
        .await
        .expect("迁移");
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
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    v["token"].as_str().unwrap().to_string()
}

/// 发 JSON 请求（body 为 None 时无请求体），返回 (status, json)。
async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
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
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

#[tokio::test]
async fn project_crud_flow() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    // 新建（type=dev → 默认四分类）
    let (st, v) = send(
        &app,
        "POST",
        "/projects",
        &admin,
        Some(serde_json::json!({"name":"engram","type":"dev"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    assert_eq!(v["type"], "dev");
    assert_eq!(
        v["categories"],
        serde_json::json!(["后端", "前端", "测试", "规划"]),
        "dev 类型默认四分类"
    );

    // 列表
    let (st, v) = send(&app, "GET", "/projects", &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 1);

    // 详情
    let (st, v) = send(&app, "GET", &format!("/projects/{id}"), &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["name"], "engram");
    assert!(v["locations"].as_array().unwrap().is_empty());
    assert!(v["docs"].as_array().unwrap().is_empty());

    // 编辑（改状态 + 分类增删）
    let (st, v) = send(
        &app,
        "PUT",
        &format!("/projects/{id}"),
        &admin,
        Some(serde_json::json!({
            "name":"engram",
            "status":"done",
            "description":"改过的描述",
            "categories":["后端","前端","运维"]
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "done");
    assert_eq!(v["categories"], serde_json::json!(["后端", "前端", "运维"]));

    // 删除
    let (st, _) = send(&app, "DELETE", &format!("/projects/{id}"), &admin, None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // 删后 404
    let (st, _) = send(&app, "GET", &format!("/projects/{id}"), &admin, None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn project_type_template_and_filter() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    // 类型模板：dev 4 分类 + research 6 分类
    let (st, v) = send(&app, "GET", "/projects/types", &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    let types = v.as_array().unwrap();
    assert_eq!(types.len(), 2);
    let dev = types.iter().find(|t| t["type"] == "dev").unwrap();
    assert_eq!(dev["label"], "开发");
    assert_eq!(dev["default_categories"].as_array().unwrap().len(), 4);
    let research = types.iter().find(|t| t["type"] == "research").unwrap();
    assert_eq!(research["label"], "调研");
    assert_eq!(research["default_categories"].as_array().unwrap().len(), 6);

    // 建 dev + research 各一
    send(
        &app,
        "POST",
        "/projects",
        &admin,
        Some(serde_json::json!({"name":"p-dev","type":"dev"})),
    )
    .await;
    send(
        &app,
        "POST",
        "/projects",
        &admin,
        Some(serde_json::json!({"name":"p-res","type":"research"})),
    )
    .await;

    // 类型筛选
    let (_, v) = send(&app, "GET", "/projects?type=dev", &admin, None).await;
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "p-dev");

    let (_, v) = send(&app, "GET", "/projects?type=research", &admin, None).await;
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "p-res");
}

#[tokio::test]
async fn project_batch_delete() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    let mut ids = vec![];
    for i in 0..3 {
        let (_, v) = send(
            &app,
            "POST",
            "/projects",
            &admin,
            Some(serde_json::json!({"name":format!("p{i}"),"type":"dev"})),
        )
        .await;
        ids.push(v["id"].as_str().unwrap().to_string());
    }

    let (st, v) = send(
        &app,
        "POST",
        "/projects/batch-delete",
        &admin,
        Some(serde_json::json!({"ids": ids})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["deleted"], 3);
    assert_eq!(v["failed"].as_array().unwrap().len(), 0);

    let (_, v) = send(&app, "GET", "/projects", &admin, None).await;
    assert!(v.as_array().unwrap().is_empty());

    // 含不存在 id → 报 failed
    let bogus = uuid::Uuid::now_v7().to_string();
    let (st, v) = send(
        &app,
        "POST",
        "/projects/batch-delete",
        &admin,
        Some(serde_json::json!({"ids": [bogus]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["deleted"], 0);
    assert_eq!(v["failed"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn project_locations_flow() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    let (_, v) = send(
        &app,
        "POST",
        "/projects",
        &admin,
        Some(serde_json::json!({"name":"am","type":"dev"})),
    )
    .await;
    let id = v["id"].as_str().unwrap().to_string();

    // 加位置（多主机登记）
    let (st, v) = send(
        &app,
        "POST",
        &format!("/projects/{id}/locations"),
        &admin,
        Some(serde_json::json!({"ip":"192.168.1.5","host":"MacBook Pro","os":"macOS 15","path":"~/Documents/Code/Rust/engram","purpose":"开发"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let loc_id = v["id"].as_str().unwrap().to_string();
    assert_eq!(v["ip"], "192.168.1.5");
    assert_eq!(v["host"], "MacBook Pro");
    assert_eq!(v["os"], "macOS 15");

    // 详情里可见位置
    let (_, v) = send(&app, "GET", &format!("/projects/{id}"), &admin, None).await;
    assert_eq!(v["locations"].as_array().unwrap().len(), 1);

    // 位置单读
    let (st, v) = send(
        &app,
        "GET",
        &format!("/projects/{id}/locations/{loc_id}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["host"], "MacBook Pro");

    // 改位置
    let (st, v) = send(
        &app,
        "PUT",
        &format!("/projects/{id}/locations/{loc_id}"),
        &admin,
        Some(serde_json::json!({"ip":"82.157.147.224","host":"tencent-beijing","os":"Ubuntu 22.04","path":"/srv/am","purpose":"部署"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["ip"], "82.157.147.224");
    assert_eq!(v["host"], "tencent-beijing");
    assert_eq!(v["os"], "Ubuntu 22.04");

    // 删位置
    let (st, _) = send(
        &app,
        "DELETE",
        &format!("/projects/{id}/locations/{loc_id}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn project_docs_flow() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    let (_, v) = send(
        &app,
        "POST",
        "/projects",
        &admin,
        Some(serde_json::json!({"name":"am","type":"dev"})),
    )
    .await;
    let id = v["id"].as_str().unwrap().to_string();

    // 加文档（分类）
    let (st, v) = send(
        &app,
        "POST",
        &format!("/projects/{id}/docs"),
        &admin,
        Some(serde_json::json!({"category":"后端","title":"api.md","content":"# API 设计"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let doc_id = v["id"].as_str().unwrap().to_string();
    assert_eq!(v["category"], "后端");

    // 读单个文档
    let (st, v) = send(
        &app,
        "GET",
        &format!("/projects/{id}/docs/{doc_id}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["content"], "# API 设计");

    // 改文档
    let (st, v) = send(
        &app,
        "PUT",
        &format!("/projects/{id}/docs/{doc_id}"),
        &admin,
        Some(serde_json::json!({"category":"后端","title":"api.md","content":"# API 设计 v2"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["content"], "# API 设计 v2");

    // 删文档
    let (st, _) = send(
        &app,
        "DELETE",
        &format!("/projects/{id}/docs/{doc_id}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn project_name_unique_conflict() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    let (st, _) = send(
        &app,
        "POST",
        "/projects",
        &admin,
        Some(serde_json::json!({"name":"dup","type":"dev"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // 同名项目 → 409
    let (st, v) = send(
        &app,
        "POST",
        "/projects",
        &admin,
        Some(serde_json::json!({"name":"dup","type":"research"})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("已存在"));
}

#[tokio::test]
async fn project_doc_title_unique_conflict() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    let (_, v) = send(
        &app,
        "POST",
        "/projects",
        &admin,
        Some(serde_json::json!({"name":"docdup","type":"dev"})),
    )
    .await;
    let id = v["id"].as_str().unwrap().to_string();

    let (st, _) = send(
        &app,
        "POST",
        &format!("/projects/{id}/docs"),
        &admin,
        Some(serde_json::json!({"category":"后端","title":"api.md","content":"# v1"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // 同项目同分类重复 title → 409
    let (st, v) = send(
        &app,
        "POST",
        &format!("/projects/{id}/docs"),
        &admin,
        Some(serde_json::json!({"category":"后端","title":"api.md","content":"# v2"})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("已存在"));

    // 跨分类同名 → 允许
    let (st, _) = send(
        &app,
        "POST",
        &format!("/projects/{id}/docs"),
        &admin,
        Some(serde_json::json!({"category":"前端","title":"api.md","content":"# 前端"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
}

#[tokio::test]
async fn project_scope_required() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    // 无 project scope 的 key → 403
    let key = {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/settings/api-keys")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {admin}"))
                    .body(Body::from(r#"{"name":"no-project","scopes":["memory"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        v["key"].as_str().unwrap().to_string()
    };

    let (st, _) = send(&app, "GET", "/projects", &key, None).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "无 project scope 应 403");

    // 有 project scope 的 key → 200
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/settings/api-keys")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {admin}"))
                .body(Body::from(
                    r#"{"name":"with-project","scopes":["project"]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let pkey = v["key"].as_str().unwrap().to_string();

    let (st, _) = send(&app, "GET", "/projects", &pkey, None).await;
    assert_eq!(st, StatusCode::OK, "有 project scope 应 200");
}

/// 审计发现：浏览器硬刷新 /projects（Accept: text/html）曾命中 API 处理器返回 401 JSON。
/// 修复后走 SPA 分流（回 index.html），不再是认证墙。
#[tokio::test]
async fn projects_spa_navigation_bypasses_auth() {
    let (app, _pg) = app().await;
    // 浏览器导航（Accept: text/html）→ 回 SPA（200 若 web/dist 嵌入，404 若未构建），绝不应 401
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/projects")
                .header("accept", "text/html")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "SPA 导航不应命中 401 认证墙"
    );
    // 详情页深链同理
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/projects/00000000-0000-0000-0000-000000000000")
                .header("accept", "text/html")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// 改名撞名 → 409（修复前会冒成 500 存储错误）；空名 → 400。
#[tokio::test]
async fn project_rename_conflict_and_empty_name() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    let (st, a) = send(&app, "POST", "/projects", &admin, Some(serde_json::json!({"name":"改名-甲","type":"dev"}))).await;
    assert_eq!(st, StatusCode::CREATED, "{a}");
    let (st, b) = send(&app, "POST", "/projects", &admin, Some(serde_json::json!({"name":"改名-乙","type":"dev"}))).await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    let b_id = b["id"].as_str().unwrap();

    // 改成已有项目名 → 409，不是 500
    let (st, v) = send(
        &app,
        "PUT",
        &format!("/projects/{b_id}"),
        &admin,
        Some(serde_json::json!({
            "name":"改名-甲","status":"active","description":null,
            "categories":["后端","前端","测试","规划"]
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "改名撞名应 409：{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("已被项目"));

    // 空名 / 纯空白名 → 400
    for bad in ["", "   "] {
        let (st, v) = send(
            &app,
            "PUT",
            &format!("/projects/{b_id}"),
            &admin,
            Some(serde_json::json!({
                "name":bad,"status":"active","description":null,
                "categories":["后端"]
            })),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "空名应 400：{v}");
        let (st, _) = send(&app, "POST", "/projects", &admin, Some(serde_json::json!({"name":bad,"type":"dev"}))).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "空名建项目应 400");
    }

    // 改回自己原名（id 相同）不算撞名
    let (st, v) = send(
        &app,
        "PUT",
        &format!("/projects/{b_id}"),
        &admin,
        Some(serde_json::json!({
            "name":"改名-乙","status":"paused","description":"ok",
            "categories":["后端","前端","测试","规划"]
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "原名写回不应 409：{v}");
    assert_eq!(v["status"], "paused");
}

/// 归属校验：用项目乙的路径寻址项目甲的位置/文档，get/update/delete 一律 404 且资源无损。
#[tokio::test]
async fn project_cross_project_access_blocked() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    let (st, a) = send(&app, "POST", "/projects", &admin, Some(serde_json::json!({"name":"归属-甲","type":"dev"}))).await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, b) = send(&app, "POST", "/projects", &admin, Some(serde_json::json!({"name":"归属-乙","type":"dev"}))).await;
    assert_eq!(st, StatusCode::CREATED);
    let (a_id, b_id) = (a["id"].as_str().unwrap(), b["id"].as_str().unwrap());

    // 甲挂位置和文档
    let (st, loc) = send(
        &app, "POST", &format!("/projects/{a_id}/locations"), &admin,
        Some(serde_json::json!({"ip":"127.0.0.1","host":"h","os":"macOS","path":"/a","purpose":null})),
    ).await;
    assert_eq!(st, StatusCode::CREATED, "{loc}");
    let loc_id = loc["id"].as_str().unwrap();
    let (st, doc) = send(
        &app, "POST", &format!("/projects/{a_id}/docs"), &admin,
        Some(serde_json::json!({"category":"后端","title":"甲的文档","content":"v1"})),
    ).await;
    assert_eq!(st, StatusCode::CREATED, "{doc}");
    let doc_id = doc["id"].as_str().unwrap();

    // 乙的路径 + 甲的资源 id → 全部 404
    for (method, uri) in [
        ("GET", format!("/projects/{b_id}/locations/{loc_id}")),
        ("GET", format!("/projects/{b_id}/docs/{doc_id}")),
    ] {
        let (st, _) = send(&app, method, &uri, &admin, None).await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{method} {uri} 应 404");
    }
    let (st, _) = send(
        &app, "PUT", &format!("/projects/{b_id}/locations/{loc_id}"), &admin,
        Some(serde_json::json!({"ip":"0.0.0.0","host":"x","os":"x","path":"/x","purpose":null})),
    ).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "跨项目改位置应 404");
    let (st, _) = send(
        &app, "PUT", &format!("/projects/{b_id}/docs/{doc_id}"), &admin,
        Some(serde_json::json!({"category":"后端","title":"hijack","content":"hijack"})),
    ).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "跨项目改文档应 404");
    let (st, _) = send(&app, "DELETE", &format!("/projects/{b_id}/locations/{loc_id}"), &admin, None).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "跨项目删位置应 404");
    let (st, _) = send(&app, "DELETE", &format!("/projects/{b_id}/docs/{doc_id}"), &admin, None).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "跨项目删文档应 404");

    // 资源无损：内容未被篡改、仍在甲名下
    let (st, doc_now) = send(&app, "GET", &format!("/projects/{a_id}/docs/{doc_id}"), &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(doc_now["content"], "v1", "被拦截的跨项目写不应生效");
    assert_eq!(doc_now["title"], "甲的文档");
    let (st, loc_now) = send(&app, "GET", &format!("/projects/{a_id}/locations/{loc_id}"), &admin, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(loc_now["path"], "/a");

    // 正路仍通：甲的路径 + 甲的资源
    let (st, _) = send(&app, "DELETE", &format!("/projects/{a_id}/locations/{loc_id}"), &admin, None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
}

/// 分类校验：add 到未登记分类 400；换到未登记分类 400；
/// 分类被移除后存量文档可原地编辑（保留现分类豁免）；换到已登记分类可迁移。
#[tokio::test]
async fn project_doc_category_validation() {
    let (app, _pg) = app().await;
    let admin = login_token(&app).await;

    let (st, p) = send(&app, "POST", "/projects", &admin, Some(serde_json::json!({"name":"分类校验","type":"dev"}))).await;
    assert_eq!(st, StatusCode::CREATED);
    let pid = p["id"].as_str().unwrap();

    // add 到未登记分类 → 400，错误列出现有分类
    let (st, v) = send(
        &app, "POST", &format!("/projects/{pid}/docs"), &admin,
        Some(serde_json::json!({"category":"后段","title":"t","content":""})),
    ).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("后端"), "应列出现有分类：{v}");

    let (st, doc) = send(
        &app, "POST", &format!("/projects/{pid}/docs"), &admin,
        Some(serde_json::json!({"category":"后端","title":"存量文档","content":"v1"})),
    ).await;
    assert_eq!(st, StatusCode::CREATED, "{doc}");
    let doc_id = doc["id"].as_str().unwrap();

    // 把「后端」从项目分类里移除
    let (st, _) = send(
        &app, "PUT", &format!("/projects/{pid}"), &admin,
        Some(serde_json::json!({"name":"分类校验","status":"active","description":null,"categories":["前端","测试","规划"]})),
    ).await;
    assert_eq!(st, StatusCode::OK);

    // 存量文档原地编辑（分类不变）→ 放行
    let (st, v) = send(
        &app, "PUT", &format!("/projects/{pid}/docs/{doc_id}"), &admin,
        Some(serde_json::json!({"category":"后端","title":"存量文档","content":"v2"})),
    ).await;
    assert_eq!(st, StatusCode::OK, "保留现分类的原地编辑不应被拦：{v}");
    assert_eq!(v["content"], "v2");

    // 换到未登记分类 → 400
    let (st, v) = send(
        &app, "PUT", &format!("/projects/{pid}/docs/{doc_id}"), &admin,
        Some(serde_json::json!({"category":"瞎写","title":"存量文档","content":"v2"})),
    ).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");

    // 把「后端」加回，再迁移 → 放行
    let (st, _) = send(
        &app, "PUT", &format!("/projects/{pid}"), &admin,
        Some(serde_json::json!({"name":"分类校验","status":"active","description":null,"categories":["后端","前端","测试","规划"]})),
    ).await;
    assert_eq!(st, StatusCode::OK);
    let (st, v) = send(
        &app, "PUT", &format!("/projects/{pid}/docs/{doc_id}"), &admin,
        Some(serde_json::json!({"category":"测试","title":"存量文档","content":"v3"})),
    ).await;
    assert_eq!(st, StatusCode::OK, "迁移到已登记分类应放行：{v}");
    assert_eq!(v["category"], "测试");
}

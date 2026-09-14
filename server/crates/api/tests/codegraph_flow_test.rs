//! CodeGraph 域集成测试：注册查重 / 删除 / CLI status / 索引入队（202）/ 图未就绪语义。
//! 不依赖真实 codegraph CLI（status 的 available 两态皆合法）。

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;

use crate::support::{create_key, login_token};

async fn key(app: &axum::Router) -> String {
    let admin = login_token(app).await;
    create_key(app, &admin, &["codegraph"]).await
}

#[tokio::test]
async fn register_dedup_and_delete_flow() {
    let (app, _pg) = support::app().await;
    let k = key(&app).await;
    // 本地路径注册要求路径存在——用真实 tempdir
    let tmp = tempfile::tempdir().unwrap();
    let uri = tmp.path().to_string_lossy().replace('\\', "/");

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/codegraph/projects")
                .header("authorization", format!("Bearer {k}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"name":"demo","source_uri":uri}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "注册应 201");
    let v: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let id = v["id"].as_str().unwrap().to_string();

    // 同名 400
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/codegraph/projects")
                .header("authorization", format!("Bearer {k}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"name":"demo","source_uri":"D:/another"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "同名应 400");

    // 注册重复名会先撞名字，同源查重用第二个名字验证
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/codegraph/projects")
                .header("authorization", format!("Bearer {k}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"name":"demo2","source_uri":uri}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "同源应 400");

    // 未就绪项目 graph → 400（不是 404）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/codegraph/projects/{id}/graph?symbol=foo"))
                .header("authorization", format!("Bearer {k}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "未就绪 graph 应 400"
    );

    // 删除 → 200 + workdir_removed；再删 → 404
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/codegraph/projects/{id}"))
                .header("authorization", format!("Bearer {k}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(v["deleted"].as_str().unwrap(), id);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/codegraph/projects/{id}"))
                .header("authorization", format!("Bearer {k}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND, "重复删除应 404");
}

#[tokio::test]
async fn index_enqueues_job_returns_202() {
    let (app, _pg) = support::app().await;
    let k = key(&app).await;

    // 本地路径注册（路径存在性校验需要一个真目录——用 tempdir）
    let tmp = tempfile::tempdir().unwrap();
    let uri = tmp.path().to_string_lossy().replace('\\', "/");
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/codegraph/projects")
                .header("authorization", format!("Bearer {k}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"name":"queued","source_uri":uri}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let id = v["id"].as_str().unwrap().to_string();

    // 建索引：202 + job_id（异步——不等待 CLI，无 CLI 环境下 job 会失败但入队本身成功）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/codegraph/projects/{id}/index"))
                .header("authorization", format!("Bearer {k}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED, "入队应 202");
    let v: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(v["job_id"].as_str().is_some(), "应返回 job_id");
    assert_eq!(v["kind"], "cg_index");

    // 不存在的项目入队 → 404（先查后入队）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/codegraph/projects/{}/index",
                    uuid::Uuid::now_v7()
                ))
                .header("authorization", format!("Bearer {k}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cli_status_shape_is_stable() {
    let (app, _pg) = support::app().await;
    let k = key(&app).await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/codegraph/status")
                .header("authorization", format!("Bearer {k}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    // 两态皆合法（有无 CLI 的环境都要结构稳定）
    assert!(v["available"].is_boolean(), "available 应为 bool: {v}");
    assert!(v["pin"].as_str().is_some(), "pin 应为字符串: {v}");
}

/// EN-26 版本诚实：codegraph list 每项带 freshness，两种形态——
/// ① .git 可读 → stale 布尔 + snapshot_head；② 路径失效 → stale=null（不虚报）。
#[tokio::test]
async fn list_freshness_two_shapes() {
    let (app, _pg) = support::app().await;
    let k = key(&app).await;
    let post = |app: &axum::Router, k: &str, name: &str, uri: String| {
        let app = app.clone();
        let k = k.to_string();
        let name = name.to_string();
        async move {
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/codegraph/projects")
                    .header("authorization", format!("Bearer {k}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"name": name, "source_uri": uri}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };

    // ① 真实 temp git repo（可读 .git）
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("r");
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
    let resp = post(
        &app,
        &k,
        "fresh-live",
        repo.to_string_lossy().replace('\\', "/"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    // 模拟「已索引」：写入 stats（head 快照戳 + last_indexed）——HTTP 测试环境无真 CLI
    {
        let url = support::connection_url(&_pg).await.unwrap();
        let pool = support::connect_with_retry(&url).await.unwrap();
        let head = String::from_utf8_lossy(&git(&["rev-parse", "HEAD"]).stdout)
            .trim()
            .to_string();
        sqlx::query("UPDATE cg_projects SET stats = $1 WHERE name = 'fresh-live'")
            .bind(serde_json::json!({
                "files": 1, "symbols": 1, "edges": 0, "by_kind": {},
                "last_indexed": chrono::Utc::now().to_rfc3339(),
                "head": head,
            }))
            .execute(&pool)
            .await
            .unwrap();
    }

    // ② 注册后摘走目录（不可读）
    let tmp2 = tempfile::tempdir().unwrap();
    let ghost = tmp2.path().join("ghost");
    std::fs::create_dir_all(&ghost).unwrap();
    let resp = post(
        &app,
        &k,
        "fresh-ghost",
        ghost.to_string_lossy().replace('\\', "/"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    std::fs::remove_dir_all(&ghost).unwrap();

    // list：两项都带 freshness，形态正确
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/codegraph/projects")
                .header("authorization", format!("Bearer {k}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let items = v.as_array().expect("list 应为数组");
    let live = items
        .iter()
        .find(|i| i["name"] == "fresh-live")
        .expect("live 项目在列");
    let f = &live["freshness"];
    assert!(f["stale"].is_boolean(), "可读 .git → stale 布尔: {f}");
    assert!(f["head"].as_str().is_some(), "{f}");

    let ghost_item = items
        .iter()
        .find(|i| i["name"] == "fresh-ghost")
        .expect("ghost 项目在列");
    let f = &ghost_item["freshness"];
    assert!(f["stale"].is_null(), "路径失效 → stale=null 不虚报: {f}");
    assert!(f["hint"].as_str().is_some(), "应有提示: {f}");
}

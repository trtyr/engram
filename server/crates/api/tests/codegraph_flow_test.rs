//! CodeGraph 域集成测试：注册收口（URI-only / 默认与自定义落盘 / 一步到位自动建索引）/
//! 删除分流（默认落盘连目录清、自定义落盘目录保留）/ CLI status / 索引入队（202）/ 图未就绪语义。
//!
//! 夹具：真 git 仓库（`file://` URL）——clone 与新鲜度都要真 git；**落盘测试一律用
//! `app_with_data_dir(tempdir)`**，否则会写进 `./data`（甚至真数据根）。
//! 不依赖真实 codegraph CLI（status 的 available 两态皆合法，index 只验入队）。

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;

use crate::support::{app_with_data_dir, create_key, login_token};

async fn key(app: &axum::Router) -> String {
    let admin = login_token(app).await;
    create_key(app, &admin, &["codegraph"]).await
}

/// 造一个真 git 仓库并给出 `file://` URL（注册入口只收 URI 形态）。
/// 返回 `(仓库路径, file:// URL)`。
fn git_repo(dir: &std::path::Path, name: &str) -> (std::path::PathBuf, String) {
    let repo = dir.join(name);
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
    (repo, url)
}

/// POST /codegraph/projects 的薄封装：返回 (状态码, JSON)。
async fn register(
    app: &axum::Router,
    k: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/codegraph/projects")
                .header("authorization", format!("Bearer {k}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let v: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    (status, v)
}

/// 普通文件系统路径必须被拒，且文案可行动（指引转向「上传产物」入口）。
#[tokio::test]
async fn register_rejects_local_path_and_points_to_upload() {
    let data = tempfile::tempdir().unwrap();
    let (app, _pg) = app_with_data_dir(data.path()).await;
    let k = key(&app).await;

    // 路径真实存在也不行——入口只认 URI 形态
    let real_dir = tempfile::tempdir().unwrap();
    let (status, v) = register(
        &app,
        &k,
        serde_json::json!({"name": "pathy", "source_uri": real_dir.path().to_string_lossy()}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("git 仓库地址"), "应说明只收 git 地址：{msg}");
    assert!(msg.contains("上传"), "应指引改用上传入口：{msg}");
}

/// 注册即 clone 到**默认路径**并**自动入队建索引**（一步到位）。
#[tokio::test]
async fn register_clones_to_default_dest_and_auto_indexes() {
    let data = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let (_repo, url) = git_repo(src.path(), "myrepo");
    let (app, _pg) = app_with_data_dir(data.path()).await;
    let k = key(&app).await;

    let (status, v) = register(
        &app,
        &k,
        serde_json::json!({"name": "demo", "source_uri": url}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let p = &v["project"];
    assert_eq!(
        p["path"].as_str().unwrap(),
        data.path().join("codegraph").join("demo").to_string_lossy(),
        "默认落盘应是 <数据根>/codegraph/<项目名>"
    );
    assert_eq!(p["dest_mode"], "default", "{v}");
    assert_eq!(p["status"], "registered", "clone 后待索引");
    assert!(
        v["index_job_id"].as_str().is_some(),
        "一步到位：应自动入队建索引并回 job_id：{v}"
    );
    assert!(v["warning"].is_null(), "入队成功不应有 warning：{v}");
    // clone 真的落地了（含 .git）
    let cloned = std::path::Path::new(p["path"].as_str().unwrap());
    assert!(cloned.join(".git").exists(), "应真 clone 到落盘目录");
    assert!(cloned.join("a.txt").exists(), "应含仓库文件");

    // 同名再注册 → 400（名字唯一语义不变）
    let (status, v2) = register(
        &app,
        &k,
        serde_json::json!({"name": "demo", "source_uri": url}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v2}");
}

/// 默认路径被占用（同名目录已在盘）→ 自动加后缀 `-2`。
#[tokio::test]
async fn register_default_dest_dedups_with_suffix() {
    let data = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let (_repo, url) = git_repo(src.path(), "myrepo");
    let (app, _pg) = app_with_data_dir(data.path()).await;
    let k = key(&app).await;

    // 先占住 <数据根>/codegraph/demo（模拟历史残留目录）
    let taken = data.path().join("codegraph").join("demo");
    std::fs::create_dir_all(&taken).unwrap();
    std::fs::write(taken.join("keep.txt"), b"x").unwrap();

    let (status, v) = register(
        &app,
        &k,
        serde_json::json!({"name": "demo", "source_uri": url}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let path = v["project"]["path"].as_str().unwrap();
    assert!(path.ends_with("demo-2"), "被占用应加后缀：{path}");
    assert!(std::path::Path::new(path).join(".git").exists());
    // 原目录内容不被碰
    assert!(taken.join("keep.txt").exists());
}

/// 自定义落盘 = `<父目录>/<仓库名>`，落 `dest_mode=custom`。
#[tokio::test]
async fn register_custom_parent_appends_repo_name() {
    let data = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let (_repo, url) = git_repo(src.path(), "myrepo");
    let parent = data.path().join("repos");
    std::fs::create_dir_all(&parent).unwrap();
    let (app, _pg) = app_with_data_dir(data.path()).await;
    let k = key(&app).await;

    let (status, v) = register(
        &app,
        &k,
        serde_json::json!({
            "name": "custom-one",
            "source_uri": url,
            "dest_parent": parent.to_string_lossy(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let p = &v["project"];
    assert_eq!(p["dest_mode"], "custom", "{v}");
    let expect = parent.canonicalize().unwrap().join("myrepo");
    assert_eq!(
        std::path::Path::new(p["path"].as_str().unwrap()),
        expect,
        "自定义落盘应是 <父目录>/<仓库名>"
    );
    assert!(expect.join(".git").exists(), "应真 clone 到自定义目录");
}

/// 自定义父目录**越白名单根**（默认只允许数据根之内）→ 400 且文案给出放开方式。
#[tokio::test]
async fn register_custom_dest_outside_allowlist_rejected() {
    let data = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let (_repo, url) = git_repo(src.path(), "myrepo");
    let (app, _pg) = app_with_data_dir(data.path()).await;
    let k = key(&app).await;

    let (status, v) = register(
        &app,
        &k,
        serde_json::json!({
            "name": "outside",
            "source_uri": url,
            "dest_parent": outside.path().to_string_lossy(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("越界"), "应说明越界：{msg}");
    assert!(
        msg.contains("AGENT_MEMORY_CG_DEST_ROOTS"),
        "应给出放开方式：{msg}"
    );
}

/// 自定义目标目录**已存在且非空** → 400（服务端不覆盖已有目录）。
#[tokio::test]
async fn register_custom_dest_nonempty_target_rejected() {
    let data = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let (_repo, url) = git_repo(src.path(), "myrepo");
    let parent = data.path().join("repos");
    let occupied = parent.join("myrepo");
    std::fs::create_dir_all(&occupied).unwrap();
    std::fs::write(occupied.join("keep.txt"), b"x").unwrap();
    let (app, _pg) = app_with_data_dir(data.path()).await;
    let k = key(&app).await;

    let (status, v) = register(
        &app,
        &k,
        serde_json::json!({
            "name": "occupied",
            "source_uri": url,
            "dest_parent": parent.to_string_lossy(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("非空"), "应说明目标非空：{msg}");
    assert!(occupied.join("keep.txt").exists(), "原目录内容不能被碰");
}

/// clone 失败 → 400 + **不落条目** + **不留半成品目录**。
#[tokio::test]
async fn clone_failure_leaves_no_row_and_no_dir() {
    let data = tempfile::tempdir().unwrap();
    let (app, _pg) = app_with_data_dir(data.path()).await;
    let k = key(&app).await;

    let (status, v) = register(
        &app,
        &k,
        serde_json::json!({
            "name": "ghost-repo",
            "source_uri": "file:///definitely/not/a/repo/xyz",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("clone 失败"), "应说明 clone 失败：{msg}");
    assert!(
        !data.path().join("codegraph").join("ghost-repo").exists(),
        "失败不许留半成品目录"
    );
    // 列表里也不该有这条记录
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
    let rows: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        !rows
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["name"] == "ghost-repo"),
        "失败不许落条目：{rows}"
    );
}

/// 删除分流：**默认落盘连目录清**；**自定义落盘目录保留**（note 说明）。
#[tokio::test]
async fn delete_removes_dir_only_for_default_dest() {
    let data = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let (_r1, url1) = git_repo(src.path(), "repo-a");
    let (_r2, url2) = git_repo(src.path(), "repo-b");
    let parent = data.path().join("repos");
    std::fs::create_dir_all(&parent).unwrap();
    let (app, _pg) = app_with_data_dir(data.path()).await;
    let k = key(&app).await;

    let (st1, v1) = register(
        &app,
        &k,
        serde_json::json!({"name": "del-default", "source_uri": url1}),
    )
    .await;
    assert_eq!(st1, StatusCode::CREATED, "{v1}");
    let default_dir = std::path::PathBuf::from(v1["project"]["path"].as_str().unwrap());

    let (st2, v2) = register(
        &app,
        &k,
        serde_json::json!({
            "name": "del-custom",
            "source_uri": url2,
            "dest_parent": parent.to_string_lossy(),
        }),
    )
    .await;
    assert_eq!(st2, StatusCode::CREATED, "{v2}");
    let custom_dir = std::path::PathBuf::from(v2["project"]["path"].as_str().unwrap());
    assert!(default_dir.exists() && custom_dir.exists());

    let del = |app: &axum::Router, k: &str, id: String| {
        let app = app.clone();
        let k = k.to_string();
        async move {
            let resp = app
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
            let status = resp.status();
            let v: serde_json::Value = serde_json::from_slice(
                &axum::body::to_bytes(resp.into_body(), usize::MAX)
                    .await
                    .unwrap(),
            )
            .unwrap();
            (status, v)
        }
    };

    // 默认落盘：连目录清
    let (status, v) = del(&app, &k, v1["project"]["id"].as_str().unwrap().to_string()).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["workdir_removed"], true, "默认落盘应连目录清：{v}");
    assert!(!default_dir.exists(), "默认落盘目录应已删除");

    // 自定义落盘：目录保留
    let (status, v) = del(&app, &k, v2["project"]["id"].as_str().unwrap().to_string()).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["workdir_removed"], false, "自定义落盘不该删目录：{v}");
    assert!(custom_dir.exists(), "自定义落盘目录必须保留");
    assert!(
        v["note"].as_str().unwrap_or_default().contains("保留"),
        "note 应说明目录保留：{v}"
    );

    // 重复删除 → 404
    let (status, _) = del(&app, &k, uuid::Uuid::now_v7().to_string()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn index_enqueues_job_returns_202() {
    let data = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let (_repo, url) = git_repo(src.path(), "queued-repo");
    let (app, _pg) = app_with_data_dir(data.path()).await;
    let k = key(&app).await;

    // 注册即已自动入队（一步到位）——这里再手动入队一次验 202
    let (status, v) = register(
        &app,
        &k,
        serde_json::json!({"name": "queued", "source_uri": url}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = v["project"]["id"].as_str().unwrap().to_string();

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
    // R5（task-5）：hint 字段恒存在（正常 null / 不可用或版本不符为「装 + 锁版」命令）
    assert!(v.get("hint").is_some(), "hint 字段应存在: {v}");
    assert!(
        v["hint"].is_null() || v["hint"].is_string(),
        "hint 应为 null 或字符串: {v}"
    );
}

/// EN-26 版本诚实：codegraph list 每项带 freshness，两种形态——
/// ① .git 可读 → stale 布尔 + snapshot_head；② 路径失效 → stale=null（不虚报）。
#[tokio::test]
async fn list_freshness_two_shapes() {
    let data = tempfile::tempdir().unwrap();
    let (app, _pg) = app_with_data_dir(data.path()).await;
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

    // ① 真 git 仓库 → clone 落盘（.git 可读）
    let src = tempfile::tempdir().unwrap();
    let (repo, url) = git_repo(src.path(), "live-repo");
    let head = {
        let out = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let resp = post(&app, &k, "fresh-live", url).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    // 模拟「已索引」：写入 stats（head 快照戳 + last_indexed）——HTTP 测试环境无真 CLI
    {
        let url_db = support::connection_url(&_pg).await.unwrap();
        let pool = support::connect_with_retry(&url_db).await.unwrap();
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

    // ② 注册后摘走 clone 目录（路径失效 → 不虚报）
    let src2 = tempfile::tempdir().unwrap();
    let (_repo2, url2) = git_repo(src2.path(), "ghost-repo");
    let resp = post(&app, &k, "fresh-ghost", url2).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    std::fs::remove_dir_all(data.path().join("codegraph").join("fresh-ghost")).unwrap();

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

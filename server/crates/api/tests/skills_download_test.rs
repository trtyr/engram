//! 技能下载通道集成测试：
//! ① file?raw=1 单文件直下 ② bundle 整包 zip 已随二态改造移除（路由 404）。
//! 另覆盖 script 型技能的 API 生命周期（指针现读 / file·versions 拒绝）。

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;

use crate::support::{create_key, login_token};

#[tokio::test]
async fn raw_single_file_download() {
    let (app, _pg) = support::app().await;
    let admin = login_token(&app).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/skills")
                .header("authorization", format!("Bearer {admin}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r##"{"name":"Raw 技能","slug":"raw-skill","description":"","content":"# x","tags":[]}"##,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let key = create_key(&app, &admin, &["skills"]).await;
    let put = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/skills/raw-skill/file")
                .header("authorization", format!("Bearer {key}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r##"{"path":"references/go.md","content":"# go"}"##,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);

    // raw 直下：200 + text/plain + 文件名 + 内容本体（非 JSON 包装）
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/skills/raw-skill/file?path=references%2Fgo.md&raw=1")
                .header("authorization", format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (parts, body) = resp.into_parts();
    if parts.status != StatusCode::OK {
        let b = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        panic!(
            "raw GET 状态 {:?} body: {}",
            parts.status,
            String::from_utf8_lossy(&b)
        );
    }
    assert_eq!(parts.headers["content-type"], "text/plain; charset=utf-8");
    assert_eq!(
        parts.headers["content-disposition"],
        "attachment; filename=\"go.md\""
    );
    let body = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    assert_eq!(body, "# go");
}

#[tokio::test]
async fn bundle_endpoint_removed() {
    let (app, _pg) = support::app().await;
    let admin = login_token(&app).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/skills")
                .header("authorization", format!("Bearer {admin}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r##"{"name":"打包技能","slug":"pack-me","description":"整包","content":"# 本体","tags":["ops"]}"##,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let key = create_key(&app, &admin, &["skills"]).await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/skills/pack-me/bundle")
                .header("authorization", format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // 二态改造（0038）：下载功能整体移除——bundle 路由不复存在，
    // 请求落到 SPA fallback（200 text/html），不再是 zip 下载
    let (parts, body) = resp.into_parts();
    let ct = parts.headers["content-type"].to_str().unwrap_or("");
    assert!(
        parts.status == StatusCode::NOT_FOUND || ct.starts_with("text/html"),
        "bundle 路由应已移除，实得 {} {ct}",
        parts.status
    );
    assert!(!ct.starts_with("application/zip"), "不应再有 zip 下载");
    let _ = body;
}

#[tokio::test]
async fn script_kind_api_lifecycle() {
    let (app, _pg) = support::app().await;
    let admin = login_token(&app).await;

    // 本地技能文件夹（temp 下唯一目录）
    let dir = std::env::temp_dir().join(format!(
        "engram-api-script-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SKILL.md"), "# API 现读正文").unwrap();
    let dir_str = dir.to_string_lossy().to_string();

    // create kind=script
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/skills")
                .header("authorization", format!("Bearer {admin}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "name": "本地工具",
                        "slug": "local-tool",
                        "description": "script 型",
                        "kind": "script",
                        "origin": "both",
                        "local_path": dir_str,
                        "repo_url": "https://github.com/x/local-tool",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);

    let key = create_key(&app, &admin, &["skills"]).await;

    // get：script 型从 local_path 现读正文
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/skills/local-tool")
                .header("authorization", format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["kind"], "script");
    assert_eq!(body["origin"], "both");
    assert_eq!(body["content"], "# API 现读正文");
    assert_eq!(body["repo_url"], "https://github.com/x/local-tool");

    // file 写入拒绝（真身在本地）
    let put = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/skills/local-tool/file")
                .header("authorization", format!("Bearer {key}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"path":"scripts/a.txt","content":"x"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::BAD_REQUEST);

    // versions 拒绝（script 不产快照）
    let revs = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/skills/local-tool/revisions")
                .header("authorization", format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revs.status(), StatusCode::BAD_REQUEST);

    // 指针失效：目录移走 → get 404 + 指引
    let moved = dir.with_extension("moved");
    std::fs::rename(&dir, &moved).unwrap();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/skills/local-tool")
                .header("authorization", format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("指针失效"), "实际错误体: {body}");

    let _ = std::fs::remove_dir_all(&moved);
}

#[tokio::test]
async fn text_rejects_script_attachment_api() {
    let (app, _pg) = support::app().await;
    let admin = login_token(&app).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/skills")
                .header("authorization", format!("Bearer {admin}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r##"{"name":"纯文本","slug":"text-only","description":"","content":"# x","tags":[]}"##,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let key = create_key(&app, &admin, &["skills"]).await;
    // 缺省 kind=text：附属 .py 脚本被拒
    let put = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/skills/text-only/file")
                .header("authorization", format!("Bearer {key}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"path":"scripts/run.py","content":"print('x')"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::BAD_REQUEST);
}

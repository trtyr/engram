//! 技能三层消费通道集成测试：
//! ① MCP/JSON 看内容（既有端点） ② file?raw=1 单文件直下 ③ bundle 整包 zip。

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
                    r#"{"path":"scripts/go.py","content":"print('go')"}"#,
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
                .uri("/skills/raw-skill/file?path=scripts%2Fgo.py&raw=1")
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
        "attachment; filename=\"go.py\""
    );
    let body = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    assert_eq!(body, "print('go')");
}

#[tokio::test]
async fn bundle_is_zip_with_skill_md_and_files() {
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
    let put = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/skills/pack-me/file")
                .header("authorization", format!("Bearer {key}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"path":"scripts/run.sh","content":"echo hi"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);

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
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()["content-type"], "application/zip");

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    // zip 本地文件头魔数 PK
    assert_eq!(&body[..2], b"PK", "bundle 应是真 zip");

    // 解包校验：SKILL.md（frontmatter 往返）+ scripts/run.sh
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(body)).unwrap();
    let md = {
        let mut f = z.by_name("SKILL.md").unwrap();
        let mut s = String::new();
        std::io::Read::read_to_string(&mut f, &mut s).unwrap();
        s
    };
    assert!(md.starts_with(
        "---
"
    ));
    assert!(md.contains("name: 打包技能"));
    assert!(md.contains("slug: pack-me"));
    assert!(md.contains("# 本体"));

    let script = {
        let mut f = z.by_name("scripts/run.sh").unwrap();
        let mut s = String::new();
        std::io::Read::read_to_string(&mut f, &mut s).unwrap();
        s
    };
    assert_eq!(script, "echo hi");
}

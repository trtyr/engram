//! 登录防爆破限速（公网加固 P001-t8）：连续失败 5 次锁 15 分钟，
//! 锁定窗口内即使密码正确也 429；成功登录清零。
//! 注意：限速器是进程内全局静态——本文件的测试以「admin」为对象，
//! 结尾必须 clear_all 复位；其他测试文件是独立进程不受影响。

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::util::ServiceExt;

#[tokio::test]
async fn login_locks_after_repeated_failures() {
    engram_api::login_throttle::clear_all();
    let (app, _pg) = support::app().await;

    let login = |app: &axum::Router, pw: &'static str| {
        let app = app.clone();
        async move {
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"username":"admin","password":"{pw}"}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };

    // 连续 5 次错密码 → 全部 401
    for _ in 0..5 {
        let resp = login(&app, "wrong-password").await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // 锁定生效：正确密码也 429
    let resp = login(&app, "test-admin-pw").await;
    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "连败 5 次后正确密码也应 429"
    );

    // 复位限速表 → 恢复正常登录（语义：管理干预/重启即复位）
    engram_api::login_throttle::clear_all();
    let resp = login(&app, "test-admin-pw").await;
    assert_eq!(resp.status(), StatusCode::OK, "复位后正确密码应登录成功");
}

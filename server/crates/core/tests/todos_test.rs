//! 待办域服务集成测试（R7 修验）：done 幂等（D17）、负 limit（D19）、
//! NUL 字节拒绝（D20）、空 tag 规整、非法 due_at（D18，MCP 层解析，此处测服务侧语义）。

mod support;

use engram_core::todos::TodoService;
use sqlx::PgPool;

async fn setup() -> (PgPool, TodoService, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    let svc = TodoService::new(pool.clone());
    (pool, svc, container)
}

/// D17：重复 done 是 no-op——首次完成时间戳不被改写。
#[tokio::test]
async fn done_is_idempotent_keeps_first_done_at() {
    let (_pool, svc, _pg) = setup().await;
    let t = svc
        .create("D17 幂等", "", "normal", &[], None, None)
        .await
        .unwrap();
    assert_eq!(t.status, "open");
    assert!(t.done_at.is_none());

    let done1 = svc
        .update(t.id, None, None, None, Some("done"), None, None, None)
        .await
        .unwrap();
    let first = done1.done_at.expect("首次完成应有 done_at");

    // 再 done：done_at 保持首值（此前被刷新成第二次 now()）
    let done2 = svc
        .update(t.id, None, None, None, Some("done"), None, None, None)
        .await
        .unwrap();
    assert_eq!(done2.done_at, Some(first), "重复 done 不得改写 done_at");

    // done → open 清空；open → done 重新盖新时间戳（新的一次完成）
    let reopened = svc
        .update(t.id, None, None, None, Some("open"), None, None, None)
        .await
        .unwrap();
    assert!(reopened.done_at.is_none());
    let redone = svc
        .update(t.id, None, None, None, Some("done"), None, None, None)
        .await
        .unwrap();
    assert!(redone.done_at.unwrap() >= first);
}

/// D19：负 limit 响亮拒绝（此前漏到 PG 报「LIMIT must not be negative」）。
#[tokio::test]
async fn negative_limit_is_rejected() {
    let (_pool, svc, _pg) = setup().await;
    let err = svc
        .list(None, None, None, None, -1)
        .await
        .expect_err("负 limit 应报错");
    assert!(
        err.to_string().contains("limit 不能为负"),
        "应报参数错误而非存储故障：{err}"
    );
    // 上限 clamp 语义保持：超大 limit 合法
    svc.list(None, None, None, None, 100000).await.unwrap();
}

/// D20：NUL 字节入参响亮拒绝（此前漏到 PG 报 UTF8 编码错误）。
#[tokio::test]
async fn nul_bytes_are_rejected() {
    let (_pool, svc, _pg) = setup().await;
    for (field, title, body) in [("title", "坏\0标题", ""), ("body", "正常标题", "正\0文")]
    {
        let err = svc
            .create(title, body, "normal", &[], None, None)
            .await
            .expect_err("NUL 应被拒");
        assert!(
            err.to_string().contains("NUL"),
            "{field} 应在入参层拒绝：{err}"
        );
    }
    // update 通道同样拒绝
    let t = svc
        .create("D20 更新通道", "", "normal", &[], None, None)
        .await
        .unwrap();
    let err = svc
        .update(t.id, Some("坏\0标题"), None, None, None, None, None, None)
        .await
        .expect_err("update NUL 应被拒");
    assert!(err.to_string().contains("NUL"), "{err}");
}

/// 空 tag 规整：trim + 丢弃空串（观察项）。
#[tokio::test]
async fn empty_tags_are_normalized() {
    let (_pool, svc, _pg) = setup().await;
    let t = svc
        .create(
            "tag 规整",
            "",
            "normal",
            &["  学习  ".into(), "".into(), "   ".into()],
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        t.tags,
        vec!["学习"],
        "空 tag 应被丢弃、其余 trim：{:?}",
        t.tags
    );
}

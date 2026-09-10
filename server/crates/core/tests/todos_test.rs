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
        .create(
            "D17 幂等",
            "",
            "todo",
            "normal",
            None,
            "",
            "",
            "",
            &[],
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(t.status, "open");
    assert!(t.done_at.is_none());

    let done1 = svc
        .update(
            t.id,
            None,
            None,
            None,
            None,
            Some("done"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let first = done1.done_at.expect("首次完成应有 done_at");

    // 再 done：done_at 保持首值（此前被刷新成第二次 now()）
    let done2 = svc
        .update(
            t.id,
            None,
            None,
            None,
            None,
            Some("done"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(done2.done_at, Some(first), "重复 done 不得改写 done_at");

    // done → open 清空；open → done 重新盖新时间戳（新的一次完成）
    let reopened = svc
        .update(
            t.id,
            None,
            None,
            None,
            None,
            Some("open"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(reopened.done_at.is_none());
    let redone = svc
        .update(
            t.id,
            None,
            None,
            None,
            None,
            Some("done"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(redone.done_at.unwrap() >= first);
}

/// D19：负 limit 响亮拒绝（此前漏到 PG 报「LIMIT must not be negative」）。
#[tokio::test]
async fn negative_limit_is_rejected() {
    let (_pool, svc, _pg) = setup().await;
    let err = svc
        .list(None, None, None, None, None, -1)
        .await
        .expect_err("负 limit 应报错");
    assert!(
        err.to_string().contains("limit 不能为负"),
        "应报参数错误而非存储故障：{err}"
    );
    // 上限 clamp 语义保持：超大 limit 合法
    svc.list(None, None, None, None, None, 100000)
        .await
        .unwrap();
}

/// D20：NUL 字节入参响亮拒绝（此前漏到 PG 报 UTF8 编码错误）。
#[tokio::test]
async fn nul_bytes_are_rejected() {
    let (_pool, svc, _pg) = setup().await;
    for (field, title, body) in [("title", "坏\0标题", ""), ("body", "正常标题", "正\0文")]
    {
        let err = svc
            .create(
                title,
                body,
                "todo",
                "normal",
                None,
                "",
                "",
                "",
                &[],
                None,
                None,
            )
            .await
            .expect_err("NUL 应被拒");
        assert!(
            err.to_string().contains("NUL"),
            "{field} 应在入参层拒绝：{err}"
        );
    }
    // update 通道同样拒绝
    let t = svc
        .create(
            "D20 更新通道",
            "",
            "todo",
            "normal",
            None,
            "",
            "",
            "",
            &[],
            None,
            None,
        )
        .await
        .unwrap();
    let err = svc
        .update(
            t.id,
            None,
            Some("坏\0标题"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect_err("update NUL 应被拒");
    assert!(err.to_string().contains("NUL"), "{err}");
}

/// D29：keyset 游标翻页走全量——open 优先复合排序下无重复、无丢失、无静默截断。
#[tokio::test]
async fn cursor_pagination_walks_all_without_loss() {
    let (_pool, svc, _pg) = setup().await;
    // 12 条：交替 open/done，updated_at 天然逐条递增（各自独立事务）
    let mut created = Vec::new();
    for i in 0..12 {
        let status = if i % 2 == 0 { None } else { Some("done") };
        let t = svc
            .create(
                &format!("R9D29-{i:02}"),
                "",
                "todo",
                "normal",
                None,
                "",
                "",
                "",
                &[],
                None,
                None,
            )
            .await
            .unwrap();
        let t = match status {
            Some(s) => svc
                .update(
                    t.id,
                    None,
                    None,
                    None,
                    None,
                    Some(s),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await
                .unwrap(),
            None => t,
        };
        created.push(t);
    }

    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let c = cursor.as_deref();
        let page = svc.list(None, None, None, None, c, 5).await.unwrap();
        assert!(page.len() <= 5);
        if page.is_empty() {
            break;
        }
        for t in &page {
            seen.push(t.clone());
        }
        let last = page.last().unwrap();
        let flag = if last.status == "open" { 1 } else { 0 };
        cursor = Some(format!(
            "{}|{}|{}",
            flag,
            last.updated_at.to_rfc3339(),
            last.id
        ));
        if seen.len() > 12 {
            panic!("游标翻页越走越多（重复）");
        }
    }

    assert_eq!(seen.len(), 12, "翻页应恰好走全 12 条：{}", seen.len());
    let ids: std::collections::HashSet<_> = seen.iter().map(|t| t.id).collect();
    assert_eq!(ids.len(), 12, "翻页不得重复：{:?}", seen.len());
    // open 优先：所有 open 在所有非 open 之前
    let first_non_open = seen.iter().position(|t| t.status != "open");
    if let Some(pos) = first_non_open {
        assert!(
            seen[pos..].iter().all(|t| t.status != "open"),
            "open 优先序被破坏"
        );
    }
    // 逐条对得上（幂等集合）
    let expect: std::collections::HashSet<_> = created.iter().map(|t| t.id).collect();
    assert_eq!(ids, expect, "翻页集合应与全量一致");
    // 垃圾游标响亮拒
    let err = svc
        .list(None, None, None, None, Some("garbage"), 5)
        .await
        .expect_err("垃圾游标应被拒");
    assert!(err.to_string().contains("cursor"), "{err}");
}

/// 空 tag 规整：trim + 丢弃空串（观察项）。
#[tokio::test]
async fn empty_tags_are_normalized() {
    let (_pool, svc, _pg) = setup().await;
    let t = svc
        .create(
            "tag 规整",
            "",
            "todo",
            "normal",
            None,
            "",
            "",
            "",
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

//! 待办域服务集成测试（0074 工单拆表后回归纯行动项）：
//! done 幂等（D17）、负 limit（D19）、NUL 字节拒绝（D20）、空 tag 规整、
//! keyset 游标翻页（D29）、短号与关联。

mod support;

use chrono::Utc;
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
        .create("D17 幂等", "", "normal", &[], None)
        .await
        .unwrap();
    assert_eq!(t.status, "open");
    assert!(t.done_at.is_none());

    let done1 = svc
        .update(t.id, None, None, None, Some("done"), None, None)
        .await
        .unwrap();
    let first = done1.done_at.expect("首次完成应有 done_at");

    // 再 done：done_at 保持首值
    let done2 = svc
        .update(t.id, None, None, None, Some("done"), None, None)
        .await
        .unwrap();
    assert_eq!(done2.done_at, Some(first), "重复 done 不得改写 done_at");

    // done → open 清空；open → done 重新盖新时间戳
    let reopened = svc
        .update(t.id, None, None, None, Some("open"), None, None)
        .await
        .unwrap();
    assert!(reopened.done_at.is_none());
    let redone = svc
        .update(t.id, None, None, None, Some("done"), None, None)
        .await
        .unwrap();
    assert!(redone.done_at.unwrap() >= first);
}

/// D19：负 limit 响亮拒绝。
#[tokio::test]
async fn negative_limit_is_rejected() {
    let (_pool, svc, _pg) = setup().await;
    let err = svc
        .list(None, None, None, None, None, None, -1)
        .await
        .expect_err("负 limit 应报错");
    assert!(
        err.to_string().contains("limit 不能为负"),
        "应报参数错误而非存储故障：{err}"
    );
    svc.list(None, None, None, None, None, None, 100000)
        .await
        .unwrap();
}

/// D20：NUL 字节入参响亮拒绝。
#[tokio::test]
async fn nul_bytes_are_rejected() {
    let (_pool, svc, _pg) = setup().await;
    for (field, title, body) in [("title", "坏\0标题", ""), ("body", "正常标题", "正\0文")]
    {
        let err = svc
            .create(title, body, "normal", &[], None)
            .await
            .expect_err("NUL 应被拒");
        assert!(
            err.to_string().contains("NUL"),
            "{field} 应在入参层拒绝：{err}"
        );
    }
    // update 通道同样拒绝
    let t = svc
        .create("D20 更新通道", "", "normal", &[], None)
        .await
        .unwrap();
    let err = svc
        .update(t.id, Some("坏\0标题"), None, None, None, None, None)
        .await
        .expect_err("update NUL 应被拒");
    assert!(err.to_string().contains("NUL"), "{err}");
}

/// D29：keyset 游标翻页走全量——open 优先复合排序下无重复、无丢失。
#[tokio::test]
async fn cursor_pagination_walks_all_without_loss() {
    let (_pool, svc, _pg) = setup().await;
    let mut created = Vec::new();
    for i in 0..12 {
        let done = i % 2 == 1;
        let t = svc
            .create(&format!("R9D29-{i:02}"), "", "normal", &[], None)
            .await
            .unwrap();
        let t = if done {
            svc.update(t.id, None, None, None, Some("done"), None, None)
                .await
                .unwrap()
        } else {
            t
        };
        created.push(t);
    }

    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = svc
            .list(None, None, None, None, None, cursor.as_deref(), 5)
            .await
            .unwrap();
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
    assert_eq!(ids.len(), 12, "翻页不得重复");
    let expect: std::collections::HashSet<_> = created.iter().map(|t| t.id).collect();
    assert_eq!(ids, expect, "翻页集合应与全量一致");
    // 垃圾游标响亮拒
    let err = svc
        .list(None, None, None, None, None, Some("garbage"), 5)
        .await
        .expect_err("垃圾游标应被拒");
    assert!(err.to_string().contains("cursor"), "{err}");
}

/// 空 tag 规整：trim + 丢弃空串。
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

/// 非法 status / priority 响亮拒绝（应用层友好版，CHECK 兜底）。
#[tokio::test]
async fn invalid_status_and_priority_rejected() {
    let (_pool, svc, _pg) = setup().await;
    let t = svc.create("状态机", "", "normal", &[], None).await.unwrap();
    // todo 拒工单态
    let e = svc
        .update(t.id, None, None, None, Some("in_progress"), None, None)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("status 仅接受"), "{e}");
    // 非法 priority
    let e = svc
        .create("优先级", "", "urgent", &[], None)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("priority 仅接受"), "{e}");
}

/// P019-M4：due 过滤×cursor 翻页排序键统一——旧实现首页 due_at 升序、
/// 翻页按 (open,updated_at,id) 降序且游标缺 due_at，跨页丢行/重行。
#[tokio::test]
async fn due_filter_pagination_no_loss_no_dup() {
    let (_pool, svc, _pg) = setup().await;
    // 5 条已过期待办（due_at 递增错开）
    let mut ids = Vec::new();
    for i in 0..5 {
        let t = svc
            .create(
                &format!("过期待办{i}"),
                "",
                "normal",
                &[],
                Some(Utc::now() - chrono::Duration::hours(48 - i)),
            )
            .await
            .unwrap();
        ids.push(t.id);
    }

    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let rows = svc
            .list(
                None,
                None,
                None,
                None,
                Some("overdue"),
                cursor.as_deref(),
                2,
            )
            .await
            .unwrap();
        let last = rows.last().cloned();
        seen.extend(rows.iter().map(|t| t.id));
        match last {
            Some(t) => {
                let due = t.due_at.expect("overdue 行必有 due_at");
                cursor = Some(format!("{}|{}", due.to_rfc3339(), t.id));
                if rows.len() < 2 {
                    break;
                }
            }
            None => break,
        }
    }
    assert_eq!(seen.len(), 5, "五条过期待办应全部命中: {seen:?}");
    let mut sorted = seen.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 5, "翻页不得重复: {seen:?}");
    let mut expected = ids.clone();
    expected.sort();
    let mut got = seen.clone();
    got.sort();
    assert_eq!(got, expected, "无丢行");
}

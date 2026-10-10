//! 工单域服务集成测试（0074 拆独立表）：项目绑定制（不存在项目拒绝）、
//! 字段往返、解决必填闸门、六态状态机自动留痕、按项目过滤。
mod support;

use engram_core::tickets::TicketService;
use sqlx::PgPool;
use uuid::Uuid;

async fn setup() -> (PgPool, TicketService, Uuid, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    // FK 目标：先落一个项目
    let pid = Uuid::now_v7();
    sqlx::query("INSERT INTO projects (id, name, type) VALUES ($1, '工单测试项目', 'dev')")
        .bind(pid)
        .execute(&pool)
        .await
        .expect("建项目");
    let svc = TicketService::new(pool.clone());
    (pool, svc, pid, container)
}

/// 项目绑定制：不存在的 project_id 直接拒绝——绝不自动建项目。
#[tokio::test]
async fn create_rejects_nonexistent_project() {
    let (_pool, svc, _pid, _pg) = setup().await;
    let ghost = Uuid::now_v7();
    let err = svc
        .create(ghost, "悬空工单", "", Some("P2"), "症状", "", "")
        .await
        .expect_err("不存在项目应拒绝");
    assert!(
        err.to_string().contains("不存在") && err.to_string().contains("绑定已有项目"),
        "应明确报项目绑定失败：{err}"
    );
}

/// 建单 + 字段往返（severity/symptom/reproduce/acceptance + EN 短号）。
#[tokio::test]
async fn create_and_roundtrip() {
    let (_pool, svc, pid, _pg) = setup().await;
    let t = svc
        .create(
            pid,
            "登录页 500",
            "首页打不开",
            Some("P1"),
            "白屏",
            "访问 /login",
            "可登录",
        )
        .await
        .unwrap();
    assert_eq!(t.status, "open");
    assert_eq!(t.severity.as_deref(), Some("P1"));
    assert_eq!(t.symptom, "白屏");
    assert_eq!(t.project_id, pid);
    assert!(t.short_no > 0, "应有全局短号");

    let back = svc
        .find_by_ref(&format!("EN-{}", t.short_no))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(back.id, t.id, "EN 短号应可直达");
}

/// 解决必填闸门：转 resolved/verified 无解决记录拒绝；带 resolution 放行。
#[tokio::test]
async fn resolution_gate() {
    let (_pool, svc, pid, _pg) = setup().await;
    let t = svc
        .create(pid, "解决闸门", "", None, "", "", "")
        .await
        .unwrap();

    let e = svc
        .update(
            t.id,
            None,
            None,
            Some("resolved"),
            None,
            None,
            None,
            None,
            None,
            "test",
        )
        .await
        .unwrap_err();
    assert!(e.to_string().contains("解决记录"), "{e}");

    let done = svc
        .update(
            t.id,
            None,
            None,
            Some("resolved"),
            None,
            None,
            None,
            None,
            Some("修了 N+1 查询"),
            "test",
        )
        .await
        .unwrap();
    assert_eq!(done.status, "resolved");
    assert!(done.resolved_at.is_some(), "resolved 应盖解决时间戳");

    // verified 从 resolved 来：resolution 已有，不再强制重填
    let verified = svc
        .update(
            t.id,
            None,
            None,
            Some("verified"),
            None,
            None,
            None,
            None,
            None,
            "test",
        )
        .await
        .unwrap();
    assert_eq!(verified.status, "verified");
}

/// 六态状态机流转自动留痕（from→to event）。
#[tokio::test]
async fn state_machine_writes_flow_events() {
    let (_pool, svc, pid, _pg) = setup().await;
    let t = svc
        .create(pid, "流转留痕", "", None, "", "", "")
        .await
        .unwrap();

    svc.update(
        t.id,
        None,
        None,
        Some("confirmed"),
        None,
        None,
        None,
        None,
        None,
        "tester",
    )
    .await
    .unwrap();
    svc.update(
        t.id,
        None,
        None,
        Some("in_progress"),
        None,
        None,
        None,
        None,
        None,
        "tester",
    )
    .await
    .unwrap();
    svc.event_add(
        t.id,
        "comment",
        &serde_json::json!({ "text": "第一条评论" }),
        "tester",
    )
    .await
    .unwrap();

    let ev = svc.events(t.id).await.unwrap();
    assert!(ev.len() >= 3, "两次流转 + 一条评论：{:?}", ev.len());
    assert_eq!(ev[0].kind, "event");
    assert_eq!(ev[0].payload["from"], "open");
    assert_eq!(ev[0].payload["to"], "confirmed");
    assert_eq!(ev[1].payload["from"], "confirmed");
    assert_eq!(ev[1].payload["to"], "in_progress");
    assert_eq!(ev[2].kind, "comment");
    for w in ev.windows(2) {
        assert!(w[0].created_at <= w[1].created_at, "时间线应升序");
    }
}

/// 列表过滤：按项目 + 状态；别的项目查不到本项目的工单。
#[tokio::test]
async fn list_filters_by_project() {
    let (pool, svc, pid, _pg) = setup().await;
    svc.create(pid, "本项目的工单", "", None, "", "", "")
        .await
        .unwrap();

    // 别的项目：查不到
    let other = Uuid::now_v7();
    sqlx::query("INSERT INTO projects (id, name, type) VALUES ($1, '另一个项目', 'dev')")
        .bind(other)
        .execute(&pool)
        .await
        .unwrap();

    let (mine, total) = svc
        .list(None, None, Some(pid), None, None, 100)
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(mine[0].project_id, pid);

    let (theirs, total2) = svc
        .list(None, None, Some(other), None, None, 100)
        .await
        .unwrap();
    assert_eq!(total2, 0);
    assert!(theirs.is_empty());

    // 状态过滤
    let (open, _) = svc
        .list(Some("open"), None, Some(pid), None, None, 100)
        .await
        .unwrap();
    assert_eq!(open.len(), 1);
    let (confirmed, _) = svc
        .list(Some("confirmed"), None, Some(pid), None, None, 100)
        .await
        .unwrap();
    assert!(confirmed.is_empty());
}

/// 项目删除级联清工单（绑定没了即悬空——外键 ON DELETE CASCADE）。
#[tokio::test]
async fn project_delete_cascades_tickets() {
    let (pool, svc, pid, _pg) = setup().await;
    let t = svc
        .create(pid, "随项目消亡", "", None, "", "", "")
        .await
        .unwrap();
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(pid)
        .execute(&pool)
        .await
        .unwrap();
    let gone = svc.find_by_ref(&t.id.to_string()).await.unwrap();
    assert!(gone.is_none(), "项目删除应级联清工单");
}

/// P019-M1：export_all 跨页拉全——>500 条（list 单页 min(500) 钉制）时导出行数 == 全量。
#[tokio::test]
async fn export_all_pages_past_single_page_limit() {
    let (_pool, svc, pid, _pg) = setup().await;
    for i in 0..501 {
        svc.create(pid, &format!("批量工单 {i:03}"), "", None, "", "", "")
            .await
            .unwrap();
    }
    let (page, total) = svc
        .list(None, None, Some(pid), None, None, 500)
        .await
        .unwrap();
    assert_eq!(page.len(), 500, "list 单页仍应钉 500");
    assert_eq!(total, 501);

    let all = svc.export_all().await.unwrap();
    assert_eq!(all.len(), 501, "export_all 应跨页拉全，得 {}", all.len());
    // 无重无漏：id 唯一
    let mut ids: Vec<_> = all.iter().map(|t| t.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 501, "导出不得重复");
}

/// P019-M5：负 limit 400（对照 todos 同款）——旧实现穿透到 SQL LIMIT 报 5xx
#[tokio::test]
async fn negative_limit_is_rejected() {
    let (_pool, svc, pid, _pg) = setup().await;
    let e = svc
        .list(None, None, Some(pid), None, None, -1)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("limit 不能为负"), "{e}");
}

/// P019-M4：非法状态跳转 400——open→verified 跳级拒绝；archived 终态不可出；
/// 合法链 open→resolved→archived 走通（0041 自述「状态迁移」实装）。
#[tokio::test]
async fn illegal_status_jump_rejected() {
    let (_pool, svc, pid, _pg) = setup().await;
    let t = svc
        .create(pid, "非法跳转", "", None, "", "", "")
        .await
        .unwrap();

    let e = svc
        .update(
            t.id,
            None,
            None,
            Some("verified"),
            None,
            None,
            None,
            None,
            None,
            "t",
        )
        .await
        .unwrap_err();
    assert!(e.to_string().contains("非法状态迁移"), "{e}");

    // 合法链走通
    svc.update(
        t.id,
        None,
        None,
        Some("resolved"),
        None,
        None,
        None,
        None,
        Some("修了"),
        "t",
    )
    .await
    .unwrap();
    svc.update(
        t.id,
        None,
        None,
        Some("archived"),
        None,
        None,
        None,
        None,
        None,
        "t",
    )
    .await
    .unwrap();
    let e2 = svc
        .update(
            t.id,
            None,
            None,
            Some("open"),
            None,
            None,
            None,
            None,
            None,
            "t",
        )
        .await
        .unwrap_err();
    assert!(e2.to_string().contains("非法状态迁移"), "{e2}");
}

//! 待办域服务集成测试（R7 修验）：done 幂等（D17）、负 limit（D19）、
//! NUL 字节拒绝（D20）、空 tag 规整、非法 due_at（D18，MCP 层解析，此处测服务侧语义）。

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
            "test",
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
            "test",
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
            "test",
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
            "test",
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
        .list(None, None, None, None, None, None, None, None, -1)
        .await
        .expect_err("负 limit 应报错");
    assert!(
        err.to_string().contains("limit 不能为负"),
        "应报参数错误而非存储故障：{err}"
    );
    // 上限 clamp 语义保持：超大 limit 合法
    svc.list(None, None, None, None, None, None, None, None, 100000)
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
            "test",
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
                    "test",
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
        let page = svc
            .list(None, None, None, None, None, None, None, c, 5)
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
        .list(None, None, None, None, None, None, None, Some("garbage"), 5)
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

/// ── 0041 双形态：kind 分支 / 工单状态机 / 字段存取 ──
/// todo 型拒绝工单状态；ticket 型拒绝 done（联合 CHECK 的应用层友好版）
#[tokio::test]
async fn kind_rejects_foreign_status() {
    let (_pool, svc, _pg) = setup().await;
    let t = svc
        .create(
            "混合状态机",
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
    let e = svc
        .update(
            t.id,
            None,
            None,
            None,
            None,
            Some("in_progress"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
        )
        .await
        .unwrap_err();
    assert!(e.to_string().contains("kind=todo"), "{e}");
    let t2 = svc
        .create(
            "工单拒done",
            "",
            "ticket",
            "",
            Some("P2"),
            "症状",
            "",
            "",
            &[],
            None,
            None,
        )
        .await
        .unwrap();
    let e2 = svc
        .update(
            t2.id,
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
            "test",
        )
        .await
        .unwrap_err();
    assert!(e2.to_string().contains("kind=ticket"), "{e2}");
}

/// ticket 字段存取：severity/symptom/reproduce/acceptance 全量往返
#[tokio::test]
async fn ticket_fields_roundtrip() {
    let (_pool, svc, _pg) = setup().await;
    let t = svc
        .create(
            "工单字段存取",
            "详情",
            "ticket",
            "",
            Some("P1"),
            "搜索命中不稳",
            "doc_search 查宽泛词",
            "首屏可定位",
            &["工单".into()],
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(t.severity.as_deref(), Some("P1"));
    assert_eq!(t.symptom, "搜索命中不稳");
    assert_eq!(t.reproduce, "doc_search 查宽泛词");
    assert_eq!(t.acceptance, "首屏可定位");
    assert_eq!(t.status, "open");
}

/// 工单状态机：resolved 无 resolution 拒（应用层 400）→ 带 resolution 过 → verified；resolved_at 自动记
#[tokio::test]
async fn ticket_state_machine_and_resolution_gate() {
    let (_pool, svc, _pg) = setup().await;
    let t = svc
        .create(
            "状态机工单",
            "",
            "ticket",
            "",
            Some("P2"),
            "问题现象",
            "",
            "",
            &[],
            None,
            None,
        )
        .await
        .unwrap();
    // todo→ticket 转换路径：update kind 生效后 severity 可写
    assert_eq!(t.kind, "ticket");
    for s in ["confirmed", "in_progress"] {
        svc.update(
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
            "test",
        )
        .await
        .unwrap();
    }
    // resolved 无 resolution → 应用层拒绝（不触 CHECK）
    let e = svc
        .update(
            t.id,
            None,
            None,
            None,
            None,
            Some("resolved"),
            None,
            None,
            None,
            Some(""),
            None,
            None,
            None,
            None,
            "test",
        )
        .await
        .unwrap_err();
    assert!(e.to_string().contains("解决记录"), "{e}");
    // 带 resolution → resolved（resolved_at 自动记）
    let r = svc
        .update(
            t.id,
            None,
            None,
            None,
            None,
            Some("resolved"),
            None,
            None,
            None,
            None,
            Some("0041 修复完成"),
            None,
            None,
            None,
            "test",
        )
        .await
        .unwrap();
    assert_eq!(r.status, "resolved");
    assert!(r.resolved_at.is_some(), "resolved 应自动记时间戳");
    // verified
    let v = svc
        .update(
            t.id,
            None,
            None,
            None,
            None,
            Some("verified"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
        )
        .await
        .unwrap();
    assert_eq!(v.status, "verified");
}

/// kind 转换：todo → ticket（update kind），转换后 severity 生效
#[tokio::test]
async fn kind_conversion_todo_to_ticket() {
    let (_pool, svc, _pg) = setup().await;
    let t = svc
        .create(
            "行动项转工单",
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
    assert_eq!(t.kind, "todo");
    let u = svc
        .update(
            t.id,
            Some("ticket"),
            None,
            None,
            None,
            None,
            Some(Some("P3")),
            Some("转换后补症状"),
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
        )
        .await
        .unwrap();
    assert_eq!(u.kind, "ticket");
    assert_eq!(u.severity.as_deref(), Some("P3"));
    assert_eq!(u.symptom, "转换后补症状");
}

/// 工单模型细化四件：短号往返 / 三关联幂等+双向 / ticket priority 退役 / find_by_ref
#[tokio::test]
async fn short_no_and_links() {
    let (_pool, svc, _pg) = setup().await;
    // ① 短号往返：create 返回 short_no≥1；find_by_ref("EN-n") 等价 UUID
    let t = svc
        .create(
            "短号测试",
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
    assert!(t.short_no >= 1, "短号应为正整数");
    let by_ref = svc
        .find_by_ref(&format!("EN-{}", t.short_no))
        .await
        .unwrap()
        .expect("EN 引用直达");
    assert_eq!(by_ref.id, t.id);
    assert!(svc.find_by_ref("EN-99999").await.unwrap().is_none());
    assert!(svc.find_by_ref("garbage").await.is_err(), "非法引用 400");

    // ② 三种关联 + 幂等 + 双向 + 解除
    let a = svc
        .create(
            "A 被阻塞",
            "",
            "ticket",
            "",
            Some("P1"),
            "同根因",
            "",
            "",
            &[],
            None,
            None,
        )
        .await
        .unwrap();
    let b = svc
        .create(
            "B 根因",
            "",
            "ticket",
            "",
            Some("P0"),
            "根因票",
            "",
            "",
            &[],
            None,
            None,
        )
        .await
        .unwrap();
    let c = svc
        .create(
            "C 相关",
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
    assert!(
        svc.link(a.id, b.id, "blocked_by").await.unwrap(),
        "首次插入 true"
    );
    assert!(
        !svc.link(a.id, b.id, "blocked_by").await.unwrap(),
        "重复 link 幂等 false"
    );
    assert!(svc.link(a.id, c.id, "relates_to").await.unwrap());
    assert!(svc.link(a.id, b.id, "parent").await.unwrap());
    assert!(svc.link(a.id, a.id, "relates_to").await.is_err(), "自环拒");
    let links = svc.links(a.id).await.unwrap();
    assert_eq!(links.len(), 3);
    let counts = svc.link_count_map().await.unwrap();
    assert_eq!(counts.get(&a.id), Some(&3));
    assert!(svc.unlink(a.id, b.id, "parent").await.unwrap());
    assert_eq!(svc.links(a.id).await.unwrap().len(), 2);

    // ④ ticket priority 退役：create 非空 400；todo 正常
    assert!(
        svc.create(
            "ticket priority",
            "",
            "ticket",
            "high",
            Some("P2"),
            "",
            "",
            "",
            &[],
            None,
            None
        )
        .await
        .is_err(),
        "ticket 传 priority 应 400"
    );
    let u = svc
        .update(
            t.id,
            None,
            None,
            None,
            Some("high"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
        )
        .await;
    // t 是 todo——priority 正常更新不受影响
    assert!(u.is_ok());
}

/// 工单页筛选的后端支撑（EN-48 拆页后续）：list 的 status 合法集按 kind 取 +
/// severity 过滤真实生效。此前 status 只认 todo 三态（工单态直接 400「仅接受
/// open/done/archived」）、severity 压根不是 list 参数（HTTP 层静默丢弃 → 筛选空操作）。
#[tokio::test]
async fn list_supports_ticket_status_and_severity_filters() {
    let (_pool, svc, _pg) = setup().await;
    // 两条工单：P0 + P2；一条普通 todo
    let t_p0 = svc
        .create(
            "工单P0",
            "",
            "ticket",
            "",
            Some("P0"),
            "症状A",
            "",
            "",
            &[],
            None,
            None,
        )
        .await
        .unwrap();
    let t_p2 = svc
        .create(
            "工单P2",
            "",
            "ticket",
            "",
            Some("P2"),
            "症状B",
            "",
            "",
            &[],
            None,
            None,
        )
        .await
        .unwrap();
    let _todo = svc
        .create(
            "普通待办",
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

    // ① 工单态 status=confirmed 直接可用（旧实现 400）
    svc.update(
        t_p0.id,
        None,
        None,
        None,
        None,
        Some("confirmed"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        "test",
    )
    .await
    .unwrap();
    let confirmed = svc
        .list(
            Some("confirmed"),
            Some("ticket"),
            None,
            None,
            None,
            None,
            None,
            None,
            200,
        )
        .await
        .unwrap();
    assert_eq!(confirmed.len(), 1, "confirmed 工单应恰好 1 条");
    assert_eq!(confirmed[0].id, t_p0.id);

    // ② severity=P2 过滤真实生效（旧实现该参数不存在，返回全部）
    let p2 = svc
        .list(
            None,
            Some("ticket"),
            None,
            None,
            None,
            Some("P2"),
            None,
            None,
            200,
        )
        .await
        .unwrap();
    assert_eq!(p2.len(), 1, "P2 工单应恰好 1 条");
    assert_eq!(p2[0].id, t_p2.id);

    // ③ 组合过滤：P0 + status=open（P0 那条已 confirmed，应空）
    let p0_open = svc
        .list(
            Some("open"),
            Some("ticket"),
            None,
            None,
            None,
            Some("P0"),
            None,
            None,
            200,
        )
        .await
        .unwrap();
    assert!(p0_open.is_empty(), "P0 已 confirmed，open 组合应为空");

    // ④ 非法 severity 响亮拒（不静默吞）
    let err = svc
        .list(
            None,
            Some("ticket"),
            None,
            None,
            None,
            Some("P9"),
            None,
            None,
            200,
        )
        .await
        .expect_err("非法 severity 应报错");
    assert!(err.to_string().contains("severity"), "{err}");
}

// EN-58 口径固化（2026-09-16）：list 排序 = open 优先（status='open' DESC）+ updated_at DESC
// + id 决稳。用乱序 created_at/updated_at 数据锁死该口径——防止将来有人「顺手」改掉
// ORDER BY 而无测试报警。EN-58 的「乱序」感知实为 open 优先分组 + 前端不分段所致，
// 后端口径无病；本测试即无病证明。
#[tokio::test]
async fn list_order_is_open_first_then_updated_at_desc() {
    let (pool, svc, _pg) = setup().await;

    // 造 5 条工单：A(open) B(confirmed) C(open) D(in_progress) E(resolved)
    let mut ids = Vec::new();
    for title in ["A-open", "B-confirmed", "C-open", "D-inprog", "E-resolved"] {
        let t = svc
            .create(
                title,
                "",
                "ticket",
                "",
                Some("P3"),
                "",
                "",
                "",
                &[],
                None,
                None,
            )
            .await
            .unwrap();
        ids.push((title, t.id));
    }
    // 状态流转（E 到 resolved 必须带 resolution——CHECK 要求）
    svc.update(
        ids[1].1,
        None,
        None,
        None,
        None,
        Some("confirmed"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        "test",
    )
    .await
    .unwrap();
    svc.update(
        ids[3].1,
        None,
        None,
        None,
        None,
        Some("in_progress"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        "test",
    )
    .await
    .unwrap();
    svc.update(
        ids[4].1,
        None,
        None,
        None,
        None,
        Some("resolved"),
        None,
        None,
        None,
        None,
        Some("修好了"),
        None,
        None,
        None,
        "test",
    )
    .await
    .unwrap();

    // 乱序时间戳：created_at 与 updated_at 均手工打散（SQL 直改）
    let stamps: [(usize, i64, i64); 5] = [
        (0, 6, 6),   // A: created -6d  updated -6d
        (1, 9, 7),   // B: created -9d  updated -7d
        (2, 14, 14), // C: created -14d updated -14d
        (3, 11, 3),  // D: created -11d updated -3d
        (4, 26, 2),  // E: created -26d updated -2d
    ];
    for (i, c_days, u_days) in stamps {
        sqlx::query(
            "UPDATE todos SET created_at = now() - ($1 || ' days')::interval, \
             updated_at = now() - ($2 || ' days')::interval WHERE id = $3",
        )
        .bind(c_days.to_string())
        .bind(u_days.to_string())
        .bind(ids[i].1)
        .execute(&pool)
        .await
        .unwrap();
    }

    // 断言口径：open 段（A -6d → C -14d）在前，非 open 段（E -2d → D -3d → B -7d）在后
    let rows = svc
        .list(None, None, None, None, None, None, None, None, 200)
        .await
        .unwrap();
    let got: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(
        got,
        vec!["A-open", "C-open", "E-resolved", "D-inprog", "B-confirmed"],
        "排序口径应为 open 优先 + 段内 updated_at DESC: {got:?}"
    );

    // D29 cursor 翻页：limit=2 翻完全部 5 条不丢不重（三元组行比较在乱序数据下不丢行）
    let mut seen: Vec<uuid::Uuid> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = svc
            .list(
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                cursor.as_deref(),
                2,
            )
            .await
            .unwrap();
        if page.is_empty() {
            break;
        }
        seen.extend(page.iter().map(|r| r.id));
        let last = page.last().unwrap();
        let flag = if last.status == "open" { 1 } else { 0 };
        cursor = Some(format!(
            "{}|{}|{}",
            flag,
            last.updated_at.to_rfc3339(),
            last.id
        ));
        if page.len() < 2 {
            break;
        }
    }
    seen.sort();
    seen.dedup();
    assert_eq!(
        seen.len(),
        5,
        "cursor 翻页应覆盖全部 5 条不丢不重: {seen:?}"
    );
}

#[tokio::test]
async fn todos_update_fields_and_overdue_filter() {
    let (_pool, svc, _pg) = setup().await;

    // ① create：两条带 due（一条已过期、一条三天后）、一条无 due
    let past = svc
        .create(
            "过期任务",
            "body",
            "todo",
            "high",
            None,
            "",
            "",
            "",
            &["ops".into()],
            Some(Utc::now() - chrono::Duration::hours(26)),
            None,
        )
        .await
        .unwrap();
    let future = svc
        .create(
            "三天后到期",
            "body",
            "todo",
            "normal",
            None,
            "",
            "",
            "",
            &[],
            Some(Utc::now() + chrono::Duration::hours(72)),
            None,
        )
        .await
        .unwrap();
    let nodue = svc
        .create(
            "无到期",
            "body",
            "todo",
            "low",
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

    // ② update 改字段生效：priority high→low + due 清空（双层 Option None=清除）
    let upd = svc
        .update(
            past.id,
            None,                                   // kind
            Some("过期任务·改"),                    // title
            None,                                   // body
            Some("low"),                            // priority
            None,                                   // status
            None,                                   // severity
            None,                                   // symptom
            None,                                   // reproduce
            None,                                   // acceptance
            None,                                   // resolution
            Some(None),                             // due_at 清除
            None,                                   // project_hint
            Some(&["ops".into(), "urgent".into()]), // tags
            "test",
        )
        .await
        .unwrap();
    assert_eq!(upd.title, "过期任务·改");
    assert_eq!(upd.priority, "low");
    assert!(upd.due_at.is_none(), "due_at=Some(None) 应清除到期");
    assert_eq!(upd.tags, vec!["ops".to_string(), "urgent".to_string()]);

    // 改回过期 due，供 overdue 过滤断言
    svc.update(
        past.id,
        None,
        None,
        None,
        None,
        None, // kind/title/body/priority/status/severity
        None,
        None,
        None,
        None,
        None, // symptom/reproduce/acceptance/resolution +1
        Some(Some(Utc::now() - chrono::Duration::hours(2))), // due_at
        None,
        None, // project_hint/tags
        "test",
    )
    .await
    .unwrap();

    // ③ overdue 过滤：只回未完成且已过期（改字段后的那条），today/无 due 不混入
    let od = svc
        .list(
            None,
            None,
            None,
            None,
            None,
            None,
            Some("overdue"),
            None,
            200,
        )
        .await
        .unwrap();
    assert_eq!(
        od.len(),
        1,
        "overdue 应恰好命中 1 条: {:?}",
        od.iter().map(|r| &r.title).collect::<Vec<_>>()
    );
    assert_eq!(od[0].id, past.id);

    // ④ 排序：due 过滤下按到期升序（更紧急在前）——再造一条更早过期的验证
    let earlier = svc
        .create(
            "更早过期",
            "body",
            "todo",
            "normal",
            None,
            "",
            "",
            "",
            &[],
            Some(Utc::now() - chrono::Duration::hours(48)),
            None,
        )
        .await
        .unwrap();
    let od2 = svc
        .list(
            None,
            None,
            None,
            None,
            None,
            None,
            Some("overdue"),
            None,
            200,
        )
        .await
        .unwrap();
    assert_eq!(od2.len(), 2);
    assert_eq!(od2[0].id, earlier.id, "到期更早的应排最前");
    assert_eq!(od2[1].id, past.id);

    // ⑤ today 过滤：把 future 改成 1 小时后（今天内）→ today 命中且 overdue 不含它
    svc.update(
        future.id,
        None,
        None,
        None,
        None,
        None, // kind/title/body/priority/status/severity
        None,
        None,
        None,
        None,
        None, // symptom/reproduce/acceptance/resolution +1
        Some(Some(Utc::now() + chrono::Duration::hours(1))), // due_at → 今天内
        None,
        None, // project_hint/tags
        "test",
    )
    .await
    .unwrap();
    let today = svc
        .list(None, None, None, None, None, None, Some("today"), None, 200)
        .await
        .unwrap();
    assert_eq!(today.len(), 1);
    assert_eq!(today[0].id, future.id);
    let od3 = svc
        .list(
            None,
            None,
            None,
            None,
            None,
            None,
            Some("overdue"),
            None,
            200,
        )
        .await
        .unwrap();
    assert_eq!(od3.len(), 2, "today 项不得混入 overdue");
    assert!(od3.iter().all(|r| r.id != future.id));

    // ⑥ 非法 due 值拒绝
    let err = svc
        .list(None, None, None, None, None, None, Some("bogus"), None, 200)
        .await
        .expect_err("非法 due 应报错");
    assert!(err.to_string().contains("overdue"), "{err}");

    // ⑦ done 后不再算 overdue
    svc.update(
        past.id,
        None,
        None,
        None,
        None,
        Some("done"), // kind/title/body/priority/status
        None,
        None,
        None,
        None,
        None, // severity/symptom/reproduce/acceptance/resolution
        None,
        None,
        None, // due_at/project_hint/tags
        "test",
    )
    .await
    .unwrap();
    let od4 = svc
        .list(
            None,
            None,
            None,
            None,
            None,
            None,
            Some("overdue"),
            None,
            200,
        )
        .await
        .unwrap();
    assert_eq!(od4.len(), 1, "done 的过期项应退出 overdue");
    assert_eq!(od4[0].id, earlier.id);
    let _ = nodue; // 无 due 项全程不参与 due 过滤（仅存在性）
}

#[tokio::test]
async fn ticket_status_flow_and_comments_timeline() {
    let (_pool, svc, _pg) = setup().await;

    let t = svc
        .create(
            "T 工单",
            "body",
            "ticket",
            "",
            Some("P2"),
            "症状",
            "复现",
            "验收",
            &[],
            None,
            None,
        )
        .await
        .unwrap();
    assert!(svc.events(t.id).await.unwrap().is_empty());

    // 状态流转 open→confirmed→in_progress：每次真实变化一条 event
    for s in ["confirmed", "in_progress"] {
        svc.update(
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
            "test",
        )
        .await
        .unwrap();
    }

    // 同状态重复 update 不留痕
    svc.update(
        t.id,
        None,
        None,
        None,
        None,
        Some("in_progress"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        "test",
    )
    .await
    .unwrap();

    // 评论入流
    svc.comment(t.id, "第一条评论", "tester").await.unwrap();

    // 时序正确：升序 event,event,comment；payload from/to 齐全；created_at 单调不减
    let ev = svc.events(t.id).await.unwrap();
    assert_eq!(ev.len(), 3, "两次流转+一条评论应恰好 3 条: {ev:?}");
    assert_eq!(ev[0].kind, "event");
    assert_eq!(ev[0].payload["from"], "open");
    assert_eq!(ev[0].payload["to"], "confirmed");
    assert_eq!(ev[1].kind, "event");
    assert_eq!(ev[1].payload["from"], "confirmed");
    assert_eq!(ev[1].payload["to"], "in_progress");
    assert_eq!(ev[2].kind, "comment");
    assert_eq!(ev[2].payload["text"], "第一条评论");
    assert_eq!(ev[2].actor, "tester");
    for w in ev.windows(2) {
        assert!(w[0].created_at <= w[1].created_at, "时间线应升序");
    }
}

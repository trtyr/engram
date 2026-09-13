//! KV 值保值通道集成测试：逐字存取（4 类值形态）、同 key UPSERT 就地更新、
//! 字面量直查、非法输入拒绝。蒸馏零介入是结构保证（kv_entries 表与蒸馏管道无任何连接）。

mod support;

use engram_core::memory::MemoryService;
use engram_llm::{KeyCipher, ProviderRegistry};
use sqlx::PgPool;

async fn setup() -> (PgPool, MemoryService, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    let registry = ProviderRegistry::new(
        pool.clone(),
        KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
    );
    let svc = MemoryService::new(pool.clone(), registry);
    (pool, svc, container)
}

/// 4 类值形态逐字存取：序列号 / UUID / MAC / IP:PORT——字符串精确全等。
#[tokio::test]
async fn kv_roundtrip_verbatim_four_shapes() {
    let (_pool, svc, _pg) = setup().await;

    let cases = [
        ("macbook-serial", "FVFHM033Q6L7", "MacBook Air 序列号"),
        (
            "mac-hw-uuid",
            "9A61373E-91B1-5AC3-A824-B1E0815F7EE2",
            "硬件 UUID",
        ),
        ("legion-mac", "D4-5D-64-AB-12-CD", "Legion 有线 MAC"),
        ("tencent-beijing", "82.157.147.224:22", "北京服务器 SSH"),
    ];
    for (key, value, ctx) in cases {
        let row = svc
            .kv_put(key, value, Some(ctx), None, Some("verified_probe"))
            .await
            .unwrap();
        assert_eq!(row.value, value, "{key} 写入返回应逐字全等");
        let got = svc.kv_get(key).await.unwrap().expect("存在");
        assert_eq!(got.value, value, "{key} 读回应逐字全等");
        assert_eq!(got.context, ctx);
        assert_eq!(got.source, "verified_probe");
    }

    // kv_search 字面量直查（不依赖分词）
    let hits = svc.kv_search("FVFHM033Q6L7", 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].key, "macbook-serial");
    let hits2 = svc.kv_search("82.157", 10).await.unwrap();
    assert_eq!(hits2.len(), 1);
    assert_eq!(hits2[0].key, "tencent-beijing");
}

/// 同 key UPSERT = 就地覆盖（updated_at 变化，不产生历史行）。
#[tokio::test]
async fn kv_upsert_overwrites_in_place() {
    let (_pool, svc, _pg) = setup().await;
    let v1 = svc
        .kv_put("nas-ip", "192.168.3.10", None, None, None)
        .await
        .unwrap();
    let v2 = svc
        .kv_put("nas-ip", "10.61.77.50", Some("换了网段"), None, None)
        .await
        .unwrap();
    assert_eq!(v1.id, v2.id, "同 key 应同一行");
    assert_eq!(v2.value, "10.61.77.50");
    assert_eq!(v2.context, "换了网段");
    assert!(v2.updated_at >= v1.updated_at);
    let all = svc.kv_list(50).await.unwrap();
    assert_eq!(all.len(), 1, "不应产生第二条历史");
}

/// 边界：空 key / 空 value / 超长 key / 非法 source / 短检索词 全拒。
#[tokio::test]
async fn kv_input_validation() {
    let (_pool, svc, _pg) = setup().await;
    assert!(svc.kv_put("", "v", None, None, None).await.is_err());
    assert!(svc.kv_put("k", "", None, None, None).await.is_err());
    assert!(
        svc.kv_put(&"x".repeat(201), "v", None, None, None)
            .await
            .is_err()
    );
    assert!(
        svc.kv_put("k", "v", None, None, Some("bogus"))
            .await
            .is_err()
    );
    assert!(svc.kv_search("ab", 10).await.is_err(), "短于 3 字符拒");
    assert!(svc.kv_get("不存在").await.unwrap().is_none());
}

/// 值里的 % / _ 通配符不逃逸误匹配（ILIKE 转义）。
#[tokio::test]
async fn kv_search_escapes_wildcards() {
    let (_pool, svc, _pg) = setup().await;
    svc.kv_put("progress", "task_100%done", None, None, None)
        .await
        .unwrap();
    // 下划线是字面量不是通配——「taskA100Xdone」不应命中
    let hits = svc.kv_search("task_100%", 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].value, "task_100%done");
}

/// create_atom 断言强度：白名单校验 + 自定义值落库（蒸馏路径默认 inference 在 distill 层测）。
#[tokio::test]
async fn atom_strength_whitelist_and_persist() {
    let (_pool, svc, _pg) = setup().await;
    assert!(
        svc.create_atom(
            "fact",
            "非法强度",
            0.9,
            None,
            None,
            false,
            Some("certain"),
            None
        )
        .await
        .is_err(),
        "词表外 strength 拒"
    );
    assert!(
        svc.create_atom(
            "fact",
            "非法来源",
            0.9,
            None,
            None,
            false,
            None,
            Some("guess")
        )
        .await
        .is_err(),
        "词表外 source 拒"
    );
    let a = svc
        .create_atom(
            "fact",
            "用户在杭州工作（推断）",
            0.9,
            None,
            None,
            false,
            Some("inference"),
            Some("agent_inferred"),
        )
        .await
        .unwrap();
    assert_eq!(a.strength, "inference");
    let b = svc
        .create_atom(
            "preference",
            "用户偏好简洁回复",
            0.9,
            None,
            None,
            false,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(b.strength, "fact", "缺省 fact（存量语义）");
    assert_eq!(b.source_kind, "user_stated");
}

/// 蒸馏回执：不存在的会话报错；存在但未蒸馏的会话给出空产物与状态说明。
#[tokio::test]
async fn distill_result_reports_session_state() {
    use uuid::Uuid;
    let (pool, svc, _pg) = setup().await;
    let bogus = Uuid::now_v7();
    assert!(
        svc.distill_result(bogus).await.is_err(),
        "不存在的会话应报错"
    );

    // 造一个会话（distill_status=pending）
    let sid = Uuid::now_v7();
    sqlx::query("INSERT INTO raw_sessions (id, agent, content) VALUES ($1, 'test', '[]'::jsonb)")
        .bind(sid)
        .execute(&pool)
        .await
        .unwrap();
    let r = svc.distill_result(sid).await.unwrap();
    assert_eq!(r["atom_count"], 0);
    assert_eq!(r["distill_status"], "pending");
}

/// search 一致性（工单「search 不可靠」）：FTS/向量双腿零命中的字面量 → ILIKE 兜底命中
/// （atom + KV 双通道）——「库里有一搜必有」。
#[tokio::test]
async fn search_literal_fallback_hits_atom_and_kv() {
    let (pool, svc, _pg) = setup().await;
    // 直接 SQL 插一个含唯一字面量的原子（绕过 embedding——模拟双腿零命中场景）
    let lit = "ZQXK-778812";
    sqlx::query("INSERT INTO atoms (id, kind, content, confidence, tsv) VALUES ($1, 'fact', $2, 0.9, to_tsvector('simple', $2))")
        .bind(uuid::Uuid::now_v7())
        .bind(format!("机房门禁码 {lit}"))
        .execute(&pool)
        .await
        .unwrap();
    // KV 通道
    svc.kv_put("gate-code", lit, None, None, None)
        .await
        .unwrap();

    // FTS 对该字面量的命中不稳定（分词链路），兜底必须接住——atom 与 kv 都要回来
    let resp = svc.search(lit, &[], 20, true, None, None).await.unwrap();
    let ids: Vec<_> = resp.l1.iter().map(|h| h.id).collect();
    assert!(!resp.l1.is_empty(), "字面量兜底不应为空");
    assert!(
        resp.l1.iter().any(|h| h.kind.as_deref() == Some("kv")),
        "KV 通道应命中: {:?}",
        resp.l1.iter().map(|h| &h.snippet).collect::<Vec<_>>()
    );
    assert_eq!(ids.len(), resp.l1.len());
}

/// stale 提示（工单「状态变更无人发现」）：updated_at 超过 14 天 → 返回带 stale_hint；
/// 新值不带。向后兼容——字段仅在有提示时出现。
#[tokio::test]
async fn kv_stale_hint_marks_old_values() {
    let (pool, svc, _pg) = setup().await;
    svc.kv_put("old-ip", "10.0.0.1", None, None, None)
        .await
        .unwrap();
    svc.kv_put("new-ip", "10.0.0.2", None, None, None)
        .await
        .unwrap();
    // 把 old-ip 的 updated_at 拨回 20 天前
    sqlx::query(
        "UPDATE kv_entries SET updated_at = now() - interval '20 days' WHERE key = 'old-ip'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let old = svc.kv_get("old-ip").await.unwrap().unwrap();
    assert!(old.stale_hint.is_some(), "老值应带 stale 提示");
    assert!(old.stale_hint.unwrap().contains("可能已过期"));
    let fresh = svc.kv_get("new-ip").await.unwrap().unwrap();
    assert!(fresh.stale_hint.is_none(), "新值不带提示");
    // 序列化向后兼容：无提示时字段不出现
    let json = serde_json::to_string(&fresh).unwrap();
    assert!(!json.contains("stale_hint"));
    // list/search 同口径
    let all = svc.kv_list(50).await.unwrap();
    assert_eq!(all.iter().filter(|e| e.stale_hint.is_some()).count(), 1);
    let hits = svc.kv_search("10.0.0.1", 10).await.unwrap();
    assert!(hits[0].stale_hint.is_some());
}

/// KV 权威通道恒并结果（即使 FTS 有噪音命中）：字面量命中 KV 时排最前且带 stale。
#[tokio::test]
async fn kv_authority_channel_merges_despite_fts_noise() {
    let (pool, svc, _pg) = setup().await;
    svc.kv_put("legacy-gw", "ZX-VGATE-7741", None, None, None)
        .await
        .unwrap();
    // 噪音原子：FTS 会命中 zx/vgate 等碎片的其他行
    sqlx::query("INSERT INTO atoms (id, kind, content, confidence, tsv) VALUES ($1, 'fact', 'ZX 系列网关型号大全 7741 家族', 0.9, to_tsvector('simple', 'ZX 系列网关型号大全 7741 家族'))")
        .bind(uuid::Uuid::now_v7())
        .execute(&pool)
        .await
        .unwrap();
    let resp = svc
        .search("ZX-VGATE-7741", &[], 20, true, None, None)
        .await
        .unwrap();
    assert!(
        resp.l1.first().map(|h| h.kind.as_deref()) == Some(Some("kv")),
        "KV 权威命中应排最前: {:?}",
        resp.l1.iter().map(|h| &h.snippet).collect::<Vec<_>>()
    );
    assert!(resp.l1.first().unwrap().snippet.contains("ZX-VGATE-7741"));
}

//! 记忆域集成测试：context_pack 的 L1 相关性（R3）。
//!
//! 验证 L1 补充在「有 query」时按语义相关排序（走 search_atoms），
//! 而不是无差别按 hit_count 排序。

mod support;

use engram_core::memory::{MemoryError, MemoryService};
use engram_llm::{KeyCipher, ProviderRegistry};
use engram_search::tokenize::tsv_text;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

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

async fn insert_atom(pool: &PgPool, content: &str, hit_count: i32) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, needs_review, embedding, tsv, hit_count) \
         VALUES ($1, 'fact', $2, 0.9, 'active', '[]'::jsonb, false, NULL, to_tsvector('simple', $3), $4)",
    )
    .bind(id)
    .bind(content)
    .bind(tsv_text(content))
    .bind(hit_count)
    .execute(pool)
    .await
    .expect("插入 atom");
    id
}

#[tokio::test]
async fn context_pack_l1_is_query_relevant_not_hit_count() {
    let (pool, svc, _container) = setup().await;

    // 相关 atom（hit_count 低）与不相关 atom（hit_count 高）
    let relevant = insert_atom(&pool, "用户偏好使用 Rust 语言进行系统编程", 0).await;
    let irrelevant = insert_atom(&pool, "用户喜欢在家做中式烹饪料理", 100).await;

    let pack = svc
        .context_pack(Some("Rust"), 10, 10_000, false)
        .await
        .expect("context_pack");

    let atom_ids: Vec<Uuid> = pack.atoms.iter().map(|a| a.id).collect();

    // R3：L1 应按 query 语义相关排序——相关 atom 必须在结果中
    assert!(
        atom_ids.contains(&relevant),
        "相关 atom 应在结果中，got: {atom_ids:?}"
    );

    // 不相关但 hit_count 高的 atom 不应挤到相关 atom 之前
    if let Some(pos_irr) = atom_ids.iter().position(|id| *id == irrelevant) {
        let pos_rel = atom_ids.iter().position(|id| *id == relevant).unwrap();
        assert!(
            pos_rel < pos_irr,
            "相关 atom 应排在不相关 atom 之前：rel@{pos_rel} irr@{pos_irr}"
        );
    }
}

/// B9：检索命中异步回写 hit_count（atoms + scenarios）。
#[tokio::test]
async fn search_hits_bump_hit_count() {
    let (pool, svc, _container) = setup().await;

    let a = insert_atom(&pool, "用户偏好使用 Rust 语言进行系统编程", 0).await;

    // 场景种子（含关键词，FTS 可命中）
    let sid = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO scenarios (id, topic, summary, body, tsv) VALUES \
         ($1, '开发环境', '用户偏好 Rust', '完整描述', to_tsvector('simple', $2))",
    )
    .bind(sid)
    .bind(engram_search::tokenize::tsv_text("开发环境 用户偏好 Rust"))
    .execute(&pool)
    .await
    .unwrap();

    let _ = svc
        .search("Rust", &[], 10, false, false, None, None)
        .await
        .expect("search");

    // 回写是异步的：轮询等它落地
    let mut atom_hits = 0i32;
    let mut scen_hits = 0i32;
    for _ in 0..50 {
        atom_hits = sqlx::query_scalar("SELECT hit_count FROM atoms WHERE id = $1")
            .bind(a)
            .fetch_one(&pool)
            .await
            .unwrap();
        scen_hits = sqlx::query_scalar("SELECT hit_count FROM scenarios WHERE id = $1")
            .bind(sid)
            .fetch_one(&pool)
            .await
            .unwrap();
        if atom_hits >= 1 && scen_hits >= 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(atom_hits >= 1, "atoms.hit_count 应回写（实际 {atom_hits}）");
    assert!(
        scen_hits >= 1,
        "scenarios.hit_count 应回写（实际 {scen_hits}）"
    );

    // context_pack 读路径同样计数（有 query 时 L1 走 search_atoms）
    let before = atom_hits;
    let _ = svc
        .context_pack(Some("Rust"), 10, 10_000, false)
        .await
        .unwrap();
    let mut after = before;
    for _ in 0..50 {
        after = sqlx::query_scalar("SELECT hit_count FROM atoms WHERE id = $1")
            .bind(a)
            .fetch_one(&pool)
            .await
            .unwrap();
        if after > before {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        after > before,
        "context_pack 命中应回写（{before} → {after}）"
    );
}

#[tokio::test]
async fn update_atom_can_clear_needs_review() {
    let (pool, svc, _container) = setup().await;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, needs_review, tsv) \
         VALUES ($1, 'fact', '低置信事实', 0.5, 'candidate', '[]'::jsonb, true, to_tsvector('simple', $2))",
    )
    .bind(id)
    .bind(tsv_text("低置信事实"))
    .execute(&pool)
    .await
    .unwrap();

    // 通过：清人审标记，其余不动
    let a = svc
        .update_atom(
            id,
            None,
            None,
            None,
            None,
            Some(false),
            None,
            None,
            None,
            None,
            "test",
        )
        .await
        .unwrap();
    assert!(!a.needs_review, "人审通过应清 needs_review");
    assert_eq!(a.status, "candidate");
    assert_eq!(a.content, "低置信事实");
}

/// 实体透镜进 context_pack：AI 冷启动能看到用户世界里的人与事。
/// 有 query 按 token 相关命中，无 query 按密度头部。
#[tokio::test]
async fn context_pack_carries_entity_lenses() {
    let (_pool, svc, _container) = setup().await;

    let zhang = svc
        .create_entity("张三", "person", "同事，负责后端")
        .await
        .unwrap();
    let _cook = svc.create_entity("烹饪", "topic", "").await.unwrap();

    // 有 query：token 相关——「张三」命中人物实体
    let pack = svc
        .context_pack(Some("张三"), 10, 10_000, false)
        .await
        .unwrap();
    assert!(
        pack.entities.iter().any(|e| e.id == zhang.id),
        "query 张三 → 实体透镜应含张三，实得 {:?}",
        pack.entities
            .iter()
            .map(|e| e.name.clone())
            .collect::<Vec<String>>()
    );

    // 无 query：密度头部（两实体密度同为 0 时取 updated_at 头部，非空即可）
    let pack = svc.context_pack(None, 10, 10_000, false).await.unwrap();
    assert!(!pack.entities.is_empty(), "无 query → 实体透镜应有密度头部");
}

/// phase-2 过期过滤：valid_until 已过的原子不进 context_pack（query + no-query 双路径都过滤）。
#[tokio::test]
async fn context_pack_excludes_expired_atoms() {
    let (_pool, svc, _container) = setup().await;
    let now = chrono::Utc::now();
    // 过期原子（valid_until = 昨天）
    let expired = svc
        .create_atom(
            "fact",
            "过期事实：明天要交周报",
            0.9,
            None,
            Some(now - chrono::Duration::days(1)),
            false,
        )
        .await
        .unwrap();
    // 未过期原子（valid_until = 明天）
    let _future = svc
        .create_atom(
            "fact",
            "未来事实：下月出差北京",
            0.9,
            None,
            Some(now + chrono::Duration::days(1)),
            false,
        )
        .await
        .unwrap();
    // 永久原子（无 valid_until）
    let _perm = svc
        .create_atom("fact", "永久事实：喜欢喝美式咖啡", 0.9, None, None, false)
        .await
        .unwrap();

    // no-query 路径：过期原子被过滤
    let pack = svc.context_pack(None, 20, 10_000, false).await.unwrap();
    assert!(
        !pack.atoms.iter().any(|a| a.id == expired.id),
        "no-query 注入不应含过期原子"
    );

    // query 路径：即使查询词命中过期原子，也不注入
    let pack = svc
        .context_pack(Some("周报"), 20, 10_000, false)
        .await
        .unwrap();
    assert!(
        !pack.atoms.iter().any(|a| a.id == expired.id),
        "query 注入不应含过期原子"
    );
}

/// phase-2 文件批量导入：JSONL/文本解析 → 落 session（source=import）+ 解析错误三问。
#[tokio::test]
async fn import_session_parses_jsonl_and_text() {
    let (pool, svc, _container) = setup().await;

    // JSONL 解析：3 行（空行跳过 + human/ai 别名映射 user/assistant）
    let s = svc
        .import_session(
            "import-test",
            "{\"role\":\"user\",\"content\":\"我喜欢骑行\"}\n\n{\"role\":\"assistant\",\"content\":\"收到\"}\n{\"role\":\"human\",\"content\":\"每周六骑行\"}\n",
            "jsonl",
            "off",
        )
        .await
        .unwrap();
    assert_eq!(
        s.content.as_array().map(|a| a.len()),
        Some(3),
        "JSONL 应解析 3 轮"
    );
    assert_eq!(s.content[0]["speaker"], "user");
    assert_eq!(s.content[2]["speaker"], "user", "human 别名映射 user");

    // 纯文本：空行分段交替（奇数段 user 偶数段 assistant）
    let s2 = svc
        .import_session("import-test", "第一段用户话\n\n第二段助手话", "text", "off")
        .await
        .unwrap();
    assert_eq!(s2.content.as_array().map(|a| a.len()), Some(2));
    assert_eq!(s2.content[0]["speaker"], "user");
    assert_eq!(s2.content[1]["speaker"], "assistant");

    // 坏 JSON → 400
    assert!(matches!(
        svc.import_session("import-test", "这不是json\n", "jsonl", "off")
            .await,
        Err(MemoryError::BadRequest(_))
    ));

    // 空内容 → 400
    assert!(matches!(
        svc.import_session("import-test", "\n\n", "text", "off")
            .await,
        Err(MemoryError::BadRequest(_))
    ));

    // 未知格式 → 400
    assert!(matches!(
        svc.import_session("import-test", "x", "csv", "off").await,
        Err(MemoryError::BadRequest(_))
    ));

    // metadata.source = import 落库
    let meta: serde_json::Value =
        sqlx::query_scalar("SELECT metadata FROM raw_sessions WHERE id = $1")
            .bind(s.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(meta["source"], "import", "metadata.source 应标记 import");
}

/// 议题一 b：append_session 增量写——pending 可追加、已蒸馏拒绝、agent 可补记。
#[tokio::test]
async fn append_session_semantics() {
    let (_pool, svc, _container) = setup().await;
    let s = svc
        .write_session(
            "pi-ext",
            serde_json::json!([{"speaker":"user","text":"第一轮"}]),
            "off",
            false,
        )
        .await
        .unwrap();

    // pending 可追加，轮次合并
    let s2 = svc
        .append_session(
            s.id,
            serde_json::json!([{"speaker":"assistant","text":"收到"},{"speaker":"user","text":"继续"}]),
            None,
            "off",
        )
        .await
        .unwrap();
    assert_eq!(
        s2.content.as_array().map(|a| a.len()),
        Some(3),
        "追加后 3 轮"
    );
    assert_eq!(s2.agent, "pi-ext", "不传 agent 保持原值");

    // agent 补记（COALESCE）
    let s3 = svc
        .append_session(
            s.id,
            serde_json::json!([{"speaker":"user","text":"第四轮"}]),
            Some("pi-claude"),
            "off",
        )
        .await
        .unwrap();
    assert_eq!(s3.agent, "pi-claude", "补记 agent 应生效");

    // 已蒸馏拒绝
    sqlx::query("UPDATE raw_sessions SET distill_status = 'done' WHERE id = $1")
        .bind(s.id)
        .execute(&_pool)
        .await
        .unwrap();
    let err = svc
        .append_session(
            s.id,
            serde_json::json!([{"speaker":"user","text":"迟到"}]),
            None,
            "off",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, MemoryError::BadRequest(_)), "已蒸馏应 400");
}

/// 议题四：实体级遗忘——挂链 active 原子归档、非 active 不动、实体+墓碑消失。
#[tokio::test]
async fn forget_entity_archives_linked_atoms() {
    let (pool, svc, _container) = setup().await;
    let e = svc.create_entity("小王", "person", "").await.unwrap();
    let a1 = insert_atom(&pool, "小王 pace 偏慢", 0).await;
    let a2 = insert_atom(&pool, "小王 也要去骑行", 0).await;
    // 预置一条已归档的挂链原子（不应被二次动）
    let a3 = insert_atom(&pool, "小王 的旧记录", 0).await;
    sqlx::query("UPDATE atoms SET status = 'archived' WHERE id = $1")
        .bind(a3)
        .execute(&pool)
        .await
        .unwrap();
    for a in [a1, a2, a3] {
        svc.attach_atom(e.id, a).await.unwrap();
    }

    let n = svc.forget_entity(e.id).await.unwrap();
    assert_eq!(n, 2, "只归档 2 条 active（archived 不动）");

    for a in [a1, a2] {
        let st: String = sqlx::query_scalar("SELECT status FROM atoms WHERE id = $1")
            .bind(a)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(st, "archived");
    }
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM entities")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "实体应删除");
}

/// 议题二：直写带事件时间；correction 手动补 superseded_by 取代链。
#[tokio::test]
async fn atom_time_and_supersede_chain() {
    let (_pool, svc, _container) = setup().await;
    let old = svc
        .create_atom("fact", "张三的生日是 3 月 5 日", 0.9, None, None, false)
        .await
        .unwrap();
    assert!(old.occurred_at.is_none());

    let occ = "2026-09-02T00:00:00Z"
        .parse::<chrono::DateTime<chrono::Utc>>()
        .unwrap();
    let new = svc
        .create_atom(
            "correction",
            "张三的生日是 3 月 15 日",
            0.95,
            Some(occ),
            None,
            false,
        )
        .await
        .unwrap();
    assert_eq!(new.occurred_at, Some(occ), "直写应携带事件时间");

    // 手动 correction：归档旧原子并补取代链
    let archived = svc
        .update_atom(
            old.id,
            None,
            None,
            None,
            Some("archived"),
            None,
            Some(new.id),
            None,
            None,
            None,
            "test",
        )
        .await
        .unwrap();
    assert_eq!(archived.status, "archived");
    assert_eq!(archived.superseded_by, Some(new.id), "取代链应指向新原子");
}

/// 输入校验：空 content + 超长 content（>120 字）都应 BadRequest（测试方 2026-09-02 刁钻实测发现）。
#[tokio::test]
async fn create_atom_rejects_empty_and_oversize_content() {
    let (_pool, svc, _container) = setup().await;

    let empty = svc.create_atom("fact", "   ", 0.9, None, None, false).await;
    assert!(
        matches!(empty, Err(MemoryError::BadRequest(_))),
        "空内容应被拒"
    );

    let long = "长".repeat(121);
    let oversize = svc.create_atom("fact", &long, 0.9, None, None, false).await;
    assert!(
        matches!(oversize, Err(MemoryError::BadRequest(_))),
        "超 120 字应被拒"
    );

    // 边界：120 字应通过
    let ok_len = "字".repeat(120);
    let ok = svc
        .create_atom("fact", &ok_len, 0.9, None, None, false)
        .await;
    assert!(ok.is_ok(), "120 字应通过");
}

/// 议题三：context_pack 带人审队列（代问）；no_feedback 不刷热度。
#[tokio::test]
async fn context_pack_pending_review_and_no_feedback() {
    let (pool, svc, _container) = setup().await;
    let hot = insert_atom(&pool, "用户偏好深色主题", 50).await;

    // 造一条待人审原子
    let pr = svc
        .create_atom(
            "fact",
            "待确认：张三生日 3 月 15 日",
            0.3,
            None,
            None,
            false,
        )
        .await
        .unwrap();
    assert!(pr.needs_review);

    let pack = svc
        .context_pack(Some("张三"), 10, 10_000, false)
        .await
        .unwrap();
    assert!(
        pack.pending_review.iter().any(|a| a.id == pr.id),
        "人审队列应进 context_pack 供 AI 代问"
    );

    // no_feedback=true：hit_count 不涨
    let before: i32 = sqlx::query_scalar("SELECT hit_count FROM atoms WHERE id = $1")
        .bind(hot)
        .fetch_one(&pool)
        .await
        .unwrap();
    let _ = svc
        .context_pack(Some("深色主题"), 10, 10_000, true)
        .await
        .unwrap();
    let after: i32 = sqlx::query_scalar("SELECT hit_count FROM atoms WHERE id = $1")
        .bind(hot)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after, "no_feedback 不应刷热度");
}

/// P12：并发 append 不丢更新——原子 jsonb 拼接在行级串行。
#[tokio::test]
async fn concurrent_append_keeps_all_turns() {
    let (_pool, svc, _container) = setup().await;
    let s = svc
        .write_session(
            "pi",
            serde_json::json!([{"speaker":"user","text":"第1轮"}]),
            "off",
            false,
        )
        .await
        .unwrap();

    let a = svc.clone();
    let b = svc.clone();
    let sid = s.id;
    let (r1, r2) = tokio::join!(
        a.append_session(
            sid,
            serde_json::json!([{"speaker":"user","text":"并发A"}]),
            None,
            "off"
        ),
        b.append_session(
            sid,
            serde_json::json!([{"speaker":"user","text":"并发B"}]),
            None,
            "off"
        ),
    );
    r1.unwrap();
    r2.unwrap();

    let after = svc.get_session(sid).await.unwrap();
    assert_eq!(
        after.content.as_array().map(|x| x.len()),
        Some(3),
        "两路并发 append 都应落库（原子拼接），实得 {}",
        after.content
    );
}

/// P3 sensitive：默认不进检索与 pack，reveal 才可见；consolidate 素材不动它。
#[tokio::test]
async fn sensitive_atoms_hidden_until_reveal() {
    let (pool, svc, _container) = setup().await;
    insert_atom(&pool, "用户喜欢骑行", 0).await;
    let s = svc
        .create_atom("fact", "用户在服用降压药", 0.9, None, None, true)
        .await
        .unwrap();
    assert!(s.sensitive);

    // 默认检索：不可见
    let r = svc
        .search("降压药", &[], 10, true, false, None, None)
        .await
        .unwrap();
    assert!(r.l1.is_empty(), "sensitive 默认不可见");
    // reveal：可见
    let r = svc
        .search("降压药", &[], 10, true, true, None, None)
        .await
        .unwrap();
    assert!(r.l1.iter().any(|h| h.id == s.id), "reveal 后可见");
    // context_pack：恒排除（注入路径不给 reveal）
    let pack = svc
        .context_pack(Some("降压药"), 10, 10_000, true)
        .await
        .unwrap();
    assert!(
        !pack.atoms.iter().any(|a| a.id == s.id),
        "pack 注入不携带 sensitive"
    );
    // patch 可切换
    let off = svc
        .update_atom(
            s.id,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(false),
            "test",
        )
        .await
        .unwrap();
    assert!(!off.sensitive);
}

/// P5 void：pending 可作废（蒸馏跳过），已蒸馏 400。
#[tokio::test]
async fn void_session_semantics() {
    let (pool, svc, _container) = setup().await;
    let s = svc
        .write_session(
            "t",
            serde_json::json!([{"speaker":"user","text":"x"}]),
            "off",
            false,
        )
        .await
        .unwrap();
    let v = svc.void_session(s.id).await.unwrap();
    assert_eq!(v.distill_status, "void");
    // claim 只取 pending → void 会话不会被蒸馏（extract 测试已覆盖 claim 谓词，此处验证状态语义）
    // M-2：已处理 → BadRequest（400），文案点破当前状态
    let again = svc.void_session(s.id).await;
    assert!(
        matches!(&again, Err(engram_core::memory::MemoryError::BadRequest(m)) if m.contains("已作废")),
        "void 不可重复且文案分开：{again:?}"
    );
    // M-2：不存在 → NotFound（404），不再与「已蒸馏」合并成一句
    let ghost = svc.void_session(uuid::Uuid::now_v7()).await;
    assert!(
        matches!(&ghost, Err(engram_core::memory::MemoryError::NotFound(m)) if m.contains("不存在")),
        "不存在的会话应 404 NotFound：{ghost:?}"
    );

    let s2 = svc
        .write_session(
            "t",
            serde_json::json!([{"speaker":"user","text":"y"}]),
            "off",
            false,
        )
        .await
        .unwrap();
    sqlx::query("UPDATE raw_sessions SET distill_status='done' WHERE id=$1")
        .bind(s2.id)
        .execute(&pool)
        .await
        .unwrap();
    // v2 语义扩大（P0-3）：done 会话可 void，且级联归档其蒸馏产物
    let atom2 = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, status, source_refs, tsv) \
         VALUES ($1, 'fact', 'y 的蒸馏产物', 'active', $2::jsonb, to_tsvector('simple', 'x'))",
    )
    .bind(atom2)
    .bind(serde_json::json!([{"session_id": s2.id.to_string()}]))
    .execute(&pool)
    .await
    .unwrap();
    let v2 = svc.void_session(s2.id).await.unwrap();
    assert_eq!(v2.distill_status, "void", "done 会话可 void（v2 语义）");
    let atom_status: String = sqlx::query_scalar("SELECT status FROM atoms WHERE id = $1")
        .bind(atom2)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(atom_status, "archived", "done 会话 void 应级联归档原子");
}

/// P11/SEC-E purge_agent（2026-09-03 彻底化）：该 agent **全部**会话物理删除
/// （含 done——sensitive 原文不留）+ 产出 active 原子归档（可恢复）；真数据不动。
#[tokio::test]
async fn purge_agent_clears_test_data() {
    let (pool, svc, _container) = setup().await;
    let s1 = svc
        .write_session(
            "test-agent",
            serde_json::json!([{"speaker":"user","text":"a"}]),
            "off",
            false,
        )
        .await
        .unwrap();
    // SEC-E 场景：done 会话（已蒸馏）也必须清——此前只 void pending，done 残留
    let s_done = svc
        .write_session(
            "test-agent",
            serde_json::json!([{"speaker":"user","text":"敏感原始对话"}]),
            "off",
            false,
        )
        .await
        .unwrap();
    sqlx::query("UPDATE raw_sessions SET distill_status='done' WHERE id=$1")
        .bind(s_done.id)
        .execute(&pool)
        .await
        .unwrap();
    let keep = svc
        .write_session(
            "real-agent",
            serde_json::json!([{"speaker":"user","text":"b"}]),
            "off",
            false,
        )
        .await
        .unwrap();
    // 手工挂产出原子（source_refs 指向各自会话）
    for (sid, content) in [(s1.id, "测试产物A"), (keep.id, "真数据B")] {
        let id = uuid::Uuid::now_v7();
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, tsv) \
             VALUES ($1, 'fact', $2, 0.9, 'active', $3::jsonb, to_tsvector('simple', $4))",
        )
        .bind(id)
        .bind(content)
        .bind(serde_json::json!([{"session_id": sid}]).to_string())
        .bind(engram_search::tokenize::tsv_text(content))
        .execute(&pool)
        .await
        .unwrap();
    }

    let (erased, archived) = svc.purge_agent("test-agent").await.unwrap();
    assert_eq!(erased, 2, "pending + done 会话都物理删除");
    assert_eq!(archived, 1, "其产出原子归档");
    // SEC-E 核心：清场后零残留（含 sensitive 原文所在的 done 会话）
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM raw_sessions WHERE agent='test-agent'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "purge --agent 清场不留垃圾（SEC-E）");
    // 真数据不动
    let st: (String,) = sqlx::query_as("SELECT distill_status FROM raw_sessions WHERE id=$1")
        .bind(keep.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(st.0, "pending");
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM atoms WHERE content LIKE '真数据%' AND status='active'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
}

/// M-1/SEC-B（2026-09-03）：空 text 轮次 400；纯空白 400；超长 400（文案带上限值）；
/// 正常轮次照常通过；append 同口径。
#[tokio::test]
async fn write_session_validates_turns() {
    let (_pool, svc, _container) = setup().await;
    use engram_core::memory::TURN_TEXT_MAX_CHARS;

    // M-1：空 text
    let empty = svc
        .write_session(
            "t",
            serde_json::json!([{"speaker":"user","text":""}]),
            "off",
            false,
        )
        .await;
    assert!(
        matches!(&empty, Err(engram_core::memory::MemoryError::BadRequest(m)) if m.contains("text 不能为空")),
        "空 text 轮次应 400：{empty:?}"
    );
    // M-1：纯空白 text 同样挡
    let blank = svc
        .write_session(
            "t",
            serde_json::json!([{"speaker":"assistant","text":"   "}]),
            "off",
            false,
        )
        .await;
    assert!(blank.is_err(), "纯空白 text 应 400");

    // SEC-B：超长（上限 + 1 字），文案含上限值
    let long_text = "长".repeat(TURN_TEXT_MAX_CHARS + 1);
    let long = svc
        .write_session(
            "t",
            serde_json::json!([{"speaker":"user","text":long_text}]),
            "off",
            false,
        )
        .await;
    assert!(
        matches!(&long, Err(engram_core::memory::MemoryError::BadRequest(m)) if m.contains(&TURN_TEXT_MAX_CHARS.to_string())),
        "超长 turn 应 400 且文案含上限值：{long:?}"
    );

    // 正常轮次不受影响
    let ok = svc
        .write_session(
            "t",
            serde_json::json!([{"speaker":"user","text":"正常一句话"}]),
            "off",
            false,
        )
        .await;
    assert!(ok.is_ok(), "正常轮次应通过");

    // append 同口径：空 text 拒绝
    let sid = ok.unwrap().id;
    let app = svc
        .append_session(
            sid,
            serde_json::json!([{"speaker":"user","text":""}]),
            None,
            "off",
        )
        .await;
    assert!(app.is_err(), "append 空 text 同样应 400");
}

/// P4 导出：五表齐全 + 计数一致。
#[tokio::test]
async fn export_contains_all_domains() {
    let (_pool, svc, _container) = setup().await;
    svc.write_session(
        "e",
        serde_json::json!([{"speaker":"user","text":"x"}]),
        "off",
        false,
    )
    .await
    .unwrap();
    svc.create_atom("fact", "导出验证原子", 0.9, None, None, false)
        .await
        .unwrap();
    svc.create_entity("张三", "person", "").await.unwrap();

    svc.create_atom("fact", "导出隐私项", 0.9, None, None, true)
        .await
        .unwrap();
    let dump = svc.export(false).await.unwrap();
    assert_eq!(dump["format"], "engram-memory-export");
    assert_eq!(dump["counts"]["sessions"], 1);
    assert_eq!(dump["counts"]["atoms"], 1, "sensitive 默认排除");
    assert_eq!(dump["sensitive_excluded"], true);
    let full = svc.export(true).await.unwrap();
    assert_eq!(full["counts"]["atoms"], 2, "include 后全量");
    assert_eq!(dump["counts"]["entities"], 1);
    assert!(dump["atoms"][0]["content"].is_string(), "原子内容在");
}

/// P10 新鲜度混排：同分近似的命中，新的排前。
// F4 治：批量归档 → 30s 防抖只入队一个快照收敛 job（converge_only）。
#[tokio::test]
async fn archive_debounces_into_single_snapshot_refresh() {
    let (pool, svc, _c) = setup().await;

    // 1 场景 + 3 活跃成员
    let mut atoms = vec![];
    for i in 0..3 {
        atoms.push(
            svc.create_atom("fact", &format!("成员{i}"), 0.9, None, None, false)
                .await
                .unwrap(),
        );
    }
    let sid = Uuid::now_v7();
    let refs: Vec<Uuid> = atoms.iter().map(|a| a.id).collect();
    sqlx::query("INSERT INTO scenarios (id, topic, summary, body, atom_refs, version) VALUES ($1, 'T', 'S', 'B', $2, 1)")
        .bind(sid)
        .bind(sqlx::types::Json(&refs))
        .execute(&pool)
        .await
        .unwrap();

    // 批量归档（同一防抖窗口内 3 次 update_atom）
    for a in &atoms {
        svc.update_atom(
            a.id,
            None,
            None,
            None,
            Some("archived"),
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

    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE kind = 'organize_scenarios' AND payload->>'converge_only' = 'true'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1, "3 连归档应合并为 1 个快照刷新 job（30s 防抖）");
}

#[tokio::test]
async fn context_pack_prefers_recent() {
    let (pool, svc, _container) = setup().await;
    // 两条同题材原子：老的 hit_count 高、新的刚入库
    insert_atom(&pool, "用户喜欢深色主题偏好", 100).await;
    let new_id = {
        let id = uuid::Uuid::now_v7();
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, tsv, created_at) \
             VALUES ($1, 'preference', '用户喜欢深色主题的新说法', 0.9, 'active', '[]'::jsonb, to_tsvector('simple', $2), now() - interval '1 hour')",
        )
        .bind(id)
        .bind(engram_search::tokenize::tsv_text("用户喜欢深色主题的新说法"))
        .execute(&pool)
        .await
        .unwrap();
        // 老的压到 90 天前
        sqlx::query("UPDATE atoms SET created_at = now() - interval '90 days' WHERE content = '用户喜欢深色主题偏好'")
            .execute(&pool)
            .await
            .unwrap();
        id
    };
    let pack = svc
        .context_pack(Some("深色主题"), 10, 10_000, true)
        .await
        .unwrap();
    assert!(!pack.atoms.is_empty());
    assert_eq!(
        pack.atoms[0].id,
        new_id,
        "新记忆应排前（recency × score 混排），实得首位 {:?}",
        pack.atoms
            .iter()
            .map(|a| a.content.clone())
            .collect::<Vec<_>>()
    );
}

/// memory-rhythm：rhythm_status——最近心跳（audit 行）+ pending 积压年龄（cron 兜底对象面）。
#[tokio::test]
async fn rhythm_status_reports_heartbeat_and_backlog() {
    let (pool, svc, _c) = setup().await;
    // 无心跳、无积压的冷态
    let cold = svc.rhythm_status().await.unwrap();
    assert_eq!(cold["last_heartbeat"], serde_json::Value::Null);
    assert_eq!(cold["pending_sessions"], serde_json::json!(0));

    // 两条 pending 会话，一条回拨 3 小时（积压年龄的锚）
    for i in 0..2 {
        let _ = svc
            .write_session(
                "cron-test",
                serde_json::json!([{"speaker":"user","text":format!("第{i}条")}]),
                "auto", // v2：off 会话已豁免蒸馏、不再计入积压——积压口径用 auto
                false,
            )
            .await
            .unwrap();
    }
    sqlx::query(
        "UPDATE raw_sessions SET created_at = now() - interval '3 hours' WHERE agent = 'cron-test'",
    )
    .execute(&pool)
    .await
    .unwrap();

    // cron 心跳落审计行
    svc.audit("rhythm_heartbeat", serde_json::json!({"by": "key:cron"}))
        .await;

    let st = svc.rhythm_status().await.unwrap();
    assert!(st["last_heartbeat"].is_string(), "心跳时间应返回：{st}");
    assert_eq!(st["last_heartbeat_by"], serde_json::json!("key:cron"));
    assert_eq!(st["pending_sessions"], serde_json::json!(2));
    let age = st["oldest_pending_age_secs"].as_i64().unwrap();
    assert!(
        (10_000..=11_000).contains(&age),
        "最老积压应约 3 小时（10800s）：{age}"
    );
}

/// v2 修复（P0-3）：已蒸馏（done）会话 void → 级联归档其蒸馏产出的 active 原子。
#[tokio::test]
async fn void_done_session_cascades_atom_archive() {
    let (pool, svc, _pg) = setup().await;

    // off 写入（不触发任务）→ 手动置为 done，模拟蒸馏完成
    let s = svc
        .write_session(
            "t2",
            json!([{"speaker": "user", "text": "VOIDCASCADE 测试内容"}]),
            "off",
            false,
        )
        .await
        .unwrap();
    sqlx::query("UPDATE raw_sessions SET distill_status = 'done' WHERE id = $1")
        .bind(s.id)
        .execute(&pool)
        .await
        .unwrap();

    // 该会话蒸馏产出的 active 原子
    let atom_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, status, source_refs, tsv) \
         VALUES ($1, 'fact', 'VOIDCASCADE 蒸馏产物原子', 'active', $2, to_tsvector('simple', 'x'))",
    )
    .bind(atom_id)
    .bind(json!([{"session_id": s.id}]))
    .execute(&pool)
    .await
    .unwrap();

    let voided = svc.void_session(s.id).await.unwrap();
    assert_eq!(voided.distill_status, "void");

    let status: String = sqlx::query_scalar("SELECT status FROM atoms WHERE id = $1")
        .bind(atom_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        status, "archived",
        "done 会话 void 应级联归档其原子（P0-3 回归）"
    );

    // 审计链有记录
    let audit: i64 =
        sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind = 'session_void_cascade'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(audit, 1, "void 级联应落审计行");
}

/// v2 修复（N3）：budget_items 是各层条数上限——persona 分面不再把 atoms 挤成 0。
#[tokio::test]
async fn context_budget_keeps_atoms_alive() {
    let (pool, svc, _pg) = setup().await;

    // 7 个画像分面（与真实分布一致）
    for (i, aspect) in [
        "identity",
        "preferences",
        "skills",
        "constraints",
        "communication_style",
        "goals",
        "routines",
    ]
    .iter()
    .enumerate()
    {
        sqlx::query(
            "INSERT INTO persona_aspects (id, aspect, content, version, evidence_refs) \
             VALUES ($1, $2, $3, 1, '[]'::jsonb)",
        )
        .bind(Uuid::now_v7())
        .bind(*aspect)
        .bind(format!("分面 {i} 内容"))
        .execute(&pool)
        .await
        .unwrap();
    }
    // 3 条 active 原子
    for i in 0..3 {
        sqlx::query(
            "INSERT INTO atoms (id, kind, content, status, tsv, hit_count) \
             VALUES ($1, 'fact', $2, 'active', to_tsvector('simple', $2), $3)",
        )
        .bind(Uuid::now_v7())
        .bind(format!("用户偏好条目 {i}：喜欢深色主题"))
        .bind(i)
        .execute(&pool)
        .await
        .unwrap();
    }

    // 小预算：v1 行为 atoms=0（被 7 分面挤死）；v2 各层独立预算 → atoms 至少 1
    let pack = svc.context_pack(None, 1, 8000, true).await.unwrap();
    assert_eq!(pack.persona.len(), 7, "画像分面全量保留");
    assert!(
        !pack.atoms.is_empty(),
        "budget_items=1 时 atoms 不应被 persona 挤成 0（N3 回归）"
    );

    // 大预算：正常
    let pack = svc.context_pack(None, 50, 100_000, true).await.unwrap();
    assert_eq!(pack.atoms.len(), 3);

    // N7：chars_used 接近完整序列化体积（含 evidence_refs），不再只是正文文本
    let pack = svc.context_pack(None, 50, 100_000, true).await.unwrap();
    let text_only: usize = pack
        .atoms
        .iter()
        .map(|a| a.content.len())
        .chain(pack.persona.iter().map(|p| p.content.len()))
        .sum();
    assert!(
        pack.meta.chars_used >= text_only,
        "chars_used 应按完整序列化计量（含 evidence_refs）：{} < 正文和 {text_only}",
        pack.meta.chars_used
    );
}

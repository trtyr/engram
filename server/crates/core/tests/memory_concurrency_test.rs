//! 记忆域并发正确性测试（P019-M4）：
//! create_entity 并发同名——双方幂等成功（败方 ON CONFLICT DO NOTHING 后回查返回胜方）；
//! append_session 在会话已蒸馏后被拒（repo 层 distill_status 守卫）。

mod support;

use engram_core::memory::MemoryService;
use engram_llm::{KeyCipher, ProviderRegistry};
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

#[tokio::test]
async fn concurrent_create_entity_is_idempotent() {
    let (pool, svc, _pg) = setup().await;
    let s1 = svc.clone();
    let s2 = svc.clone();
    let (a, b) = tokio::join!(
        s1.create_entity("并发实体", "topic", ""),
        s2.create_entity("并发实体", "topic", ""),
    );
    let ea = a.expect("并发创建方 A 应成功");
    let eb = b.expect("并发创建方 B 应幂等成功（不 500）");
    assert_eq!(ea.id, eb.id, "双方应返回同一实体");

    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM entities WHERE lower(btrim(name)) = lower('并发实体')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1, "同名同类活体只许一个");
}

#[tokio::test]
async fn append_after_distilled_rejected() {
    let (pool, _svc, _pg) = setup().await;
    let sid = Uuid::now_v7();
    sqlx::query("INSERT INTO raw_sessions (id, agent, content, distill_status) VALUES ($1,'t',$2::jsonb,'done')")
        .bind(sid)
        .bind(r#"[{"speaker":"user","text":"旧"}]"#)
        .execute(&pool)
        .await
        .unwrap();
    let r = engram_storage::repo::memory::append_session_update(
        &pool,
        sid,
        &serde_json::json!([{"speaker":"user","text":"新"}]),
        None,
    )
    .await;
    assert!(r.is_err(), "done 会话追加应被 repo 守卫拒绝");
}

//! assets 别名唯一性 DB 兑底（P019-M4 · 0076 触发器）：
//! 并发窗口外的重复别名（应用层预检漏网）必须被 DB 拒绝——大小写不敏感，含与 name 冲突。

mod support;

use sqlx::PgPool;

async fn setup() -> PgPool {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    pool
}

#[tokio::test]
async fn duplicate_alias_across_assets_rejected() {
    let pool = setup().await;
    sqlx::query("INSERT INTO assets (id, kind, name, aliases) VALUES ($1,'host','mac-a',ARRAY['trtyr-mac'])")
        .bind(uuid::Uuid::now_v7())
        .execute(&pool)
        .await
        .unwrap();

    // 另一资产同别名（不同大小写）→ 触发器拒绝
    let r = sqlx::query("INSERT INTO assets (id, kind, name, aliases) VALUES ($1,'host','mac-b',ARRAY['Trtyr-MAC'])")
        .bind(uuid::Uuid::now_v7())
        .execute(&pool)
        .await;
    assert!(r.is_err(), "跨资产重复别名应被 DB 拒绝");
}

#[tokio::test]
async fn alias_conflicting_with_other_name_rejected() {
    let pool = setup().await;
    sqlx::query("INSERT INTO assets (id, kind, name) VALUES ($1,'host','primary-box')")
        .bind(uuid::Uuid::now_v7())
        .execute(&pool)
        .await
        .unwrap();

    let r = sqlx::query("INSERT INTO assets (id, kind, name, aliases) VALUES ($1,'host','other-box',ARRAY['PRIMARY-BOX'])")
        .bind(uuid::Uuid::now_v7())
        .execute(&pool)
        .await;
    assert!(r.is_err(), "别名与它资产 name 冲突应被 DB 拒绝");
}

#[tokio::test]
async fn duplicate_within_own_list_rejected_but_unique_ok() {
    let pool = setup().await;
    // 包内重复
    let r = sqlx::query("INSERT INTO assets (id, kind, name, aliases) VALUES ($1,'host','self-dup',ARRAY['dup','DUP'])")
        .bind(uuid::Uuid::now_v7())
        .execute(&pool)
        .await;
    assert!(r.is_err(), "包内重复别名应被 DB 拒绝");

    // 唯一别名正常落库；自己保留自己的 name 不算冲突
    sqlx::query("INSERT INTO assets (id, kind, name, aliases) VALUES ($1,'host','ok-box',ARRAY['ok-alias','OK-BOX'])")
        .bind(uuid::Uuid::now_v7())
        .execute(&pool)
        .await
        .unwrap_or_else(|e| panic!("唯一别名应放行: {e}"));
}

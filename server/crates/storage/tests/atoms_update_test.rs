//! T004：update_atom_full 单值字段三态语义——None=不动 / Some(None)=清空 /
//! Some(Some(v))=设置。回归「单值字段传 None 永远清不掉」（EN-BUG-1 同族、方向相反）。

mod support;

use chrono::Utc;
use uuid::Uuid;

async fn setup() -> (sqlx::PgPool, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    (pool, container)
}

async fn seed_full_atom(pool: &sqlx::PgPool) -> (Uuid, Uuid) {
    let id = Uuid::now_v7();
    let occurred = Utc::now();
    let valid = Utc::now();
    let sup = Uuid::now_v7();
    // superseded_by 有 FK——被指向的原子先落
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, status, confidence, tsv) \
         VALUES ($1, 'fact', '旧版本原子', 'archived', 0.9, to_tsvector('simple', '旧版本原子'))",
    )
    .bind(sup)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, status, confidence, occurred_at, valid_until, \
         superseded_by, sensitive, tsv) \
         VALUES ($1, 'fact', '种子原子', 'active', 0.9, $2, $3, $4, TRUE, \
         to_tsvector('simple', '种子原子'))",
    )
    .bind(id)
    .bind(occurred)
    .bind(valid)
    .bind(sup)
    .execute(pool)
    .await
    .unwrap();
    (id, sup)
}

type AtomClearables = (
    Option<chrono::DateTime<Utc>>,
    Option<chrono::DateTime<Utc>>,
    Option<Uuid>,
    bool,
);

async fn row(pool: &sqlx::PgPool, id: Uuid) -> AtomClearables {
    sqlx::query_as(
        "SELECT occurred_at, valid_until, superseded_by, sensitive FROM atoms WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn t004_three_state_clearable_fields() {
    let (pool, _pg) = setup().await;
    let (id, some_sup) = seed_full_atom(&pool).await;

    // 基线：四字段全有值
    let (oa0, vu0, sb0, se0) = row(&pool, id).await;
    assert!(oa0.is_some() && vu0.is_some() && sb0.is_some() && se0);

    // ① None = 不动
    engram_storage::repo::memory::update_atom_full(
        &pool,
        id,
        "种子原子",
        0.9,
        "active",
        None,
        None,
        None,
        None,
        None,
        None,
        "种子原子",
        "fact",
    )
    .await
    .unwrap();
    let (oa1, vu1, sb1, se1) = row(&pool, id).await;
    assert_eq!(oa1, oa0, "None 不得清 occurred_at");
    assert_eq!(vu1, vu0, "None 不得清 valid_until");
    assert_eq!(sb1, sb0, "None 不得清 superseded_by");
    assert!(se1, "None 不得清 sensitive");

    // ② Some(Some(v)) = 设置
    let new_oa = Utc::now();
    engram_storage::repo::memory::update_atom_full(
        &pool,
        id,
        "种子原子",
        0.9,
        "active",
        None,
        Some(Some(some_sup)),
        Some(Some(new_oa)),
        Some(Some(new_oa)),
        Some(Some(false)),
        None,
        "种子原子",
        "fact",
    )
    .await
    .unwrap();
    let (oa2, vu2, sb2, se2) = row(&pool, id).await;
    assert_eq!(oa2, Some(new_oa), "显式设置 occurred_at");
    assert_eq!(vu2, Some(new_oa), "显式设置 valid_until");
    assert_eq!(sb2, Some(some_sup), "显式设置 superseded_by");
    assert!(!se2, "显式设置 sensitive=false");

    // ③ Some(None) = 清空（sensitive 因 NOT NULL 落 false）
    engram_storage::repo::memory::update_atom_full(
        &pool,
        id,
        "种子原子",
        0.9,
        "active",
        None,
        Some(None),
        Some(None),
        Some(None),
        Some(Some(true)),
        None,
        "种子原子",
        "fact",
    )
    .await
    .unwrap();
    let (oa3, vu3, sb3, se3) = row(&pool, id).await;
    assert_eq!(oa3, None, "显式 null 应清空 occurred_at");
    assert_eq!(vu3, None, "显式 null 应清空 valid_until");
    assert_eq!(sb3, None, "显式 null 应清空 superseded_by");
    assert!(se3, "sensitive 显式设置 true");
}

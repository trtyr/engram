//! study 仓储单测（P007-T002）：track/item CRUD + 状态机 learned_at 语义 + 进度统计。

mod support;

use uuid::Uuid;

async fn setup() -> sqlx::PgPool {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    pool
}

#[tokio::test]
async fn track_crud_roundtrip() {
    let pool = setup().await;
    let id = Uuid::now_v7();

    engram_storage::repo::study::track_create(&pool, id, "RAG 入门", "掌握到能设计切分管线")
        .await
        .unwrap();

    let t = engram_storage::repo::study::track_get(&pool, id)
        .await
        .unwrap()
        .expect("应存在");
    assert_eq!(t.name, "RAG 入门");
    assert_eq!(t.status, "active", "默认 active");

    // 补丁更新：只改 status，name/goal 不动
    engram_storage::repo::study::track_update(&pool, id, None, None, Some("paused"))
        .await
        .unwrap();
    let t = engram_storage::repo::study::track_get(&pool, id).await.unwrap().unwrap();
    assert_eq!(t.status, "paused");
    assert_eq!(t.goal, "掌握到能设计切分管线", "未更新字段不动");

    // list
    let all = engram_storage::repo::study::track_list(&pool).await.unwrap();
    assert!(all.iter().any(|t| t.id == id));

    // delete（级联 items）
    engram_storage::repo::study::track_delete(&pool, id).await.unwrap();
    assert!(engram_storage::repo::study::track_get(&pool, id).await.unwrap().is_none());
}

#[tokio::test]
async fn item_status_machine_learned_at_semantics() {
    let pool = setup().await;
    let track = Uuid::now_v7();
    engram_storage::repo::study::track_create(&pool, track, "T", "").await.unwrap();

    let item = Uuid::now_v7();
    engram_storage::repo::study::item_create(&pool, item, track, "切分策略", 1)
        .await
        .unwrap();

    // 初始 not_started，learned_at NULL
    let it = engram_storage::repo::study::item_get(&pool, item).await.unwrap().unwrap();
    assert_eq!(it.status, "not_started");
    assert!(it.learned_at.is_none());

    // → learning：learned_at 仍 NULL
    engram_storage::repo::study::item_update(&pool, item, None, Some("learning"), None, None, None)
        .await
        .unwrap();
    let it = engram_storage::repo::study::item_get(&pool, item).await.unwrap().unwrap();
    assert_eq!(it.status, "learning");
    assert!(it.learned_at.is_none(), "learning 不记 learned_at");

    // → learned：learned_at 写入
    engram_storage::repo::study::item_update(&pool, item, None, Some("learned"), None, None, None)
        .await
        .unwrap();
    let it = engram_storage::repo::study::item_get(&pool, item).await.unwrap().unwrap();
    assert_eq!(it.status, "learned");
    assert!(it.learned_at.is_some(), "learned 应记时间");

    // 离开 learned（回 learning）：learned_at 清空
    engram_storage::repo::study::item_update(&pool, item, None, Some("learning"), None, None, None)
        .await
        .unwrap();
    let it = engram_storage::repo::study::item_get(&pool, item).await.unwrap().unwrap();
    assert!(it.learned_at.is_none(), "离开 learned 应清空");

    // links 字段
    engram_storage::repo::study::item_update(
        &pool,
        item,
        None,
        None,
        None,
        Some(&serde_json::json!(["fixed-chunking", "recursive-chunking"])),
        Some(&serde_json::json!(["01a0f7e6-03e5-76e3-9146-6909bea09f70"])),
    )
    .await
    .unwrap();
    let it = engram_storage::repo::study::item_get(&pool, item).await.unwrap().unwrap();
    assert_eq!(it.wiki_slugs.as_array().unwrap().len(), 2);
    assert_eq!(it.doc_ids.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn items_order_and_progress() {
    let pool = setup().await;
    let track = Uuid::now_v7();
    engram_storage::repo::study::track_create(&pool, track, "T", "").await.unwrap();

    // 乱序 position 创建
    for (name, pos) in [("c", 3), ("a", 1), ("b", 2)] {
        engram_storage::repo::study::item_create(&pool, Uuid::now_v7(), track, name, pos)
            .await
            .unwrap();
    }
    let items = engram_storage::repo::study::items_by_track(&pool, track).await.unwrap();
    let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["a", "b", "c"], "按 position ASC");

    // 进度：2 个 learned
    for it in items.iter().take(2) {
        engram_storage::repo::study::item_update(&pool, it.id, None, Some("learned"), None, None, None)
            .await
            .unwrap();
    }
    let (total, learned) = engram_storage::repo::study::track_progress(&pool, track).await.unwrap();
    assert_eq!((total, learned), (3, 2));

    // 级联删除
    engram_storage::repo::study::track_delete(&pool, track).await.unwrap();
    let left = engram_storage::repo::study::items_by_track(&pool, track).await.unwrap();
    assert!(left.is_empty(), "track 删除应级联 items");
}

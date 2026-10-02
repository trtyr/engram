//! StudyService 单测（P007-T003）：topic_get 核心契约（一次拿全进度/下一步/进行中）+ 状态机 + 错误路径。

mod support;

use engram_core::study::StudyService;
use uuid::Uuid;

async fn setup() -> (sqlx::PgPool, StudyService, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    (pool.clone(), StudyService::new(pool), container)
}

#[tokio::test]
async fn topic_get_full_contract() {
    let (_pool, svc, _pg) = setup().await;

    // 开题
    let topic = svc.topic_create("RAG 入门", "掌握到能设计切分管线").await.unwrap();

    // 空态：进度 0/0，next_up 空
    let full = svc.topic_get(topic).await.unwrap().unwrap();
    assert_eq!(full.track.name, "RAG 入门");
    assert_eq!(full.track.goal, "掌握到能设计切分管线");
    assert_eq!(full.track.status, "active");
    assert_eq!(full.progress.total, 0);
    assert_eq!(full.progress.learned, 0);

    // 加三个知识点（乱序 position，验证排序）
    let a = svc.item_add(topic, "基础流程", Some(10)).await.unwrap();
    let b = svc.item_add(topic, "切分策略", Some(20)).await.unwrap();
    let c = svc.item_add(topic, "嵌入与向量检索", None).await.unwrap(); // 缺省排尾 = 30
    let full = svc.topic_get(topic).await.unwrap().unwrap();
    assert_eq!(full.progress.total, 3);
    let names: Vec<&str> = full.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["基础流程", "切分策略", "嵌入与向量检索"], "position 升序");
    assert_eq!(full.next_up.len(), 3, "全 not_started 都在 next_up");

    // 状态机：a learned、b learning → next_up 只剩 c，in_progress = [b]
    svc.item_set_status(a, "learned").await.unwrap();
    svc.item_set_status(b, "learning").await.unwrap();
    let full = svc.topic_get(topic).await.unwrap().unwrap();
    assert_eq!(full.progress.learned, 1);
    assert_eq!(full.next_up.len(), 1);
    assert_eq!(full.next_up[0].id, c);
    assert_eq!(full.in_progress.len(), 1);
    assert_eq!(full.in_progress[0].id, b);
}

#[tokio::test]
async fn item_add_appends_to_tail_by_default() {
    let (_pool, svc, _pg) = setup().await;
    let topic = svc.topic_create("T", "").await.unwrap();
    let _a = svc.item_add(topic, "a", Some(10)).await.unwrap();
    let _b = svc.item_add(topic, "b", Some(20)).await.unwrap();
    let c = svc.item_add(topic, "c", None).await.unwrap(); // 缺省 = 20+10 = 30
    let full = svc.topic_get(topic).await.unwrap().unwrap();
    assert_eq!(full.items.last().unwrap().id, c);
    assert_eq!(full.items.last().unwrap().position, 30);
}

#[tokio::test]
async fn error_paths() {
    let (_pool, svc, _pg) = setup().await;

    // 空 topic 名
    assert!(svc.topic_create("  ", "").await.is_err());
    // 不存在 topic
    let ghost = Uuid::now_v7();
    assert!(matches!(
        svc.topic_update(ghost, None, None, Some("done")).await,
        Err(engram_core::study::StudyError::NotFound(_))
    ));
    assert!(svc.item_add(ghost, "x", None).await.is_err());
    // 非法 status
    let topic = svc.topic_create("T", "").await.unwrap();
    assert!(matches!(
        svc.topic_update(topic, None, None, Some("bogus")).await,
        Err(engram_core::study::StudyError::BadRequest(_))
    ));
    let item = svc.item_add(topic, "n", None).await.unwrap();
    assert!(svc.item_set_status(item, "mastered").await.is_err(), "非三态拒绝");
    // 不存在 item
    assert!(svc.item_set_status(Uuid::now_v7(), "learned").await.is_err());
}

#[tokio::test]
async fn topic_update_and_archive() {
    let (_pool, svc, _pg) = setup().await;
    let topic = svc.topic_create("T", "旧目标").await.unwrap();

    // 补丁更新：goal+status 一起，name 不动
    svc.topic_update(topic, None, Some("新目标"), Some("paused")).await.unwrap();
    let full = svc.topic_get(topic).await.unwrap().unwrap();
    assert_eq!(full.track.name, "T");
    assert_eq!(full.track.goal, "新目标");
    assert_eq!(full.track.status, "paused");

    // 归档
    svc.topic_update(topic, None, None, Some("done")).await.unwrap();
    let full = svc.topic_get(topic).await.unwrap().unwrap();
    assert_eq!(full.track.status, "done", "归档后数据保留（产出物留 wiki）");

    // 删除
    svc.topic_delete(topic).await.unwrap();
    assert!(svc.topic_get(topic).await.unwrap().is_none());
}

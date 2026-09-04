//! 实体（记忆星系）集成测试：建/挂/密度/共现图/合并全链路。

mod support;

use engram_core::memory::MemoryService;
use engram_llm::{KeyCipher, ProviderRegistry};
use engram_search::tokenize::tsv_text;
use sqlx::PgPool;
use uuid::Uuid;

async fn setup() -> (PgPool, MemoryService, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool)
        .await
        .expect("迁移");
    let registry = ProviderRegistry::new(
        pool.clone(),
        KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap(),
    );
    let svc = MemoryService::new(pool.clone(), registry);
    (pool, svc, container)
}

async fn insert_atom(pool: &PgPool, content: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO atoms (id, kind, content, confidence, status, source_refs, needs_review, embedding, tsv) \
         VALUES ($1, 'fact', $2, 0.9, 'active', '[]'::jsonb, false, NULL, to_tsvector('simple', $3))",
    )
    .bind(id)
    .bind(content)
    .bind(tsv_text(content))
    .execute(pool)
    .await
    .expect("插入 atom");
    id
}

#[tokio::test]
async fn entity_lifecycle_create_attach_graph_merge() {
    let (pool, svc, _container) = setup().await;

    // 建：两个人物 + 一个项目
    let zhang = svc
        .create_entity("张三", "person", "同事，负责后端")
        .await
        .unwrap();
    let project = svc
        .create_entity("Engram", "project", "主力项目")
        .await
        .unwrap();
    assert_eq!(zhang.atom_count, 0);
    assert_eq!(zhang.summary, "同事，负责后端");

    // 同名同类拒重
    let dup = svc.create_entity("张三", "person", "").await;
    assert!(dup.is_err(), "同名同类活体应拒重");
    // 同名不同类允许（张三 既是 person 也可以是 topic？——按约束允许）
    let topic_zhang = svc.create_entity("张三", "topic", "").await;
    assert!(topic_zhang.is_ok());

    // 挂：3 条原子挂张三，其中 2 条同时挂项目（共现）
    let a1 = insert_atom(&pool, "用户和张三讨论了 Engram 的后端架构").await;
    let a2 = insert_atom(&pool, "张三建议用户用 Rust 重写 Engram 索引层").await;
    let a3 = insert_atom(&pool, "用户和张三每周一对齐一次").await;
    for (atom, entities) in [
        (a1, vec![zhang.id, project.id]),
        (a2, vec![zhang.id, project.id]),
        (a3, vec![zhang.id]),
    ] {
        for eid in entities {
            svc.attach_atom(eid, atom).await.unwrap();
            // 幂等：重复挂不报错不重复
            svc.attach_atom(eid, atom).await.unwrap();
        }
    }

    // 列表：密度排序（张三 3 > 项目 2）
    let list = svc.list_entities(Some("person")).await.unwrap();
    assert_eq!(list[0].id, zhang.id);
    assert_eq!(list[0].atom_count, 3);
    let all = svc.list_entities(None).await.unwrap();
    assert_eq!(all.len(), 3);

    // 详情：原子时间线 + 关联场景（无场景时空数组）
    let detail = svc.get_entity(zhang.id).await.unwrap();
    assert_eq!(detail.atoms.len(), 3);
    assert!(detail.scenarios.is_empty());

    // 图：共现边 张三—Engram weight=2
    let graph = svc.entity_graph().await.unwrap();
    assert_eq!(graph.nodes.len(), 3);
    let edge = graph
        .edges
        .iter()
        .find(|e| (e.a == zhang.id && e.b == project.id) || (e.a == project.id && e.b == zhang.id))
        .expect("应存在张三—Engram 共现边");
    assert_eq!(edge.weight, 2);

    // 摘除
    svc.detach_atom(zhang.id, a3).await.unwrap();
    assert_eq!(svc.get_entity(zhang.id).await.unwrap().entity.atom_count, 2);

    // 合并：topic 张三并入 person 张三（moved=0，无关联）——反向验证 moved 计数
    let moved = svc
        .merge_entities(topic_zhang.unwrap().id, zhang.id)
        .await
        .unwrap();
    assert_eq!(moved, 0);
    // 合并后 from 不在活体列表，名字让位
    let after = svc.list_entities(None).await.unwrap();
    assert_eq!(after.len(), 2);
    // 让位后可重建同名 topic
    assert!(svc.create_entity("张三", "topic", "").await.is_ok());

    // 删实体不删原子
    svc.delete_entity(project.id).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM atoms")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 3);
}

#[tokio::test]
async fn merge_moves_atom_links_and_counts() {
    let (pool, svc, _container) = setup().await;
    let winner = svc.create_entity("李四", "person", "").await.unwrap();
    let loser = svc.create_entity("Li Si", "person", "").await.unwrap();
    let a1 = insert_atom(&pool, "用户和李四结对编程").await;
    let a2 = insert_atom(&pool, "Li Si 帮用户修了 CI").await;
    let a3 = insert_atom(&pool, "用户和李四都讨厌周会").await; // 挂 winner，验证冲突忽略
    for a in [a1, a2] {
        svc.attach_atom(loser.id, a).await.unwrap();
    }
    svc.attach_atom(winner.id, a3).await.unwrap();
    svc.attach_atom(loser.id, a3).await.unwrap(); // 两边都挂 a3 → 合并时应冲突跳过

    let moved = svc.merge_entities(loser.id, winner.id).await.unwrap();
    // a1 a2 迁移成功；a3 冲突跳过 → moved = 2
    assert_eq!(moved, 2);
    let detail = svc.get_entity(winner.id).await.unwrap();
    assert_eq!(detail.entity.atom_count, 3);
    assert!(
        svc.get_entity(loser.id).await.is_err(),
        "合并后 from 应 NotFound"
    );

    // 删除赢家：必须连带清墓碑（merged_into 指向赢家的输家行），
    // 否则 FK(entities_merged_into_fkey) 拒绝——2026-08-30 AI 全旅程实测逮到。
    svc.delete_entity(winner.id).await.unwrap();
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM entities")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "删赢家应连带清掉墓碑，两行都不剩");
}

/// 议题六：place 是合法实体类型（淀山湖案——地点是用户世界的高频透镜）。
#[tokio::test]
async fn place_is_a_valid_entity_kind() {
    let (_pool, svc, _container) = setup().await;
    let e = svc
        .create_entity("淀山湖", "place", "周末骑行常去")
        .await
        .unwrap();
    assert_eq!(e.kind, "place");
    // 旧四类不受影响
    let p = svc.create_entity("张三", "person", "").await.unwrap();
    assert_eq!(p.kind, "person");
}

/// 圈子强化 P3：类型化关系建/查/删 + 同向同类型 upsert + 详情含关系。
#[tokio::test]
async fn relation_crud_upsert_and_detail() {
    let (_pool, svc, _container) = setup().await;
    let zhang = svc.create_entity("张三", "person", "").await.unwrap();
    let team = svc.create_entity("后端组", "group", "").await.unwrap();
    let shanghai = svc.create_entity("上海", "place", "").await.unwrap();

    // 建关系：张三 member_of 后端组
    let r1 = svc
        .create_relation(zhang.id, team.id, "member_of", "manual")
        .await
        .unwrap();
    assert_eq!(r1.rel_type, "member_of");
    assert_eq!(r1.weight, 1);

    // 同向同类型 upsert：weight 累加到 2
    let r2 = svc
        .create_relation(zhang.id, team.id, "member_of", "distill")
        .await
        .unwrap();
    assert_eq!(r2.id, r1.id, "同向同类型应 upsert 同一条");
    assert_eq!(r2.weight, 2);

    // 反向是不同关系
    let r3 = svc
        .create_relation(team.id, zhang.id, "member_of", "manual")
        .await
        .unwrap();
    assert_ne!(r3.id, r1.id);

    // 非法类型 / 自环
    assert!(
        svc.create_relation(zhang.id, shanghai.id, "bad_type", "manual")
            .await
            .is_err()
    );
    assert!(
        svc.create_relation(zhang.id, zhang.id, "related_to", "manual")
            .await
            .is_err()
    );

    // 详情含关系
    let detail = svc.get_entity(zhang.id).await.unwrap();
    assert!(detail.relations.iter().any(|r| r.rel_type == "member_of"));

    // list 过滤 + 删关系
    let rels = svc.list_relations(Some(zhang.id)).await.unwrap();
    assert!(rels.iter().any(|r| r.rel_type == "member_of"));
    svc.delete_relation(r1.id).await.unwrap();
    assert!(
        svc.delete_relation(r1.id).await.is_err(),
        "重复删应 NotFound"
    );
}

//! 资产运维手册（runbook）集成测试：保存→旧文入修订史→版本清单→回滚生效（回滚本身也留痕）。
use engram_core::assets::AssetService;

mod support;

#[tokio::test]
async fn runbook_save_versions_restore() {
    let pg = support::start_pgvector().await.expect("测试库");
    let url = support::connection_url(&pg).await.expect("连接串");
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");

    let id = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO assets (id, kind, name) VALUES ($1,'host','rb-test-host')")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();

    let svc = AssetService::new(pool.clone());

    // ① 两次保存：v1 → v2
    svc.save_runbook(id, "# v1\n- 磁盘 2T", "tester")
        .await
        .expect("save v1");
    svc.save_runbook(id, "# v2\n- 磁盘 2T\n- 32G 内存", "tester")
        .await
        .expect("save v2");

    // ② 当前正文 = v2
    assert_eq!(
        svc.runbook(id).await.expect("runbook"),
        "# v2\n- 磁盘 2T\n- 32G 内存"
    );

    // ③ 修订史：1 条，old = v1（新→旧）
    let versions = svc.runbook_versions(id).await.expect("versions");
    assert_eq!(versions.len(), 1, "两次保存应留 1 条旧文：{versions:?}");
    assert_eq!(versions[0].old_runbook_md, "# v1\n- 磁盘 2T");
    assert_eq!(versions[0].edited_by, "tester");

    // ④ 回滚到 v1：当前变 v1，回滚本身留痕（史变 2 条）
    let vid = versions[0].id;
    svc.restore_runbook(id, vid, "tester")
        .await
        .expect("restore");
    assert_eq!(
        svc.runbook(id).await.expect("runbook after restore"),
        "# v1\n- 磁盘 2T"
    );
    let versions = svc
        .runbook_versions(id)
        .await
        .expect("versions after restore");
    assert_eq!(versions.len(), 2, "回滚本身也留痕：{versions:?}");
    assert_eq!(versions[0].old_runbook_md, "# v2\n- 磁盘 2T\n- 32G 内存");

    // ⑤ 详情 DTO 携带 runbook；不存在资产 → NotFound
    let detail = svc.get(id).await.expect("detail");
    assert_eq!(detail.runbook_md, "# v1\n- 磁盘 2T");
    assert!(svc.runbook(uuid::Uuid::now_v7()).await.is_err());

    // ⑥ 幂等细节：不存在资产的保存报 NotFound（而非静默）
    let miss = svc
        .save_runbook(uuid::Uuid::now_v7(), "# x", "tester")
        .await;
    assert!(miss.is_err());
}

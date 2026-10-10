//! 迁移（transfer）往返闭环集成测试：
//! 造五域数据 → 导出迁移包 → 清库 → 导入 → 断言计数与内容 → 重复导入全跳过（幂等）。

use serde_json::Value;

mod support;

#[tokio::test]
async fn transfer_roundtrip_and_idempotency() {
    let pg = support::start_pgvector().await.expect("测试库");
    let url = support::connection_url(&pg).await.expect("连接串");
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");

    // ---------- 造五域数据（含跨域引用：原子挂会话、文件挂技能、位置/文档挂项目） ----------
    let sid = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO raw_sessions (id, agent, content, sensitive) VALUES ($1,'demo',$2,false)",
    )
    .bind(sid)
    .bind(serde_json::json!([{"speaker":"user","text":"我喜欢 Rust"}]))
    .execute(&pool)
    .await
    .unwrap();
    let aid = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO atoms (id, kind, content, confidence, status, source_refs, strength, source_kind) VALUES ($1,'fact','用户喜欢 Rust',0.95,'active',$2,'inference','agent_inferred')")
        .bind(aid)
        .bind(serde_json::json!([{"session_id": sid.to_string()}]))
        .execute(&pool).await.unwrap();
    let eid = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO entities (id, name, kind, summary) VALUES ($1,'engram','project','记忆平台')",
    )
    .bind(eid)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO entity_relations (id, from_id, to_id, rel_type, weight, source) VALUES ($1,$2,$3,'works_on',1,'manual')")
        .bind(uuid::Uuid::now_v7()).bind(eid).bind(eid).execute(&pool).await.unwrap();

    sqlx::query("INSERT INTO wiki_pages (id, library_id, slug, title, page_type, content, folder) VALUES ($1, (SELECT id FROM wiki_libraries WHERE slug = 'main'), 'transfer-page','迁移页','concept','# 页面','docs')")
        .bind(uuid::Uuid::now_v7()).execute(&pool).await.unwrap();

    let prj = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO projects (id, name, type, status, description, categories) VALUES ($1,'迁移项目','dev','active','往返测试',$2)")
        .bind(prj).bind(serde_json::json!(["规划"])).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO project_locations (id, project_id, ip, host, os, path, purpose, sort_order) VALUES ($1,$2,'127.0.0.1','h1','linux','/repo','dev',0)")
        .bind(uuid::Uuid::now_v7()).bind(prj).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO project_docs (id, project_id, category, title, content) VALUES ($1,$2,'规划','迁移文档','内容')")
        .bind(uuid::Uuid::now_v7()).bind(prj).execute(&pool).await.unwrap();

    // 资产台账 + 位置真引用 + 工作线关联（0058；2026-09-22 上云补齐的覆盖面）
    let asset = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO assets (id, kind, name, aliases, ip, os, note) VALUES ($1,'host','迁移主机',$2,'10.0.0.9','linux','')")
        .bind(asset)
        .bind(vec!["old-name".to_string()])
        .execute(&pool).await.unwrap();
    sqlx::query("UPDATE project_locations SET asset_id = $1 WHERE project_id = $2")
        .bind(asset)
        .bind(prj)
        .execute(&pool)
        .await
        .unwrap();
    let prj2 = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO projects (id, name, type, status, description, categories) VALUES ($1,'母项目','dev','active','往返测试',$2)")
        .bind(prj2).bind(serde_json::json!(["规划"])).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO project_links (id, from_project, to_project, kind, note) VALUES ($1,$2,$3,'part_of','子→母')")
        .bind(uuid::Uuid::now_v7()).bind(prj).bind(prj2).execute(&pool).await.unwrap();

    // ---------- 第二轮覆盖面（2026-09-22 上云核账补齐的 10 张表） ----------
    let pfile = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO project_files (id, project_id, name, mime, content, version) \
         VALUES ($1,$2,'report.html','text/html','<h1>v2</h1>',2)",
    )
    .bind(pfile)
    .bind(prj)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO project_file_versions (id, file_id, version, content) \
         VALUES ($1,$2,1,'<h1>v1</h1>')",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(pfile)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("INSERT INTO atom_entities (atom_id, entity_id) VALUES ($1,$2)")
        .bind(aid)
        .bind(eid)
        .execute(&pool)
        .await
        .unwrap();

    let todo_a = uuid::Uuid::now_v7();
    let todo_b = uuid::Uuid::now_v7();
    for (id, title) in [(todo_a, "迁移待办甲"), (todo_b, "迁移待办乙")] {
        sqlx::query("INSERT INTO todos (id, title) VALUES ($1,$2)")
            .bind(id)
            .bind(title)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO todo_links (id, from_id, to_id, kind) VALUES ($1,$2,$3,'relates_to')")
        .bind(uuid::Uuid::now_v7())
        .bind(todo_a)
        .bind(todo_b)
        .execute(&pool)
        .await
        .unwrap();

    // wiki 关联五表：文档 → 分块 → 来源 → 复核项 → 页间链接（外键序同导入序）
    let wlib: uuid::Uuid = sqlx::query_scalar("SELECT id FROM wiki_libraries WHERE slug = 'main'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let wdoc = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO wiki_documents (id, title, source_uri, mime, raw_path, sha256, status, library_id) \
         VALUES ($1,'迁移文档','https://example.test/a','text/plain','/tmp/a.txt','sha-a','ready',$2)",
    )
    .bind(wdoc)
    .bind(wlib)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO wiki_chunks (id, document_id, seq, content, library_id) VALUES ($1,$2,0,'分块正文 Rust',$3)",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(wdoc)
    .bind(wlib)
    .execute(&pool)
    .await
    .unwrap();
    let wsrc = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO wiki_sources (id, sha256, raw_path, title, status, library_id) \
         VALUES ($1,'sha-a','/tmp/a.txt','迁移来源','ready',$2)",
    )
    .bind(wsrc)
    .bind(wlib)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO wiki_review_items (id, kind, payload, action, source_id, status, library_id) \
         VALUES ($1,'create_page',$2,'create_page',$3,'open',$4)",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(serde_json::json!({"title": "建议页"}))
    .bind(wsrc)
    .bind(wlib)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO wiki_links (from_slug, to_slug, weight, library_id) \
         VALUES ('transfer-page','other-page',2.0,$1)",
    )
    .bind(wlib)
    .execute(&pool)
    .await
    .unwrap();

    // ---------- 导出 ----------
    let bundle = engram_core::transfer::export_bundle(&pool)
        .await
        .expect("导出");
    assert_eq!(bundle["format"], "engram-transfer");
    assert_eq!(bundle["counts"]["sessions"], 1);
    assert_eq!(bundle["counts"]["atoms"], 1);
    assert_eq!(bundle["counts"]["wiki_pages"], 1);
    assert_eq!(bundle["counts"]["projects"], 2);
    assert_eq!(
        bundle["counts"]["assets"], 1,
        "资产台账须随包（2026-09-22 上云补齐）"
    );
    assert_eq!(bundle["counts"]["project_links"], 1, "工作线关联须随包");
    // 第二轮补齐的 10 张表须计数可见（2026-09-22 上云核账）
    for (key, why) in [
        ("project_files", "项目文件是真实产物，不随包就是丢"),
        ("project_file_versions", "文件历史"),
        ("atom_entities", "圈子图的边，重建要重跑抽取"),
        ("todo_links", "工单关联"),
        ("wiki_documents", "wiki 源文档"),
        ("wiki_chunks", "wiki 分块"),
        ("wiki_sources", "wiki 来源台账"),
        ("wiki_review_items", "wiki 复核队列"),
        ("wiki_links", "wiki 页间链接图"),
    ] {
        assert_eq!(bundle["counts"][key], 1, "{key} 须随包（{why}）");
    }
    // 派生列不在包里
    assert!(bundle["memory"]["atoms"][0].get("embedding").is_none());
    assert!(
        bundle["wiki"]["chunks"][0].get("embedding").is_none(),
        "分块向量是派生列，不随包（导入后落 embed_failed 等 re-embed）"
    );

    // ---------- 清库（模拟一台空 B 机） ----------
    sqlx::query("TRUNCATE raw_sessions, atoms, entities, entity_relations, atom_entities, wiki_pages, wiki_documents, wiki_chunks, wiki_sources, wiki_review_items, wiki_links, projects, project_locations, project_docs, project_files, project_file_versions, assets, project_links, todos, todo_links CASCADE")
        .execute(&pool).await.unwrap();

    // ---------- 导入 ----------
    let report = engram_core::transfer::import_bundle(&pool, &bundle)
        .await
        .expect("导入");
    let imported = |path: &[&str]| -> usize {
        path.iter()
            .fold(&report, |acc, k| &acc[*k])
            .get("imported")
            .and_then(|x| x.as_u64())
            .unwrap_or(0) as usize
    };
    assert_eq!(imported(&["memory", "sessions"]), 1);
    assert_eq!(imported(&["memory", "atoms"]), 1);
    assert_eq!(imported(&["memory", "entities"]), 1);
    assert_eq!(imported(&["memory", "relations"]), 1);
    assert_eq!(imported(&["wiki", "pages"]), 1);
    assert_eq!(imported(&["projects", "projects"]), 2);
    assert_eq!(imported(&["projects", "locations"]), 1);
    assert_eq!(imported(&["projects", "docs"]), 1);
    assert_eq!(imported(&["assets"]), 1, "资产台账须能导入");
    assert_eq!(imported(&["project_links"]), 1, "工作线关联须能导入");
    // 第二轮补齐的 10 张表导入计数（2026-09-22 上云核账）
    assert_eq!(
        imported(&["memory", "atom_entities"]),
        1,
        "圈子图的边须能导入"
    );
    assert_eq!(imported(&["projects", "files"]), 1, "项目文件须能导入");
    assert_eq!(
        imported(&["projects", "file_versions"]),
        1,
        "项目文件历史须能导入"
    );
    assert_eq!(imported(&["todo_links"]), 1, "工单关联须能导入");
    assert_eq!(imported(&["wiki", "documents"]), 1, "wiki 文档须能导入");
    assert_eq!(imported(&["wiki", "chunks"]), 1, "wiki 分块须能导入");
    assert_eq!(imported(&["wiki", "sources"]), 1, "wiki 来源须能导入");
    assert_eq!(
        imported(&["wiki", "review_items"]),
        1,
        "wiki 复核项须能导入"
    );
    assert_eq!(imported(&["wiki", "links"]), 1, "wiki 页间链接须能导入");

    // ---------- 内容抽验（跨域引用与派生列重建） ----------
    let (content, refs): (String, Value) =
        sqlx::query_as("SELECT content, source_refs FROM atoms WHERE id = $1")
            .bind(aid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(content, "用户喜欢 Rust");
    // P019-M4：断言强度两列随包保真（旧实现漏列静默回落 fact）
    let (strength, source_kind): (String, String) =
        sqlx::query_as("SELECT strength, source_kind FROM atoms WHERE id = $1")
            .bind(aid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (strength.as_str(), source_kind.as_str()),
        ("inference", "agent_inferred"),
        "断言强度应随迁移保真，不得回落 DDL 默认 fact"
    );
    assert_eq!(
        refs[0]["session_id"],
        sid.to_string(),
        "原子↔会话引用应随 id 保留"
    );
    let wiki_ok: bool = sqlx::query_scalar(
        "SELECT tsv @@ to_tsquery('simple','页面') FROM wiki_pages WHERE slug='transfer-page'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(wiki_ok);

    // 位置 → 资产的真引用必须随包保留（0058；缺 asset_id 列会静默丢引用）
    let loc_asset: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT asset_id FROM project_locations WHERE project_id = $1")
            .bind(prj)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(loc_asset, Some(asset), "位置对资产的真引用应在导入后仍在");
    let (asset_name, asset_kind): (String, String) =
        sqlx::query_as("SELECT name, kind FROM assets WHERE id = $1")
            .bind(asset)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (asset_name.as_str(), asset_kind.as_str()),
        ("迁移主机", "host")
    );
    let link_n: i64 = sqlx::query_scalar("SELECT count(*) FROM project_links")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(link_n, 1, "工作线关联应随迁移进");

    // 第二轮 10 张表的落地抽验（2026-09-22 上云核账）
    let (pfile_mime, pfile_ver): (String, i32) =
        sqlx::query_as("SELECT mime, version FROM project_files WHERE id = $1")
            .bind(pfile)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (pfile_mime.as_str(), pfile_ver),
        ("text/html", 2),
        "项目文件内容与版本号须保留"
    );
    let pver_n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM project_file_versions WHERE file_id = $1")
            .bind(pfile)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(pver_n, 1, "文件历史应随迁移进");
    let ae_n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM atom_entities WHERE atom_id = $1 AND entity_id = $2",
    )
    .bind(aid)
    .bind(eid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ae_n, 1, "圈子图的边应随迁移进（否则实体失联）");
    let tlink_n: i64 = sqlx::query_scalar("SELECT count(*) FROM todo_links")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(tlink_n, 1, "工单/待办关联应随迁移进");
    let (chunk_tsv_ok, chunk_flag, chunk_vec_null): (bool, bool, bool) = sqlx::query_as(
        "SELECT tsv @@ to_tsquery('simple','rust'), embed_failed, embedding IS NULL \
         FROM wiki_chunks WHERE document_id = $1",
    )
    .bind(wdoc)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(chunk_tsv_ok, "分块 tsv 应按摄取同口径重建（FTS 可检索）");
    assert!(
        chunk_flag && chunk_vec_null,
        "分块向量缺省须落 embed_failed=true（等 re-embed 补）"
    );
    let src_status: String = sqlx::query_scalar("SELECT status FROM wiki_sources WHERE id = $1")
        .bind(wsrc)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(src_status, "ready", "来源台账状态须保留");
    let review_kind: String =
        sqlx::query_scalar("SELECT kind FROM wiki_review_items WHERE source_id = $1")
            .bind(wsrc)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(review_kind, "create_page", "复核项须保留且外键指回来源");
    let link_weight: f32 =
        sqlx::query_scalar("SELECT weight FROM wiki_links WHERE from_slug = 'transfer-page'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!((link_weight - 2.0).abs() < 0.01, "页间链接权重须保留");

    // ---------- 幂等：重复导入全 skipped ----------
    let report2 = engram_core::transfer::import_bundle(&pool, &bundle)
        .await
        .expect("重复导入");
    let all_skipped = ["memory", "wiki", "projects"].iter().all(|dom| {
        let d = &report2[dom];
        serde_json::to_string(d).unwrap().contains("\"skipped\"")
    });
    assert!(all_skipped);
    assert_eq!(report2["memory"]["sessions"]["skipped"], 1);
    assert_eq!(report2["assets"]["skipped"], 1, "资产重复导入应跳过");
    assert_eq!(report2["project_links"]["skipped"], 1, "关联重复导入应跳过");
    assert_eq!(
        report2["memory"]["atom_entities"]["skipped"], 1,
        "圈子图的边重复导入应跳过"
    );
    assert_eq!(report2["projects"]["files"]["skipped"], 1);
    assert_eq!(report2["projects"]["file_versions"]["skipped"], 1);
    assert_eq!(report2["todo_links"]["skipped"], 1);
    assert_eq!(report2["wiki"]["chunks"]["skipped"], 1);
    assert_eq!(report2["wiki"]["links"]["skipped"], 1);
}

#[tokio::test]
async fn import_rejects_foreign_format() {
    let pg = support::start_pgvector().await.expect("测试库");
    let url = support::connection_url(&pg).await.expect("连接串");
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    let err = engram_core::transfer::import_bundle(&pool, &serde_json::json!({"foo": 1}))
        .await
        .expect_err("非迁移包应拒绝");
    assert!(err.to_string().contains("engram-transfer"));
}

#[tokio::test]
async fn location_asset_ref_self_heals_by_name() {
    let pg = support::start_pgvector().await.expect("测试库");
    let url = support::connection_url(&pg).await.expect("连接串");
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");

    // 目标库先有一条同名资产（id 与「源包」里的不同）+ 备一个项目
    let real_asset = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO assets (id, kind, name, aliases) VALUES ($1,'host','MacBook Air M1',$2)",
    )
    .bind(real_asset)
    .bind(vec!["trtyr-mac".to_string()])
    .execute(&pool)
    .await
    .unwrap();
    let prj = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO projects (id, name, type, status) VALUES ($1,'自愈项目','dev','active')",
    )
    .bind(prj)
    .execute(&pool)
    .await
    .unwrap();

    // 包里带一个目标库不存在的 asset_id（模拟另一次导入产生的不同 UUID），host 用别名写法
    let bundle = serde_json::json!({
        "format": "engram-transfer",
        "projects": {
            "projects": [{"id": prj, "name": "自愈项目", "type": "dev", "status": "active", "categories": []}],
            "locations": [{
                "id": uuid::Uuid::now_v7(), "project_id": prj, "ip": "", "host": "trtyr-mac",
                "os": "", "path": "/x", "purpose": null, "sort_order": 0,
                "asset_id": uuid::Uuid::now_v7()
            }],
            "docs": []
        }
    });
    engram_core::transfer::import_bundle(&pool, &bundle)
        .await
        .expect("导入");

    let got: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT asset_id FROM project_locations WHERE project_id = $1")
            .bind(prj)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        got,
        Some(real_asset),
        "asset_id 落空时应按别名回落到目标库同名资产"
    );
}

/// P019-M4：promotions 按包内库映射挂原库——非 main 库的晋升登记导入后归属不变
/// （旧实现硬编码 main，全部改挂主库）；缺 created_at 的会话行不中断导入（now 兑底）。
#[tokio::test]
async fn import_promotions_follow_library_mapping_and_tolerate_missing_ts() {
    let pg = support::start_pgvector().await.expect("测试库");
    let url = support::connection_url(&pg).await.expect("连接串");
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");

    let main_lib: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM wiki_libraries WHERE slug = 'main'")
            .fetch_one(&pool)
            .await
            .unwrap();
    // 非 main 库
    let alt_lib = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO wiki_libraries (id, slug, name) VALUES ($1, 'alt', '备用库')")
        .bind(alt_lib)
        .execute(&pool)
        .await
        .unwrap();
    let prj = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO projects (id, name, type, status) VALUES ($1,'晋升项目','dev','active')",
    )
    .bind(prj)
    .execute(&pool)
    .await
    .unwrap();
    let doc = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO project_docs (id, project_id, category, title, content) VALUES ($1,$2,'规划','晋升源文档','内容')")
        .bind(doc)
        .bind(prj)
        .execute(&pool)
        .await
        .unwrap();
    // 晋升登记挂在 alt 库
    sqlx::query("INSERT INTO wiki_promotions (id, library_id, page_slug, project_id, doc_id) VALUES ($1,$2,'alt-page',$3,$4)")
        .bind(uuid::Uuid::now_v7())
        .bind(alt_lib)
        .bind(prj)
        .bind(doc)
        .execute(&pool)
        .await
        .unwrap();
    // 缺 created_at 的会话行（旧行为：NOT NULL 23502 中断整个导入域）
    sqlx::query("INSERT INTO raw_sessions (id, agent, content) VALUES ($1,'demo',$2)")
        .bind(uuid::Uuid::now_v7())
        .bind(serde_json::json!([{"speaker":"user","text":"缺时间戳"}]))
        .execute(&pool)
        .await
        .unwrap();

    let bundle = engram_core::transfer::export_bundle(&pool)
        .await
        .expect("导出");
    assert_eq!(
        bundle["wiki_promotions"].as_array().map(|a| a.len()),
        Some(1),
        "bundle 应含 1 条 promotions: {}",
        bundle["wiki_promotions"]
    );
    // 清库模拟目标机
    sqlx::query(
        "TRUNCATE wiki_promotions, wiki_libraries, project_docs, projects, raw_sessions CASCADE",
    )
    .execute(&pool)
    .await
    .unwrap();
    let report = engram_core::transfer::import_bundle(&pool, &bundle)
        .await
        .expect("导入应成功（缺 created_at 不再中断）");

    let imported = |path: &[&str]| -> usize {
        path.iter()
            .fold(&report, |acc, k| &acc[*k])
            .get("imported")
            .and_then(|x| x.as_u64())
            .unwrap_or(0) as usize
    };
    assert_eq!(
        imported(&["memory", "sessions"]),
        1,
        "缺 created_at 会话应导入"
    );
    assert_eq!(imported(&["wiki_promotions"]), 1, "晋升登记应导入");

    // 库映射：transfer 保留原 id——alt 库导入后 id 不变，晋升登记应挂回 alt（非 main）
    let new_alt: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM wiki_libraries WHERE slug = 'alt'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(new_alt, alt_lib, "transfer 保留原库 id");
    let promo_lib: uuid::Uuid =
        sqlx::query_scalar("SELECT library_id FROM wiki_promotions WHERE page_slug = 'alt-page'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(promo_lib, new_alt, "晋升登记应随包内库映射挂回 alt 库");
    assert_ne!(promo_lib, main_lib, "不得硬编码改挂 main");
}

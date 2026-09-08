//! 迁移（transfer）往返闭环集成测试：
//! 造五域数据 → 导出迁移包 → 清库 → 导入 → 断言计数与内容 → 重复导入全跳过（幂等）。

use engram_storage::repo::transfer as trepo;

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
    sqlx::query("INSERT INTO atoms (id, kind, content, confidence, status, source_refs, tsv) VALUES ($1,'fact','用户喜欢 Rust',0.95,'active',$2,to_tsvector('simple','用户喜欢 Rust'))")
        .bind(aid)
        .bind(serde_json::json!([{"session_id": sid.to_string()}]))
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO scenarios (id, topic, summary, body, version) VALUES ($1,'Rust 偏好','语言偏好','话题正文',1)")
        .bind(uuid::Uuid::now_v7()).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO persona_aspects (id, aspect, content, version) VALUES ($1,'preferences','偏好 Rust',1)")
        .bind(uuid::Uuid::now_v7()).execute(&pool).await.unwrap();
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

    let slug = "transfer-skill";
    sqlx::query("INSERT INTO skills (id, slug, name, description, content, tags, enabled, source) VALUES ($1,$2,'打包技能','迁移','正文',$3,true,'manual')")
        .bind(uuid::Uuid::now_v7()).bind(slug)
        .bind(vec!["ops".to_string()]).execute(&pool).await.unwrap();
    let skid: uuid::Uuid = sqlx::query_scalar("SELECT id FROM skills WHERE slug=$1")
        .bind(slug)
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO skill_files (id, skill_id, path, content) VALUES ($1,$2,'scripts/run.sh','echo hi')")
        .bind(uuid::Uuid::now_v7()).bind(skid).execute(&pool).await.unwrap();

    sqlx::query("INSERT INTO wiki_pages (id, library_id, slug, title, page_type, content, folder) VALUES ($1, (SELECT id FROM wiki_libraries WHERE slug = 'main'), 'transfer-page','迁移页','concept','# 页面','docs')")
        .bind(uuid::Uuid::now_v7()).execute(&pool).await.unwrap();

    let prj = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO projects (id, name, type, status, description, categories) VALUES ($1,'迁移项目','dev','active','往返测试',$2)")
        .bind(prj).bind(serde_json::json!(["规划"])).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO project_locations (id, project_id, ip, host, os, path, purpose, sort_order) VALUES ($1,$2,'127.0.0.1','h1','linux','/repo','dev',0)")
        .bind(uuid::Uuid::now_v7()).bind(prj).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO project_docs (id, project_id, category, title, content) VALUES ($1,$2,'规划','迁移文档','内容')")
        .bind(uuid::Uuid::now_v7()).bind(prj).execute(&pool).await.unwrap();

    // ---------- 导出 ----------
    let bundle = engram_core::transfer::export_bundle(&pool)
        .await
        .expect("导出");
    assert_eq!(bundle["format"], "engram-transfer");
    assert_eq!(bundle["counts"]["sessions"], 1);
    assert_eq!(bundle["counts"]["atoms"], 1);
    assert_eq!(bundle["counts"]["skills"], 1);
    assert_eq!(bundle["counts"]["wiki_pages"], 1);
    assert_eq!(bundle["counts"]["projects"], 1);
    // 派生列不在包里
    assert!(bundle["memory"]["atoms"][0].get("embedding").is_none());
    assert!(
        bundle["skills"][0]
            .get("files")
            .and_then(|f| f.as_array())
            .map(|f| f.len())
            == Some(1)
    );

    // ---------- 清库（模拟一台空 B 机） ----------
    sqlx::query("TRUNCATE raw_sessions, atoms, scenarios, persona_aspects, entities, entity_relations, skills, skill_files, wiki_pages, projects, project_locations, project_docs CASCADE")
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
    assert_eq!(imported(&["memory", "scenarios"]), 1);
    assert_eq!(imported(&["memory", "persona"]), 1);
    assert_eq!(imported(&["memory", "entities"]), 1);
    assert_eq!(imported(&["memory", "relations"]), 1);
    assert_eq!(imported(&["skills", "skills"]), 1);
    assert_eq!(imported(&["wiki", "pages"]), 1);
    assert_eq!(imported(&["projects", "projects"]), 1);
    assert_eq!(imported(&["projects", "locations"]), 1);
    assert_eq!(imported(&["projects", "docs"]), 1);

    // ---------- 内容抽验（跨域引用与派生列重建） ----------
    let (content, refs): (String, Value) =
        sqlx::query_as("SELECT content, source_refs FROM atoms WHERE id = $1")
            .bind(aid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(content, "用户喜欢 Rust");
    assert_eq!(
        refs[0]["session_id"],
        sid.to_string(),
        "原子↔会话引用应随 id 保留"
    );
    let tsv_ok: bool =
        sqlx::query_scalar("SELECT tsv @@ to_tsquery('simple','rust') FROM atoms WHERE id = $1")
            .bind(aid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(tsv_ok, "导入应重建 tsv（FTS 可检索）");
    let files: i64 = sqlx::query_scalar("SELECT count(*) FROM skill_files WHERE skill_id = $1")
        .bind(skid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(files, 1, "技能附属文件应随迁移进");
    let wiki_ok: bool = sqlx::query_scalar(
        "SELECT tsv @@ to_tsquery('simple','页面') FROM wiki_pages WHERE slug='transfer-page'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(wiki_ok);

    // ---------- 幂等：重复导入全 skipped ----------
    let report2 = engram_core::transfer::import_bundle(&pool, &bundle)
        .await
        .expect("重复导入");
    let all_skipped = ["memory", "skills", "wiki", "projects"].iter().all(|dom| {
        let d = &report2[dom];
        serde_json::to_string(d).unwrap().contains("\"skipped\"")
    });
    assert!(all_skipped);
    assert_eq!(report2["skills"]["skills"]["skipped"], 1);
    assert_eq!(report2["memory"]["sessions"]["skipped"], 1);
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
async fn export_roundtrip_skills_files_count() {
    let pg = support::start_pgvector().await.expect("测试库");
    let url = support::connection_url(&pg).await.expect("连接串");
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    trepo::import_skill(
        &pool,
        &serde_json::json!({"id": uuid::Uuid::now_v7(), "slug":"f1","name":"F","description":"","content":"c","tags":[],"enabled":true}),
        &[serde_json::json!({"path":"a/b.txt","content":"x"})],
    ).await.expect("import_skill");
    let skills = trepo::export_skills_with_files(&pool)
        .await
        .expect("export");
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].1.len(), 1);
    assert_eq!(skills[0].1[0]["path"], "a/b.txt");
}

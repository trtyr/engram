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

    // ---------- 导出 ----------
    let bundle = engram_core::transfer::export_bundle(&pool)
        .await
        .expect("导出");
    assert_eq!(bundle["format"], "engram-transfer");
    assert_eq!(bundle["counts"]["sessions"], 1);
    assert_eq!(bundle["counts"]["atoms"], 1);
    assert_eq!(bundle["counts"]["skills"], 1);
    assert_eq!(bundle["counts"]["wiki_pages"], 1);
    assert_eq!(bundle["counts"]["projects"], 2);
    assert_eq!(
        bundle["counts"]["assets"], 1,
        "资产台账须随包（2026-09-22 上云补齐）"
    );
    assert_eq!(bundle["counts"]["project_links"], 1, "工作线关联须随包");
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
    sqlx::query("TRUNCATE raw_sessions, atoms, scenarios, persona_aspects, entities, entity_relations, skills, skill_files, wiki_pages, projects, project_locations, project_docs, assets, project_links CASCADE")
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
    assert_eq!(imported(&["projects", "projects"]), 2);
    assert_eq!(imported(&["projects", "locations"]), 1);
    assert_eq!(imported(&["projects", "docs"]), 1);
    assert_eq!(imported(&["assets"]), 1, "资产台账须能导入");
    assert_eq!(imported(&["project_links"]), 1, "工作线关联须能导入");

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
    assert_eq!(report2["assets"]["skipped"], 1, "资产重复导入应跳过");
    assert_eq!(report2["project_links"]["skipped"], 1, "关联重复导入应跳过");
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

/// 二态（0038）：script 型技能迁移 = 只导元数据（指针 + 来源），
/// A 机导出 → B 机导入，指针照收（导入端待本地就位）。
#[tokio::test]
async fn transfer_script_skill_metadata_only() {
    let pg_a = support::start_pgvector().await.expect("A 机测试库");
    let url_a = support::connection_url(&pg_a).await.expect("A 连接串");
    let pool_a = support::connect_with_retry(&url_a).await.expect("A 连接");
    engram_storage::run_migrations(&pool_a)
        .await
        .expect("A 迁移");
    let pg_b = support::start_pgvector().await.expect("B 机测试库");
    let url_b = support::connection_url(&pg_b).await.expect("B 连接串");
    let pool_b = support::connect_with_retry(&url_b).await.expect("B 连接");
    engram_storage::run_migrations(&pool_b)
        .await
        .expect("B 迁移");

    // A 机：script 型技能（指针 + 来源，无附属文件）
    trepo::import_skill(
        &pool_a,
        &serde_json::json!({
            "id": uuid::Uuid::now_v7(), "slug":"remote-tool","name":"远程工具",
            "description":"","content":"","tags":[],"enabled":true,
            "kind":"script","origin":"both",
            "local_path":"/opt/skills/remote-tool","repo_url":"https://github.com/x/remote-tool"
        }),
        &[],
    )
    .await
    .expect("A 机导入 script 技能");

    // A 机导出：script 型无 files 行、元数据随行
    let skills = trepo::export_skills_with_files(&pool_a)
        .await
        .expect("A 机导出");
    let entry = skills
        .iter()
        .find(|(s, _)| s["slug"] == "remote-tool")
        .expect("导出应含 remote-tool");
    assert_eq!(entry.1.len(), 0, "script 型不应带附属文件");
    assert_eq!(entry.0["kind"], "script");
    assert_eq!(entry.0["origin"], "both");
    assert_eq!(entry.0["local_path"], "/opt/skills/remote-tool");
    assert_eq!(entry.0["repo_url"], "https://github.com/x/remote-tool");

    // B 机导入：指针照收（先转成 export_bundle 同构形态：skill 对象 + files 字段）
    let payload: Vec<Value> = skills
        .into_iter()
        .map(|(mut s, files)| {
            s["files"] = serde_json::json!(files);
            s
        })
        .collect();
    let report = engram_core::transfer::import_skills_bundle(&pool_b, &serde_json::json!(payload))
        .await
        .expect("B 机导入");
    assert_eq!(report["imported"], 1);
    assert_eq!(report["files_imported"], 0, "script 型不导文件");
    let (kind, origin, lp, ru): (String, String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT kind, origin, local_path, repo_url FROM skills WHERE slug='remote-tool'",
    )
    .fetch_one(&pool_b)
    .await
    .unwrap();
    assert_eq!((kind.as_str(), origin.as_str()), ("script", "both"));
    assert_eq!(lp.as_deref(), Some("/opt/skills/remote-tool"));
    assert_eq!(ru.as_deref(), Some("https://github.com/x/remote-tool"));
}

/// 位置 → 资产引用的**自愈**：包里 asset_id 在目标库不存在时，按「台账名 / 别名」重新解析
/// （与 0058 存量清洗同规则）——跨实例迁移后引用不会静默落空。
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

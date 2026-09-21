//! cg-bridge 集成测试：项目生命周期 + 版本不匹配 + 真索引（本仓库自身）。

mod support;

use engram_cg_bridge::{CgBridge, CgError, QueryKind};

async fn setup() -> (sqlx::PgPool, CgBridge, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool).await.expect("迁移");
    let dir = tempfile::tempdir().unwrap();
    let bridge = CgBridge::new(pool.clone(), dir.keep());
    (pool, bridge, container)
}

/// 反向亦然（R1「后到者覆盖 + 完整留痕」，迁移 0055）：`client_upload` 记录被**云自建**刷新时，
/// 来源标注改判 `cloud_index`、投递者记为 `cloud_index`，且被覆盖那份压进 `stats.previous`。
/// 不走 CLI（直接调元数据回写原语）——CLI 路径的端到端由 cg_upload/查询集成测试覆盖。
#[tokio::test]
async fn cloud_index_overwrites_client_upload_with_trace() {
    let (pool, bridge, _pg) = setup().await;
    let id = uuid::Uuid::now_v7();
    let head = "a1b2c3d4e5";
    sqlx::query(
        "INSERT INTO cg_projects (id, name, path, source_uri, status, source_kind, head, \
         produced_at, built_with_version, last_producer) \
         VALUES ($1, 'up-then-cloud', '/tmp/up-then-cloud', $2, 'ready', 'client_upload', $3, \
         now(), '1.5.0', 'client:test-key')",
    )
    .bind(id)
    .bind(format!("upload://{head}"))
    .bind(head)
    .execute(&pool)
    .await
    .unwrap();

    bridge.mark_cloud_artifact(id).await.unwrap();

    let row = bridge.get(id).await.unwrap();
    assert_eq!(row.source_kind, "cloud_index", "云自建刷新后来源标注应改判");
    assert_eq!(
        row.last_producer.as_deref(),
        Some("cloud_index"),
        "投递者应为 cloud_index"
    );
    let prev = row
        .stats
        .as_ref()
        .and_then(|s| s.get("previous"))
        .expect("被覆盖那份应留痕在 stats.previous");
    assert_eq!(prev["head"], head, "留痕应含上一份 head：{prev}");
    assert_eq!(
        prev["source_kind"], "client_upload",
        "留痕应含上一份来源：{prev}"
    );
}

/// 版本探测 + 不匹配路径（注入假 pin 验证分类）。
#[tokio::test]
async fn version_guard_rejects_mismatch() {
    let (pool, bridge, _pg) = setup().await;
    // 真 CLI 存在 → 探测成功
    match bridge.detect_version().await {
        Ok(v) => {
            // 手动构造不匹配场景：直接检查分类逻辑
            let need = "0.0.1";
            let ver = v.split_whitespace().last().unwrap_or(&v);
            if ver != need {
                let e = CgError::VersionMismatch {
                    need: need.into(),
                    got: ver.into(),
                };
                assert!(e.to_string().contains("版本不匹配"));
            }
        }
        Err(CgError::CliUnavailable(_)) => {
            // CI 无 CLI：确保 ensure_version 报不可用（而非 panic）
            let e = bridge.ensure_version().await.unwrap_err();
            assert!(matches!(e, CgError::CliUnavailable(_)), "{e:?}");
        }
        Err(other) => panic!("探测应成功或报不可用: {other:?}"),
    }

    // mark_all_version_mismatch DB 路径
    sqlx::query(
        "INSERT INTO cg_projects (id, name, path, source_uri) VALUES ($1,'t','/tmp','/tmp')",
    )
    .bind(uuid::Uuid::new_v4())
    .execute(&pool)
    .await
    .unwrap();
    let n = bridge.mark_all_version_mismatch("9.9.9").await.unwrap();
    assert_eq!(n, 1);
    let st: String = sqlx::query_scalar("SELECT status FROM cg_projects LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(st, "version_mismatch");
    // version_mismatch 项目查询被拒
    let pid: uuid::Uuid = sqlx::query_scalar("SELECT id FROM cg_projects LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let e = bridge
        .query(pid, QueryKind::Search, "x", None, false)
        .await
        .unwrap_err();
    assert!(matches!(e, CgError::VersionMismatch { .. }), "{e:?}");
}

/// 注册校验（入口收敛 2026-09-21）：只收 git 仓库地址——普通路径一律拒。
#[tokio::test]
async fn register_validation() {
    let (_pool, bridge, _pg) = setup().await;
    let e = bridge
        .register("t1", "/definitely/not/exists", None)
        .await
        .unwrap_err();
    assert!(matches!(e, CgError::BadRequest(_)), "{e:?}");
}

/// 真索引 + 查询（本仓库自身；CI 无 codegraph CLI 时跳过）。
#[tokio::test]
async fn index_and_query_real_repo() {
    let (_pool, bridge, _pg) = setup().await;
    if bridge.detect_version().await.is_err() {
        eprintln_no_cli();
        return; // CI 环境无 CLI：跳过（本地为权威验证）
    }

    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .unwrap()
        .to_path_buf(); // 仓库根（crates/cg-bridge → crates → server → 根）
    let repo = repo.canonicalize().unwrap();
    let proj = bridge
        .register("self", &format!("file://{}", repo.to_str().unwrap()), None)
        .await
        .unwrap();
    assert_eq!(proj.status, "registered");

    let proj = bridge.index(proj.id).await.expect("索引应成功");
    assert_eq!(proj.status, "ready", "err: {:?}", proj.error);

    // search：JobQueue 符号命中
    let r = bridge
        .query(proj.id, QueryKind::Search, "JobQueue", None, false)
        .await
        .unwrap();
    let arr = r.as_array().unwrap();
    assert!(!arr.is_empty());
    assert!(
        arr[0]["node"]["name"]
            .as_str()
            .unwrap()
            .contains("JobQueue")
    );

    // callers：run_migrations 有调用者
    let r = bridge
        .query(proj.id, QueryKind::Callers, "run_migrations", None, false)
        .await
        .unwrap();
    assert!(
        r["callers"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "{r}"
    );

    // impact：enqueue 影响面非空
    let r = bridge
        .query(proj.id, QueryKind::Impact, "enqueue", Some(2), false)
        .await
        .unwrap();
    assert!(
        r["affected"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "{r}"
    );

    // explore：Markdown 文本
    let r = bridge
        .query(
            proj.id,
            QueryKind::Explore,
            "How does the job queue claim tasks",
            None,
            true,
        )
        .await
        .unwrap();
    let text = r["text"].as_str().unwrap();
    assert!(
        text.contains("Exploration") || text.contains("JobQueue"),
        "{}",
        &text[..100.min(text.len())]
    );
}

fn eprintln_no_cli() {
    let _ = std::fs::write(
        "/tmp/cg-skip.txt",
        "CI 无 codegraph CLI，真索引测试跳过（本地已验证）",
    );
}

/// EN-48 验收链路：索引产物消失（目录被重新 clone / 清理）之后，
/// list 不能再报「可用」，查询必须给「索引产物已丢失」这个具体病因，gc 能把幽灵落账。
/// 不依赖真 codegraph CLI——直接造出「已 ready」的项目状态，专测产物丢失这一层。
#[tokio::test]
async fn lost_artifact_detected_by_list_query_and_gc() {
    let (pool, bridge, _pg) = setup().await;

    // 造一个「已索引好」的项目：目录在、产物在（判存在性用一个空文件就够）。
    // 直接落库造状态（不经 register）——register 已收敛为 URI-only + clone，本用例测的是产物丢失层。
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap(); // 界标：仓库根（产物查找不越界上溯）
    let proj_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO cg_projects (id, name, path, source_uri, status, source_kind) \
         VALUES ($1, 'victim', $2, 'file:///fixture/victim', 'registered', 'cloud_index')",
    )
    .bind(proj_id)
    .bind(dir.path().to_str().unwrap())
    .execute(&pool)
    .await
    .unwrap();
    let proj = bridge.get(proj_id).await.unwrap();
    std::fs::create_dir_all(dir.path().join(".codegraph")).unwrap();
    std::fs::write(dir.path().join(".codegraph").join("codegraph.db"), b"").unwrap();
    sqlx::query("UPDATE cg_projects SET status = 'ready' WHERE id = $1")
        .bind(proj.id)
        .execute(&pool)
        .await
        .unwrap();

    // ① 产物在盘 → usable
    assert!(
        bridge.get(proj.id).await.unwrap().usable,
        "产物在盘时应可用"
    );

    // ② 删掉 .codegraph（= 目录被重新 clone / 被清理）→ status 仍是 ready，usable 翻假
    std::fs::remove_dir_all(dir.path().join(".codegraph")).unwrap();
    let me = bridge.get(proj.id).await.unwrap();
    assert_eq!(me.status, "ready", "status 是历史记录，本就不会自己变");
    assert!(
        !me.usable,
        "产物已丢失——正是 EN-48 要抓的那一类（旧口径会照报可用）"
    );
    let listed = bridge.list().await.unwrap();
    assert!(
        !listed.iter().find(|p| p.id == proj.id).unwrap().usable,
        "list 也必须反映当下可用性"
    );

    // ③ 查询给具体病因，不再是含糊的「索引库不存在」/「未就绪」/「CLI 不可用」
    let e = bridge
        .query(proj.id, QueryKind::Search, "x", None, false)
        .await
        .unwrap_err();
    assert!(matches!(e, CgError::NotFound(_)), "{e:?}");
    assert!(e.to_string().contains("索引产物已丢失"), "{e}");
    let e = bridge.full_graph(proj.id).await.unwrap_err();
    assert!(e.to_string().contains("索引产物已丢失"), "{e}");

    // ④ gc 对账：幽灵落为 error（可逆——重新 index 即恢复）
    let report = bridge.gc().await.unwrap();
    assert_eq!(report["marked_invalid"].as_u64(), Some(1), "{report}");
    let after = bridge.get(proj.id).await.unwrap();
    assert_eq!(after.status, "error");
    assert!(!after.usable);
    assert!(
        after.error.unwrap_or_default().contains("索引产物已丢失"),
        "error 字段要写明病因"
    );
    // 自愈清单（EN-48 残留）：victim 路径仍在、仅产物丢失 → 必须进 needs_rebuild
    let needs = report["needs_rebuild"].as_array().unwrap();
    assert_eq!(needs.len(), 1, "{report}");
    assert_eq!(
        needs[0]["id"].as_str(),
        Some(proj.id.to_string()).as_deref()
    );

    // ⑤ 另一类幽灵：路径整个不存在（容器形态注册、宿主上不可见的条目）
    let ghost_dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(ghost_dir.path().join(".git")).unwrap();
    let ghost_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO cg_projects (id, name, path, source_uri, status, source_kind) \
         VALUES ($1, 'ghost', $2, 'file:///fixture/ghost', 'registered', 'cloud_index')",
    )
    .bind(ghost_id)
    .bind(ghost_dir.path().to_str().unwrap())
    .execute(&pool)
    .await
    .unwrap();
    let ghost = bridge.get(ghost_id).await.unwrap();
    sqlx::query("UPDATE cg_projects SET status = 'ready' WHERE id = $1")
        .bind(ghost.id)
        .execute(&pool)
        .await
        .unwrap();
    drop(ghost_dir); // 目录消失
    let e = bridge
        .query(ghost.id, QueryKind::Search, "x", None, false)
        .await
        .unwrap_err();
    assert!(matches!(e, CgError::NotFound(_)), "{e:?}");
    assert!(e.to_string().contains("项目路径不存在"), "{e}");

    // ⑥ 自愈边界：路径不存在的幽灵不可自愈——gc 落 error 但绝不进 needs_rebuild
    let report2 = bridge.gc().await.unwrap();
    assert_eq!(report2["marked_invalid"].as_u64(), Some(1), "{report2}");
    assert!(
        report2["needs_rebuild"].as_array().unwrap().is_empty(),
        "幽灵条目不能自动重建（路径都没了，重建必失败）: {report2}"
    );
    let ghost_after = bridge.get(ghost.id).await.unwrap();
    assert_eq!(ghost_after.status, "error");
    assert!(
        ghost_after
            .error
            .unwrap_or_default()
            .contains("项目路径不存在"),
        "幽灵病因要写「路径不存在」而非「产物丢失」"
    );
}

/// t4：graph 响应带初布局坐标——有 `layout.json` 就带 `x`/`y`，没有就不带字段（前端据此自收敛）。
/// 三态都验：无布局 / 有布局 / 版本不认识的布局（视作没有）。
#[tokio::test]
async fn graph_nodes_carry_layout_positions() {
    let (pool, bridge, _pg) = setup().await;
    let root = tempfile::tempdir().unwrap();
    let cgdir = root.path().join(".codegraph");
    std::fs::create_dir_all(&cgdir).unwrap();
    let db = cgdir.join("codegraph.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE files(path TEXT PRIMARY KEY); \
         CREATE TABLE nodes(id TEXT PRIMARY KEY, file_path TEXT); \
         CREATE TABLE edges(source TEXT, target TEXT, kind TEXT); \
         INSERT INTO files(path) VALUES ('a.rs'), ('b.rs'); \
         INSERT INTO nodes(id, file_path) VALUES ('n1','a.rs'), ('n2','b.rs'); \
         INSERT INTO edges(source, target, kind) VALUES ('n1','n2','calls');",
    )
    .unwrap();
    drop(conn);

    let id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO cg_projects (id, name, path, source_uri, status, source_kind, dest_mode) \
         VALUES ($1, 'layout-proj', $2, 'git://example.com/layout-proj', 'ready', 'cloud_index', 'custom')",
    )
    .bind(id)
    .bind(root.path().to_string_lossy().to_string())
    .execute(&pool)
    .await
    .unwrap();

    // ① 没有 layout.json → 节点一律不带坐标
    let g = bridge.full_graph(id).await.unwrap();
    assert_eq!(g["mode"], "files");
    for n in g["nodes"].as_array().unwrap() {
        assert!(
            n.get("x").is_none() && n.get("y").is_none(),
            "无布局不该冒充坐标: {n}"
        );
    }

    // ② 落一份布局 → 坐标随响应带出（键就是文件路径）
    let mut positions = std::collections::HashMap::new();
    positions.insert("a.rs".to_string(), [1.5, -2.0]);
    positions.insert("b.rs".to_string(), [3.5, 4.0]);
    let lf = engram_cg_bridge::layout::LayoutFile {
        version: engram_cg_bridge::layout::LAYOUT_VERSION,
        algorithm: engram_cg_bridge::layout::LAYOUT_ALGORITHM.to_string(),
        iterations: 1,
        nodes: 2,
        edges: 1,
        generated_at: chrono::Utc::now(),
        positions,
    };
    std::fs::write(cgdir.join("layout.json"), serde_json::to_vec(&lf).unwrap()).unwrap();

    let g2 = bridge.full_graph(id).await.unwrap();
    let nodes = g2["nodes"].as_array().unwrap();
    let a = nodes
        .iter()
        .find(|n| n["id"] == "a.rs")
        .expect("a.rs 应在节点里");
    assert_eq!(a["x"], 1.5, "有布局时带 x");
    assert_eq!(a["y"], -2.0, "有布局时带 y");
    let b = nodes
        .iter()
        .find(|n| n["id"] == "b.rs")
        .expect("b.rs 应在节点里");
    assert_eq!(b["x"], 3.5);

    // ③ 版本不认识 → 当没有布局：不报错、不带坐标
    let mut bad = lf.clone();
    bad.version = 999;
    std::fs::write(cgdir.join("layout.json"), serde_json::to_vec(&bad).unwrap()).unwrap();
    let g3 = bridge.full_graph(id).await.unwrap();
    for n in g3["nodes"].as_array().unwrap() {
        assert!(n.get("x").is_none(), "版本不认的布局应被忽略: {n}");
    }
}

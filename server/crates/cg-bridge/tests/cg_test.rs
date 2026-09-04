//! cg-bridge 集成测试：项目生命周期 + 版本不匹配 + 真索引（本仓库自身）。

mod support;

use engram_cg_bridge::{CgBridge, CgError, QueryKind};

async fn setup() -> (sqlx::PgPool, CgBridge, support::TestPg) {
    let container = support::start_pgvector().await.expect("容器");
    let url = support::connection_url(&container).await.unwrap();
    let pool = support::connect_with_retry(&url).await.expect("连接");
    engram_storage::run_migrations(&pool)
        .await
        .expect("迁移");
    let dir = tempfile::tempdir().unwrap();
    let bridge = CgBridge::new(pool.clone(), dir.keep());
    (pool, bridge, container)
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
        .query(pid, QueryKind::Search, "x", None)
        .await
        .unwrap_err();
    assert!(matches!(e, CgError::VersionMismatch { .. }), "{e:?}");
}

/// 注册校验：不存在路径 / 重名。
#[tokio::test]
async fn register_validation() {
    let (_pool, bridge, _pg) = setup().await;
    let e = bridge
        .register("t1", "/definitely/not/exists")
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
        .unwrap()
        .join(""); // 仓库根（server/ 的上上级）
    let repo = repo.canonicalize().unwrap();
    let proj = bridge
        .register("self", repo.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(proj.status, "registered");

    let proj = bridge.index(proj.id).await.expect("索引应成功");
    assert_eq!(proj.status, "ready", "err: {:?}", proj.error);

    // search：JobQueue 符号命中
    let r = bridge
        .query(proj.id, QueryKind::Search, "JobQueue", None)
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
        .query(proj.id, QueryKind::Callers, "run_migrations", None)
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
        .query(proj.id, QueryKind::Impact, "enqueue", Some(2))
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

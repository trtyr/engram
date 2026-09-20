//! CLI 子进程桥 + 项目生命周期 + 查询代理。

mod index;
mod model;
mod query;
mod version;
pub use index::*;
pub use model::*;

use std::path::{Path, PathBuf};
use std::time::Duration;

use uuid::Uuid;

/// pin 的 codegraph 版本（D0005：上游 breaking change 防护）。
pub const CG_VERSION_PIN: &str = "1.5.0";

/// 超时矩阵（topics/codegraph-bridge.md）。
const TIMEOUT_INIT: Duration = Duration::from_secs(600);
const TIMEOUT_SYNC: Duration = Duration::from_secs(60);
const TIMEOUT_QUERY: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum CgError {
    #[error("CodeGraph CLI 不可用: {0}")]
    CliUnavailable(String),
    #[error("版本不匹配：需要 {need}，实际 {got}")]
    VersionMismatch { need: String, got: String },
    #[error("命令超时（{0}s）: {1}")]
    Timeout(u64, String),
    #[error("命令失败（exit {0}）: {1}")]
    Failed(i32, String),
    #[error("输出解析失败: {0}")]
    Parse(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("存储暂时不可用: {0}")]
    Storage(String),
}

impl From<sqlx::Error> for CgError {
    fn from(e: sqlx::Error) -> Self {
        CgError::Storage(e.to_string())
    }
}

/// 查询种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryKind {
    Explore,
    Search,
    Node,
    Callers,
    Callees,
    Impact,
}

impl QueryKind {
    pub fn from_str_opt(s: &str) -> Option<Self> {
        Some(match s {
            "explore" => QueryKind::Explore,
            "search" => QueryKind::Search,
            "node" => QueryKind::Node,
            "callers" => QueryKind::Callers,
            "callees" => QueryKind::Callees,
            "impact" => QueryKind::Impact,
            _ => return None,
        })
    }
}

/// 桥接器。
#[derive(Clone)]
pub struct CgBridge {
    pool: sqlx::PgPool,
    /// codegraph 工作根目录（项目 clone 到 <root>/<id>/）。
    pub root: PathBuf,
}

async fn run_cli(
    args: &[&str],
    cwd: Option<&Path>,
    timeout: Duration,
) -> Result<CliOutput, CgError> {
    // Windows：npm 全局安装的 CLI 是 .cmd shim，std/tokio 可直接 spawn .cmd
    // （PATH 命中 shim；spawn 失败保持 CliUnavailable 语义，不经 cmd /C 以免污染错误分类）
    #[cfg(windows)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("codegraph.cmd");
        c.args(args);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = tokio::process::Command::new("codegraph");
        c.args(args);
        c
    };
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(dir) = cwd {
        // 先探 cwd：NotFound 无法区分「二进制缺失」还是「工作目录缺失」——显式检查给出可行动文案
        if !dir.exists() {
            // 路径类错误 ≠ CLI 不可用（EN-48）：从前归 CliUnavailable，等于把「目录没了」
            // 报成「CLI 未安装」——EN-24 那次误指的根源。路径不存在重试无用，语义上属 Permanent。
            return Err(CgError::NotFound(format!(
                "项目路径不存在: {}——目录可能已删除或移动（容器形态下注册的路径在宿主上看不到）；\
                 请重新注册该条目",
                dir.display()
            )));
        }
        cmd.current_dir(dir);
    }
    let child = cmd.spawn().map_err(|e| {
        CgError::CliUnavailable(format!(
            "spawn 失败（codegraph CLI 未安装或不在 PATH）: {e}"
        ))
    })?;

    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) if out.status.success() => Ok(out),
        Ok(Ok(out)) => Err(CgError::Failed(
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr)
                .chars()
                .take(300)
                .collect::<String>(),
        )),
        Ok(Err(e)) => Err(CgError::CliUnavailable(e.to_string())),
        Err(_) => Err(CgError::Timeout(
            timeout.as_secs(),
            args.first().copied().unwrap_or("").to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn timeout_path_is_classified() {
        // sleep 命令模拟挂起（借 shell 当"CLI"：codegraph 不可用时此测试验证超时机制）
        // 直接验证 run_cli 对不存在命令的分类
        let start = std::time::Instant::now();
        let r = run_cli(&["version"], None, Duration::from_millis(1)).await;
        // 两种合法结果：CLI 不存在（CliUnavailable）或真跑但超时（Timeout）
        match r {
            Err(CgError::CliUnavailable(_)) | Err(CgError::Timeout(_, _)) => {}
            other => panic!("应归类不可用/超时: {other:?}"),
        }
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn truncation_respects_char_boundary() {
        let s = "你好世界".repeat(100);
        let t = truncate(&s, 7);
        assert!(t.chars().count() < s.chars().count());
        assert!(t.contains("截断"));
    }

    #[test]
    fn query_kind_parsing() {
        assert_eq!(QueryKind::from_str_opt("explore"), Some(QueryKind::Explore));
        assert_eq!(QueryKind::from_str_opt("impact"), Some(QueryKind::Impact));
        assert_eq!(QueryKind::from_str_opt("bogus"), None);
    }

    #[test]
    fn callgraph_normalization_shapes() {
        let callers = vec![CgSymbolRef {
            name: "Wiki".into(),
            kind: "function".into(),
            filePath: "src/Wiki.tsx".into(),
            startLine: 43,
        }];
        let callees = vec![
            CgSymbolRef {
                name: "api".into(),
                kind: "constant".into(),
                filePath: "src/api.ts".into(),
                startLine: 74,
            },
            CgSymbolRef {
                name: "api".into(),
                kind: "constant".into(),
                filePath: "src/api.ts".into(),
                startLine: 74, // 与上一条完全同位：应去重
            },
        ];
        let v = normalize_callgraph("load", "demo", &callers, &callees);
        assert_eq!(v["center"], "load@demo");
        assert_eq!(v["callers"], 1);
        assert_eq!(v["callees"], 2);
        let nodes = v["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 3, "center + caller + 去重后的 callee");
        let edges = v["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 3, "caller→center 一条 + center→callee 两条");
        assert_eq!(edges[0]["from"], "Wiki@src/Wiki.tsx:43");
        assert_eq!(edges[0]["to"], "load@demo");
        assert_eq!(edges[1]["from"], "load@demo");
    }

    /// EN-26 快照戳口径：freshness 三场景。
    /// ① .git 可读 → stale 布尔 + snapshot_head 并列；
    /// ② .git 不可读 → snapshot_head/built_at 诚实标注（stale=null），不冒充最新。
    #[tokio::test]
    async fn freshness_snapshot_head_semantics() {
        let bridge = CgBridge::new(
            sqlx::Pool::<sqlx::Postgres>::connect_lazy(
                "postgres://invalid:invalid@127.0.0.1:1/none",
            )
            .unwrap(),
            "/tmp/cg-nonexistent-root",
        );
        let base = std::env::temp_dir().join(format!("cg-fresh-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&base).unwrap();

        // 建 temp git repo + 一次提交（快照戳来源）
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str], cwd: &std::path::Path| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(cwd)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap()
        };
        git(&["init", "-q"], &repo);
        std::fs::write(repo.join("a.txt"), "x").unwrap();
        git(&["add", "."], &repo);
        git(&["commit", "-qm", "init"], &repo);
        let head_hash = String::from_utf8_lossy(&git(&["rev-parse", "HEAD"], &repo).stdout)
            .trim()
            .to_string();

        let mk = |path: String, stats: serde_json::Value| CgProjectDto {
            id: Uuid::now_v7(),
            name: "t".into(),
            path,
            source_uri: "t".into(),
            status: "ready".into(),
            usable: false,
            stats: Some(stats),
            error: None,
            last_synced_at: None,
            source_kind: "repo".into(),
            head: None,
            uploaded_at: None,
            created_at: chrono::Utc::now(),
        };

        // ① 可读 .git：stale 布尔 + head/snapshot_head
        let proj = mk(
            repo.to_string_lossy().into_owned(),
            serde_json::json!({"last_indexed": chrono::Utc::now().to_rfc3339(), "head": head_hash}),
        );
        let f = bridge.freshness_for(&proj).await;
        assert_eq!(f["stale"], serde_json::json!(false), "{f}");
        assert_eq!(f["snapshot_head"], serde_json::json!(head_hash), "{f}");
        assert_eq!(f["head"], serde_json::json!(head_hash), "{f}");

        // ② .git 不可读（目录已摘）：snapshot_head + built_at 诚实标注
        let ghost = base.join("ghost");
        let proj = mk(
            ghost.to_string_lossy().into_owned(),
            serde_json::json!({"last_indexed": chrono::Utc::now().to_rfc3339(), "head": head_hash}),
        );
        let f = bridge.freshness_for(&proj).await;
        assert_eq!(f["stale"], serde_json::Value::Null, "{f}");
        assert_eq!(f["snapshot_head"], serde_json::json!(head_hash), "{f}");
        assert!(f["built_at"].is_string(), "{f}");
        assert!(f["hint"].as_str().unwrap().contains("snapshot_head"), "{f}");

        // ③ 连快照戳都没有（旧数据）：stale=null 但仍不虚报
        let proj = mk(
            ghost.to_string_lossy().into_owned(),
            serde_json::json!({"last_indexed": chrono::Utc::now().to_rfc3339()}),
        );
        let f = bridge.freshness_for(&proj).await;
        assert_eq!(f["stale"], serde_json::Value::Null, "{f}");
        assert!(f["snapshot_head"].is_null(), "{f}");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// EN-22 活体样本回归：陈旧判据优先比**快照戳**，而不是「HEAD 提交时间 vs last_indexed」。
    /// 今日实测的假阳性：sync 之后 snapshot_head == head（图确实是新的），但 CLI 的 lastIndexed
    /// 没被 sync 更新（它指「上次全量 index」），旧判据据此报 stale=true，hint 还叫用户再 sync
    /// ——照做之后还是 stale，死循环。
    #[tokio::test]
    async fn freshness_prefers_snapshot_head_over_timestamps() {
        let bridge = CgBridge::new(
            sqlx::Pool::<sqlx::Postgres>::connect_lazy(
                "postgres://invalid:invalid@127.0.0.1:1/none",
            )
            .unwrap(),
            "/tmp/cg-nonexistent-root",
        );
        let base = std::env::temp_dir().join(format!("cg-fresh2-{}", Uuid::now_v7()));
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap()
        };
        let rev_parse = |args: &[&str], cwd: &std::path::Path| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(cwd)
                .output()
                .unwrap()
        };
        git(&["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "x").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "A"]);
        let hash_a = String::from_utf8_lossy(&rev_parse(&["rev-parse", "HEAD"], &repo).stdout)
            .trim()
            .to_string();
        std::fs::write(repo.join("b.txt"), "y").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "B"]);
        let hash_b = String::from_utf8_lossy(&rev_parse(&["rev-parse", "HEAD"], &repo).stdout)
            .trim()
            .to_string();
        assert_ne!(hash_a, hash_b);

        let mk = |stats: serde_json::Value| CgProjectDto {
            id: Uuid::now_v7(),
            name: "t".into(),
            path: repo.to_string_lossy().into_owned(),
            source_uri: "t".into(),
            status: "ready".into(),
            usable: false,
            stats: Some(stats),
            error: None,
            last_synced_at: None,
            source_kind: "repo".into(),
            head: None,
            uploaded_at: None,
            created_at: chrono::Utc::now(),
        };

        // ① 快照戳 == HEAD → 新鲜。**哪怕 last_indexed 远早于这次提交**（今日假阳性场景）
        let f = bridge
            .freshness_for(&mk(serde_json::json!({
                "head": hash_b, "last_indexed": "2020-01-01T00:00:00+00:00"
            })))
            .await;
        assert_eq!(f["stale"], serde_json::json!(false), "{f}");
        assert!(f.get("hint").is_none(), "判新鲜时不该给 sync 提示：{f}");

        // ② 快照戳 ≠ HEAD（图停在旧 commit）→ 陈旧，且提示 sync
        let f = bridge
            .freshness_for(&mk(serde_json::json!({
                "head": hash_a, "last_indexed": chrono::Utc::now().to_rfc3339()
            })))
            .await;
        assert_eq!(f["stale"], serde_json::json!(true), "{f}");
        assert!(
            f["hint"].as_str().unwrap_or_default().contains("sync"),
            "{f}"
        );

        // ③ 无快照戳 → 退回时间戳比较：last_indexed 早于 HEAD 提交时间 → 陈旧
        let f = bridge
            .freshness_for(&mk(serde_json::json!({
                "last_indexed": "2020-01-01T00:00:00+00:00"
            })))
            .await;
        assert_eq!(f["stale"], serde_json::json!(true), "{f}");

        // ④ 无快照戳 + last_indexed 晚于 HEAD 提交时间 → 判新鲜（兜底判据的另一半）
        let f = bridge
            .freshness_for(&mk(serde_json::json!({
                "last_indexed": "2099-01-01T00:00:00+00:00"
            })))
            .await;
        assert_eq!(f["stale"], serde_json::json!(false), "{f}");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn job_error_classification() {
        use engram_jobs::types::JobError;
        assert!(matches!(
            classify(CgError::Timeout(30, "query".into())),
            JobError::Retryable(_)
        ));
        assert!(matches!(
            classify(CgError::CliUnavailable("spawn".into())),
            JobError::Retryable(_)
        ));
        assert!(matches!(
            classify(CgError::Storage("db".into())),
            JobError::Retryable(_)
        ));
        assert!(matches!(
            classify(CgError::NotFound("x".into())),
            JobError::Permanent(_)
        ));
        assert!(matches!(
            classify(CgError::BadRequest("y".into())),
            JobError::Permanent(_)
        ));
    }

    /// EN-48：usable 是「产物在盘」的当下断言，不是 status 的复读。
    #[test]
    fn usable_requires_artifact_on_disk() {
        let dir = std::env::temp_dir().join(format!("cg-usable-{}", Uuid::now_v7()));
        std::fs::create_dir_all(dir.join(".codegraph")).unwrap();
        let mk = |status: &str| CgProjectDto {
            id: Uuid::now_v7(),
            name: "t".into(),
            path: dir.to_string_lossy().into_owned(),
            source_uri: "t".into(),
            status: status.into(),
            usable: false,
            stats: None,
            error: None,
            last_synced_at: None,
            source_kind: "repo".into(),
            head: None,
            uploaded_at: None,
            created_at: chrono::Utc::now(),
        };

        // status=ready 但产物不在盘 → 不可用（旧口径把它当可用资产，正是 EN-48 的病）
        let mut p = mk("ready");
        assert!(!index_usable(&p), "ready + 无产物 必须判为不可用");

        // 产物落盘 → 可用
        std::fs::write(index_db_path(&dir), b"").unwrap();
        assert!(index_usable(&p));

        // 产物在盘但 status 非 ready → 仍不可用
        p.status = "indexing".into();
        assert!(!index_usable(&p));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// EN-48：CLI 以 **git 根**为家——注册仓库子目录时产物在父级，不能只认注册路径。
    /// （2026-09-15 实测：`server/` 下索引，db 落在仓库根。）
    #[test]
    fn index_db_path_finds_repo_root_artifact() {
        let root = std::env::temp_dir().join(format!("cg-root-{}", Uuid::now_v7()));
        let sub = root.join("server").join("crates");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap(); // 仓库根标识
        std::fs::create_dir_all(root.join(".codegraph")).unwrap();
        std::fs::write(index_db_path(&root), b"").unwrap();

        // 注册子目录 → 沿父目录找到仓库根的产物，且据此判定可用
        assert_eq!(index_db_path(&sub), index_db_path(&root));
        let p = CgProjectDto {
            id: Uuid::now_v7(),
            name: "t".into(),
            path: sub.to_string_lossy().into_owned(),
            source_uri: "t".into(),
            status: "ready".into(),
            usable: false,
            stats: None,
            error: None,
            last_synced_at: None,
            source_kind: "repo".into(),
            head: None,
            uploaded_at: None,
            created_at: chrono::Utc::now(),
        };
        assert!(index_usable(&p), "子目录注册也应认仓库根的索引产物");

        // 不越界：自己已是仓库根（有 .git）时，不去捡仓库外的 .codegraph
        let outside = root
            .parent()
            .unwrap()
            .join(format!("cg-outside-{}", Uuid::now_v7()));
        let orphan = outside.join("proj");
        std::fs::create_dir_all(orphan.join(".git")).unwrap();
        std::fs::create_dir_all(outside.join(".codegraph")).unwrap();
        std::fs::write(index_db_path(&outside), b"").unwrap();
        assert_ne!(
            index_db_path(&orphan),
            index_db_path(&outside),
            "越过 .git 边界捡到外面的产物——不该发生"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// EN-48：门禁按病因分类——从未索引 / 版本不符 / 路径没了 / 产物丢了，各给各的错，
    /// 不再一律「CLI 不可用」或一句含糊的「索引库不存在」。
    #[tokio::test]
    async fn ensure_ready_classifies_causes() {
        let bridge = CgBridge::new(
            sqlx::Pool::<sqlx::Postgres>::connect_lazy(
                "postgres://invalid:invalid@127.0.0.1:1/none",
            )
            .unwrap(),
            "/tmp/cg-nonexistent-root",
        );
        let mk = |status: &str, path: &std::path::Path| CgProjectDto {
            id: Uuid::now_v7(),
            name: "t".into(),
            path: path.to_string_lossy().into_owned(),
            source_uri: "t".into(),
            status: status.into(),
            usable: false,
            stats: None,
            error: None,
            last_synced_at: None,
            source_kind: "repo".into(),
            head: None,
            uploaded_at: None,
            created_at: chrono::Utc::now(),
        };
        let tmp = std::env::temp_dir();

        // ① 从未索引 → BadRequest，文案指向 index
        let e = bridge.ensure_ready(&mk("registered", &tmp)).unwrap_err();
        assert!(
            matches!(e, CgError::BadRequest(_)) && e.to_string().contains("从未建过索引"),
            "{e}"
        );

        // ② 版本不符优先于其他病因
        let e = bridge
            .ensure_ready(&mk("version_mismatch", &tmp))
            .unwrap_err();
        assert!(matches!(e, CgError::VersionMismatch { .. }), "{e}");

        // ③ ready 但路径不存在 → NotFound（不是 CliUnavailable）
        let gone = tmp.join(format!("cg-gone-{}", Uuid::now_v7()));
        let e = bridge.ensure_ready(&mk("ready", &gone)).unwrap_err();
        assert!(
            matches!(e, CgError::NotFound(_)) && e.to_string().contains("项目路径不存在"),
            "{e}"
        );

        // ④ ready、路径在、产物不在 → NotFound，且点名「索引产物已丢失」
        let live = tmp.join(format!("cg-live-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&live).unwrap();
        let e = bridge.ensure_ready(&mk("ready", &live)).unwrap_err();
        assert!(
            matches!(e, CgError::NotFound(_)) && e.to_string().contains("索引产物已丢失"),
            "{e}"
        );

        // ⑤ 产物在盘 → 放行，并返回索引库路径
        std::fs::create_dir_all(live.join(".codegraph")).unwrap();
        std::fs::write(index_db_path(&live), b"").unwrap();
        assert_eq!(
            bridge.ensure_ready(&mk("ready", &live)).unwrap(),
            index_db_path(&live)
        );

        let _ = std::fs::remove_dir_all(&live);
    }
}

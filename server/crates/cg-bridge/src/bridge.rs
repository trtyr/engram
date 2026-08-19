//! CLI 子进程桥 + 项目生命周期 + 查询代理。

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

/// 项目 DTO。
#[derive(Debug, serde::Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct CgProjectDto {
    pub id: Uuid,
    pub name: String,
    pub path: String,
    pub source_uri: String,
    pub status: String,
    #[schema(value_type = Option<Object>)]
    pub stats: Option<serde_json::Value>,
    pub error: Option<String>,
    pub last_synced_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// 桥接器。
#[derive(Clone)]
pub struct CgBridge {
    pool: sqlx::PgPool,
    /// codegraph 工作根目录（项目 clone 到 <root>/<id>/）。
    pub root: PathBuf,
}

impl CgBridge {
    pub fn new(pool: sqlx::PgPool, root: impl Into<PathBuf>) -> Self {
        Self {
            pool,
            root: root.into(),
        }
    }

    // ---------- CLI 原语 ----------

    /// 探测 CLI 版本（失败 = CLI 不可用）。
    pub async fn detect_version(&self) -> Result<String, CgError> {
        let out = run_cli(&["version"], None, TIMEOUT_QUERY).await?;
        let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if v.is_empty() {
            return Err(CgError::Parse("version 输出为空".into()));
        }
        Ok(v)
    }

    /// 版本守卫：pin 不符 → VersionMismatch。
    pub async fn ensure_version(&self) -> Result<(), CgError> {
        let v = self.detect_version().await?;
        // 兼容 "1.5.0" 与 "codegraph 1.5.0" 形态
        let ver = v.split_whitespace().last().unwrap_or(&v);
        if ver != CG_VERSION_PIN {
            return Err(CgError::VersionMismatch {
                need: CG_VERSION_PIN.into(),
                got: ver.into(),
            });
        }
        Ok(())
    }

    // ---------- 项目生命周期 ----------

    /// 注册项目：本地路径（须存在）或 git URL（clone depth 1 到工作目录）。
    pub async fn register(&self, name: &str, source_uri: &str) -> Result<CgProjectDto, CgError> {
        let existing: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM cg_projects WHERE name = $1")
                .bind(name)
                .fetch_optional(&self.pool)
                .await?;
        if existing.is_some() {
            return Err(CgError::BadRequest(format!("项目名 {name} 已存在")));
        }

        let id = Uuid::now_v7();
        let workdir = self.root.join(id.to_string());
        let (path, uri) = if source_uri.starts_with("http://")
            || source_uri.starts_with("https://")
            || source_uri.ends_with(".git")
        {
            // git clone --depth 1
            tokio::fs::create_dir_all(&self.root).await.ok();
            let out = tokio::process::Command::new("git")
                .args(["clone", "--depth", "1", source_uri])
                .arg(&workdir)
                .output()
                .await
                .map_err(|e| CgError::BadRequest(format!("git 不可用: {e}")))?;
            if !out.status.success() {
                return Err(CgError::BadRequest(format!(
                    "clone 失败: {}",
                    String::from_utf8_lossy(&out.stderr)
                        .chars()
                        .take(300)
                        .collect::<String>()
                )));
            }
            (
                workdir.to_string_lossy().into_owned(),
                source_uri.to_string(),
            )
        } else {
            let p = Path::new(source_uri);
            if !p.exists() {
                return Err(CgError::BadRequest(format!("本地路径不存在: {source_uri}")));
            }
            (source_uri.to_string(), source_uri.to_string())
        };

        let row = sqlx::query_as::<_, CgProjectDto>(
            "INSERT INTO cg_projects (id, name, path, source_uri, status) \
             VALUES ($1, $2, $3, $4, 'registered') RETURNING *",
        )
        .bind(id)
        .bind(name)
        .bind(&path)
        .bind(&uri)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list(&self) -> Result<Vec<CgProjectDto>, CgError> {
        Ok(
            sqlx::query_as::<_, CgProjectDto>("SELECT * FROM cg_projects ORDER BY created_at DESC")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn get(&self, id: Uuid) -> Result<CgProjectDto, CgError> {
        sqlx::query_as::<_, CgProjectDto>("SELECT * FROM cg_projects WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| CgError::NotFound(format!("项目 {id} 不存在")))
    }

    /// 建索引（registered → indexing → ready/error）。超时 10min。
    pub async fn index(&self, id: Uuid) -> Result<CgProjectDto, CgError> {
        let proj = self.get(id).await?;
        self.ensure_version().await?;

        self.set_status(id, "indexing", None, None).await?;
        match run_cli(&["init"], Some(Path::new(&proj.path)), TIMEOUT_INIT).await {
            Ok(_) => {
                let stats = self.read_stats(Path::new(&proj.path)).await;
                self.set_status(id, "ready", stats.as_ref(), None).await?;
            }
            Err(e) => {
                let msg = e.to_string();
                self.set_status(id, "error", None, Some(&msg)).await?;
                return Err(e);
            }
        }
        self.get(id).await
    }

    /// 增量同步。超时 60s。
    pub async fn sync(&self, id: Uuid) -> Result<CgProjectDto, CgError> {
        let proj = self.get(id).await?;
        self.ensure_version().await?;
        match run_cli(&["sync"], Some(Path::new(&proj.path)), TIMEOUT_SYNC).await {
            Ok(_) => {
                let stats = self.read_stats(Path::new(&proj.path)).await;
                self.set_status(id, "ready", stats.as_ref(), None).await?;
                sqlx::query("UPDATE cg_projects SET last_synced_at = now() WHERE id = $1")
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
            }
            Err(e) => {
                let msg = e.to_string();
                self.set_status(id, "error", None, Some(&msg)).await?;
                return Err(e);
            }
        }
        self.get(id).await
    }

    async fn set_status(
        &self,
        id: Uuid,
        status: &str,
        stats: Option<&serde_json::Value>,
        error: Option<&str>,
    ) -> Result<(), CgError> {
        sqlx::query("UPDATE cg_projects SET status = $2, stats = $3, error = $4, updated_at = now() WHERE id = $1")
            .bind(id)
            .bind(status)
            .bind(stats.map(sqlx::types::Json))
            .bind(error)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn read_stats(&self, path: &Path) -> Option<serde_json::Value> {
        let out = run_cli(&["status", "--json"], Some(path), TIMEOUT_QUERY)
            .await
            .ok()?;
        serde_json::from_slice(&out.stdout).ok()
    }

    // ---------- 查询代理 ----------

    /// 代理查询。explore/node 返回 Markdown（包 JSON {kind, text}）；其余返回归一 JSON。
    pub async fn query(
        &self,
        id: Uuid,
        kind: QueryKind,
        target: &str,
        depth: Option<u32>,
    ) -> Result<serde_json::Value, CgError> {
        let proj = self.get(id).await?;
        if proj.status == "version_mismatch" {
            return Err(CgError::VersionMismatch {
                need: CG_VERSION_PIN.into(),
                got: "unknown".into(),
            });
        }
        if proj.status != "ready" {
            return Err(CgError::BadRequest(format!(
                "项目未就绪（{}）",
                proj.status
            )));
        }
        let path = Path::new(&proj.path);
        self.ensure_version().await?;

        let (args, timeout): (Vec<String>, Duration) = match kind {
            QueryKind::Explore => {
                // explore 无 --json：Markdown 文本
                let mut a = vec!["explore".to_string(), target.to_string()];
                if let Some(mf) = depth {
                    a.push("--max-files".into());
                    a.push(mf.to_string());
                }
                (a, TIMEOUT_QUERY)
            }
            QueryKind::Search => (
                vec![
                    "query".into(),
                    target.to_string(),
                    "--json".into(),
                    "--limit".into(),
                    "20".into(),
                ],
                TIMEOUT_QUERY,
            ),
            QueryKind::Node => (vec!["node".into(), target.to_string()], TIMEOUT_QUERY),
            QueryKind::Callers => (
                vec!["callers".into(), target.to_string(), "--json".into()],
                TIMEOUT_QUERY,
            ),
            QueryKind::Callees => (
                vec!["callees".into(), target.to_string(), "--json".into()],
                TIMEOUT_QUERY,
            ),
            QueryKind::Impact => {
                let mut a = vec!["impact".into(), target.to_string(), "--json".into()];
                if let Some(d) = depth {
                    a.push("--depth".into());
                    a.push(d.to_string());
                }
                (a, TIMEOUT_QUERY)
            }
        };
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = run_cli(&arg_refs, Some(path), timeout).await?;
        let stdout = String::from_utf8_lossy(&out.stdout);

        match kind {
            QueryKind::Explore => Ok(serde_json::json!({
                "kind": "explore",
                "text": truncate(&stdout, 24_000),
                "truncated": stdout.len() > 24_000,
            })),
            QueryKind::Node => Ok(serde_json::json!({
                "kind": "node",
                "text": truncate(&stdout, 24_000),
            })),
            _ => serde_json::from_str(stdout.trim())
                .map_err(|e| CgError::Parse(format!("JSON 归一失败: {e}"))),
        }
    }

    /// 标记全部项目版本不匹配（CLI 升级后调用）。
    pub async fn mark_all_version_mismatch(&self, actual: &str) -> Result<u64, CgError> {
        let r = sqlx::query(
            "UPDATE cg_projects SET status = 'version_mismatch', error = $1, updated_at = now() WHERE status != 'version_mismatch'",
        )
        .bind(format!("CLI 版本 {actual} != pin {CG_VERSION_PIN}"))
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while end < s.len() && !s.is_char_boundary(end) {
            end += 1;
        }
        format!("{}\n…(截断，用更精确的查询)", &s[..end])
    }
}

/// CLI 执行原语：spawn + 超时 + 错误归一（测试通过 `#[cfg(test)]` 注入模拟）。
type CliOutput = std::process::Output;

async fn run_cli(
    args: &[&str],
    cwd: Option<&Path>,
    timeout: Duration,
) -> Result<CliOutput, CgError> {
    let mut cmd = tokio::process::Command::new("codegraph");
    cmd.args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let child = cmd
        .spawn()
        .map_err(|e| CgError::CliUnavailable(format!("spawn 失败（codegraph 未安装?）: {e}")))?;

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
}

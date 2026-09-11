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

/// CLI 可用性（GET /codegraph/status 响应）。
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct CliStatus {
    pub available: bool,
    pub version: Option<String>,
    pub pin: String,
}

/// callers/callees --json 的条目（真实 schema 探测自 CLI 1.5.0；camelCase 原样映射）。
#[derive(Debug, serde::Deserialize)]
#[allow(non_snake_case)]
pub(crate) struct CgSymbolRef {
    name: String,
    kind: String,
    #[serde(default)]
    filePath: String,
    #[serde(default)]
    startLine: i64,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct CallersShape {
    #[serde(default)]
    callers: Vec<CgSymbolRef>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct CalleesShape {
    #[serde(default)]
    callees: Vec<CgSymbolRef>,
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
        // 同一来源（路径/仓库）只许注册一次——避免同库多份索引
        let dup: Option<(Uuid, String)> =
            sqlx::query_as("SELECT id, name FROM cg_projects WHERE source_uri = $1")
                .bind(source_uri)
                .fetch_optional(&self.pool)
                .await?;
        if let Some((_, holder)) = dup {
            return Err(CgError::BadRequest(format!(
                "该来源已注册为项目 {holder}——同源一个索引，直接复用即可"
            )));
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
                return Err(CgError::BadRequest(format!(
                    "本地路径不存在: {source_uri}——注意路径按**服务端**文件系统校验 \
                     （MCP 客户端在另一台机器上时，它本地的路径服务端看不到，请改用 git URL）"
                )));
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
        // 首次 init；已有 .codegraph 目录则全量重建（CLI 的 index 命令）
        let marker = Path::new(&proj.path).join(".codegraph");
        let cmd: &str = if marker.exists() { "index" } else { "init" };
        match run_cli(&[cmd], Some(Path::new(&proj.path)), TIMEOUT_INIT).await {
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

    /// 索引统计：CLI status --json 归一为前端契约 {files, symbols, edges, by_kind}。
    /// （CLI 字段是 fileCount/nodeCount/edgeCount——此前直透导致前端恒显示 ?。）
    async fn read_stats(&self, path: &Path) -> Option<serde_json::Value> {
        let out = run_cli(&["status", "--json"], Some(path), TIMEOUT_QUERY)
            .await
            .ok()?;
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        Some(serde_json::json!({
            "files": v.get("fileCount")?,
            "symbols": v.get("nodeCount")?,
            "edges": v.get("edgeCount")?,
            "by_kind": v.get("nodesByKind").cloned().unwrap_or(serde_json::json!({})),
            "last_indexed": v.get("lastIndexed").cloned().unwrap_or(serde_json::Value::Null),
        }))
    }

    /// explore 符号大纲（R 报告 P0-3）：直接读索引库（.codegraph/codegraph.db，
    /// 只读——与 full_graph 同哲学），按「路径/文件名含 target 或符号名含 target」取
    /// 符号清单（name/kind/行号/签名截断），不返回任何源码正文。
    /// 返回 None = 大纲数据源不可用或零命中（调用方落回 CLI explore）。
    async fn explore_outline(
        &self,
        path: &Path,
        target: &str,
    ) -> Result<Option<serde_json::Value>, CgError> {
        let db_path = path.join(".codegraph").join("codegraph.db");
        if !db_path.exists() {
            return Ok(None);
        }
        let db_str = db_path.to_string_lossy().into_owned();
        let target = target.trim().to_string();
        let job = tokio::task::spawn_blocking(
            move || -> Result<Option<serde_json::Value>, CgError> {
                let conn = rusqlite::Connection::open_with_flags(
                    &db_str,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )
                .map_err(|e| CgError::Parse(format!("索引库打开失败: {e}")))?;
                // LIKE 通配符转义（target 是自由文本，% _ 会改变匹配语义）
                let like = format!(
                    "%{}%",
                    target
                        .replace('\\', "\\\\")
                        .replace('%', "\\%")
                        .replace('_', "\\_")
                );
                let mut stmt = conn
                    .prepare(
                        "SELECT name, kind, file_path, start_line, end_line, signature \
                     FROM nodes \
                     WHERE file_path LIKE ?1 ESCAPE '\\' OR name LIKE ?1 ESCAPE '\\' \
                     ORDER BY file_path, start_line LIMIT 200",
                    )
                    .map_err(|e| CgError::Parse(format!("索引查询失败: {e}")))?;
                let rows: Vec<(String, String, String, i64, i64, Option<String>)> = stmt
                    .query_map(rusqlite::params![like], |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                        ))
                    })
                    .map_err(|e| CgError::Parse(format!("索引查询失败: {e}")))?
                    .collect::<Result<_, _>>()
                    .map_err(|e| CgError::Parse(format!("索引读取失败: {e}")))?;
                if rows.is_empty() {
                    return Ok(None);
                }
                let truncate = |s: &str| -> String {
                    if s.chars().count() <= 120 {
                        s.to_string()
                    } else {
                        s.chars().take(120).collect::<String>() + "…"
                    }
                };
                let symbols: Vec<serde_json::Value> = rows
                    .iter()
                    .map(|(name, kind, fp, sl, el, sig)| {
                        let mut o = serde_json::json!({
                            "name": name, "kind": kind, "file_path": fp,
                            "line": sl, "end_line": el,
                        });
                        if let Some(s) = sig {
                            o["signature"] = serde_json::json!(truncate(s));
                        }
                        o
                    })
                    .collect();
                // 文件清单（去重保序 + 每文件符号数）
                let mut files: Vec<(String, usize)> = Vec::new();
                for (_, _, fp, _, _, _) in &rows {
                    match files.last_mut() {
                        Some((p, n)) if p == fp => *n += 1,
                        _ => files.push((fp.clone(), 1)),
                    }
                }
                let files: Vec<serde_json::Value> = files
                    .into_iter()
                    .map(|(p, n)| serde_json::json!({"path": p, "symbols": n}))
                    .collect();
                let total_hint = if symbols.len() >= 200 {
                    "（已达 200 上限——用更具体的目录/符号名缩小范围）"
                } else {
                    ""
                };
                Ok(Some(serde_json::json!({
                    "kind": "explore",
                    "mode": "outline",
                    "target": target,
                    "files": files,
                    "symbols": symbols,
                    "hint": format!(
                        "符号大纲（无源码）。看单个符号的源码与调用列表：kind=node；\
                         看影响面：kind=callers/impact；要完整源码文件：本查询传 include_source=true。{total_hint}"
                    ),
                })))
            },
        );
        job.await
            .map_err(|e| CgError::Parse(format!("索引读取线程崩溃: {e}")))?
    }

    // ---------- 查询代理 ----------

    /// 代理查询。explore/node 返回 Markdown（包 JSON {kind, text}）；其余返回归一 JSON。
    /// explore 默认（include_source=false）返回**符号大纲**（直接读索引库，不再拖整份源码——
    /// R 报告 P0-3：一次 explore 曾拖回 300+ 行 App.tsx 全文）；要看源码传 include_source=true
    /// 走 CLI 原生输出。大纲数据源缺失（无索引库）时自动回落 CLI。
    pub async fn query(
        &self,
        id: Uuid,
        kind: QueryKind,
        target: &str,
        depth: Option<u32>,
        include_source: bool,
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

        if kind == QueryKind::Explore
            && !include_source
            && let Some(outline) = self.explore_outline(path, target).await?
        {
            return Ok(outline);
            // 大纲不可用 → 落回 CLI explore 源码形态
        }
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

    /// 删除项目：移除注册行；工作目录在本桥 root 之下（git clone 的）连目录一起清，
    /// 本地路径项目不动用户的源码。返回是否删除了工作目录。
    pub async fn delete(&self, id: Uuid) -> Result<bool, CgError> {
        let proj = self.get(id).await?;
        let removed = sqlx::query("DELETE FROM cg_projects WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if removed == 0 {
            return Err(CgError::NotFound(format!("项目 {id} 不存在")));
        }
        let workdir = Path::new(&proj.path);
        let ours = self
            .root
            .canonicalize()
            .ok()
            .and_then(|root| workdir.canonicalize().ok().map(|w| w.starts_with(&root)))
            .unwrap_or(false);
        if ours {
            let _ = tokio::fs::remove_dir_all(workdir).await;
            return Ok(true);
        }
        Ok(false)
    }

    /// CLI 可用性（前端状态条）：版本探测失败 = 不可用。
    pub async fn cli_status(&self) -> CliStatus {
        match self.detect_version().await {
            Ok(v) => CliStatus {
                available: true,
                version: Some(v),
                pin: CG_VERSION_PIN.into(),
            },
            Err(e) => CliStatus {
                available: false,
                version: None,
                pin: format!("{}（不可用: {e}）", CG_VERSION_PIN),
            },
        }
    }

    /// 文件级全图：把全部跨文件依赖边按文件聚合（imports/calls/instantiates/references，
    /// 排除 contains 与自环）。这是「整个项目的调用图」的正确粒度——符号级动辄几百节点不可读，
    /// 文件级通常几十个节点正好。数据源：.codegraph/codegraph.db（只读打开，schema 随 pin 稳定）。
    pub async fn full_graph(&self, id: Uuid) -> Result<serde_json::Value, CgError> {
        let proj = self.get(id).await?;
        if proj.status != "ready" {
            return Err(CgError::BadRequest(format!(
                "项目未就绪（{}）——先建索引",
                proj.status
            )));
        }
        let db_path = Path::new(&proj.path)
            .join(".codegraph")
            .join("codegraph.db");
        if !db_path.exists() {
            return Err(CgError::NotFound(format!(
                "索引库不存在（{}）——重新建索引",
                db_path.display()
            )));
        }
        // 阻塞读小库（<10MB）：spawn_blocking 防占 worker
        let db_str = db_path.to_string_lossy().into_owned();
        let v = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, CgError> {
            let conn = rusqlite::Connection::open_with_flags(
                &db_str,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .map_err(|e| CgError::Parse(format!("索引库打开失败: {e}")))?;
            let mut stmt = conn
                .prepare(
                    "SELECT sf.path, tf.path, count(*) AS w FROM edges e                      JOIN nodes ns ON ns.id = e.source JOIN files sf ON sf.path = ns.file_path                      JOIN nodes nt ON nt.id = e.target JOIN files tf ON tf.path = nt.file_path                      WHERE sf.path != tf.path AND e.kind != 'contains'                      GROUP BY sf.path, tf.path ORDER BY w DESC",
                )
                .map_err(|e| CgError::Parse(format!("索引查询失败: {e}")))?;
            let rows: Vec<(String, String, i64)> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map_err(|e| CgError::Parse(format!("索引查询失败: {e}")))?
                .collect::<Result<_, _>>()
                .map_err(|e| CgError::Parse(format!("索引读取失败: {e}")))?;

            let file_name = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
            let mut nodes: Vec<serde_json::Value> = Vec::new();
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut edges = Vec::new();
            for (from, to, w) in rows {
                for p in [&from, &to] {
                    if seen.insert(p.clone()) {
                        nodes.push(serde_json::json!({
                            "id": p, "name": file_name(p), "kind": "file", "role": "file",
                        }));
                    }
                }
                edges.push(serde_json::json!({
                    "from": from, "to": to, "rel": "imports", "weight": w,
                }));
            }
            Ok(serde_json::json!({
                "mode": "files",
                "files": nodes.len(),
                "nodes": nodes,
                "edges": edges,
            }))
        })
        .await
        .map_err(|e| CgError::Parse(format!("索引读取线程崩溃: {e}")))??;
        Ok(v)
    }

    /// 调用图归一：符号为中心，callers/callees 展开成 nodes+edges 子图。
    /// CLI 的 callers/callees 是扁平列表（{name,kind,filePath,startLine}）无显式边——
    /// 中心与列表项连线即语义。节点 id 用 name@path:line 唯一化。
    /// 某一侧（如无 caller）失败不拖垮整图，按空处理。
    pub async fn graph(&self, id: Uuid, symbol: &str) -> Result<serde_json::Value, CgError> {
        let proj = self.get(id).await?;
        if proj.status != "ready" {
            return Err(CgError::BadRequest(format!(
                "项目未就绪（{}）——先建索引",
                proj.status
            )));
        }
        let path = Path::new(&proj.path);
        self.ensure_version().await?;

        let callers = run_cli(&["callers", symbol, "--json"], Some(path), TIMEOUT_QUERY).await;
        let callees = run_cli(&["callees", symbol, "--json"], Some(path), TIMEOUT_QUERY).await;

        let callers: Vec<CgSymbolRef> = match callers {
            Ok(out) => serde_json::from_slice::<CallersShape>(&out.stdout)
                .map(|s| s.callers)
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        let callees: Vec<CgSymbolRef> = match callees {
            Ok(out) => serde_json::from_slice::<CalleesShape>(&out.stdout)
                .map(|s| s.callees)
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        if callers.is_empty() && callees.is_empty() {
            return Err(CgError::NotFound(format!(
                "符号 {symbol:?} 无调用关系——确认名字正确且已建索引（可先用 query 搜索）"
            )));
        }

        Ok(normalize_callgraph(symbol, &proj.name, &callers, &callees))
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

/// callers/callees 扁平列表 → nodes+edges 子图（纯函数，单测覆盖 schema 归一）。
pub(crate) fn normalize_callgraph(
    symbol: &str,
    project_name: &str,
    callers: &[CgSymbolRef],
    callees: &[CgSymbolRef],
) -> serde_json::Value {
    let center_id = format!("{symbol}@{project_name}");
    let mut nodes = vec![serde_json::json!({
        "id": center_id, "name": symbol, "kind": "center", "role": "center",
    })];
    let mut edges = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    seen.insert(center_id.clone());
    let mut push_side = |list: &[CgSymbolRef], role: &str, rel: &str, outbound: bool| {
        for c in list {
            let nid = format!("{}@{}:{}", c.name, c.filePath, c.startLine);
            if seen.insert(nid.clone()) {
                nodes.push(serde_json::json!({
                    "id": nid, "name": c.name, "kind": c.kind,
                    "filePath": c.filePath, "line": c.startLine, "role": role,
                }));
            }
            let (from, to) = if outbound {
                (center_id.clone(), nid)
            } else {
                (nid, center_id.clone())
            };
            edges.push(serde_json::json!({ "from": from, "to": to, "rel": rel }));
        }
    };
    push_side(callers, "caller", "caller", false);
    push_side(callees, "callee", "callee", true);
    serde_json::json!({
        "symbol": symbol,
        "center": center_id,
        "callers": callers.len(),
        "callees": callees.len(),
        "nodes": nodes,
        "edges": edges,
    })
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

/// job 错误分类：超时/CLI 抖动 → Retryable 退避重试；存储抖动 → Retryable；其余 Permanent。
fn classify(e: CgError) -> engram_jobs::types::JobError {
    use engram_jobs::types::JobError;
    match e {
        CgError::Timeout(_, _) | CgError::CliUnavailable(_) => JobError::Retryable(e.to_string()),
        CgError::Storage(m) => JobError::Retryable(format!("存储暂时不可用: {m}")),
        other => JobError::Permanent(other.to_string()),
    }
}

/// 注册 CodeGraph 域 job handlers（main 装配用）：cg_index / cg_sync。
/// payload: {"project_id": uuid}。一切长操作走队列——索引不再阻塞 HTTP 10 分钟。
pub fn register_handlers(
    runner: engram_jobs::Runner,
    bridge_root: std::path::PathBuf,
) -> engram_jobs::Runner {
    let root_index = bridge_root.clone();
    runner
        .register("cg_index", move |ctx| {
            let root = root_index.clone();
            async move {
                let id: Uuid = ctx
                    .job
                    .payload
                    .0
                    .get("project_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or_else(|| {
                        engram_jobs::types::JobError::Permanent("payload 缺 project_id".into())
                    })?;
                let bridge = CgBridge::new(ctx.pool().clone(), root);
                let dto = bridge.index(id).await.map_err(classify)?;
                Ok(serde_json::json!({"project_id": id, "status": dto.status}))
            }
        })
        .register("cg_sync", move |ctx| {
            let root = bridge_root.clone();
            async move {
                let id: Uuid = ctx
                    .job
                    .payload
                    .0
                    .get("project_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or_else(|| {
                        engram_jobs::types::JobError::Permanent("payload 缺 project_id".into())
                    })?;
                let bridge = CgBridge::new(ctx.pool().clone(), root);
                let dto = bridge.sync(id).await.map_err(classify)?;
                Ok(serde_json::json!({"project_id": id, "status": dto.status}))
            }
        })
}

/// CLI 执行原语：spawn + 超时 + 错误归一（测试通过 `#[cfg(test)]` 注入模拟）。
type CliOutput = std::process::Output;

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
            return Err(CgError::CliUnavailable(format!(
                "项目路径不存在: {}（容器部署下宿主路径不可见——需挂载该目录，或仅宿主 dev 形态使用 codegraph）",
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
}

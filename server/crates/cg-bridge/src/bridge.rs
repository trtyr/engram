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
    /// 当下可用性（**派生字段，不落库**，EN-48）：`status`/`stats` 只说明「最后一次索引成功过」，
    /// 是历史陈述；本字段才是「现在能不能用」的当下断言——status=ready **且**索引产物确在盘。
    /// 目录被重新 clone / 清理后 `.codegraph/` 不会自己回来，那时 status 仍是 ready，靠本字段区分。
    #[sqlx(default)]
    pub usable: bool,
    #[schema(value_type = Option<Object>)]
    pub stats: Option<serde_json::Value>,
    pub error: Option<String>,
    pub last_synced_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 条目来源：repo（服务端路径/git clone，本机索引）| upload（客户端推产物，公网模型）
    pub source_kind: String,
    /// 上传型：客户端声明的 commit hash（声明式新鲜度——服务端不读代码，只存声明）
    pub head: Option<String>,
    /// 上传型：最近一次产物上传时间
    pub uploaded_at: Option<chrono::DateTime<chrono::Utc>>,
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

/// 索引产物路径（`<仓库根>/.codegraph/codegraph.db`）——**唯一出处**（EN-48）。
///
/// 注意 CLI 的「家」是**仓库根**而非 cwd：在仓库子目录里跑 `init`/`index`，它把
/// `.gitignore` 留在 cwd、数据库写到 git 根去（2026-09-15 实测：在 `server/` 下索引，
/// db 落在 `engram/.codegraph/codegraph.db`）。所以注册路径若是仓库内子目录，产物在父级。
/// 查找顺序：注册路径自己 → 沿父目录向上，走到含 `.git` 的那一级为止（不越界到仓库外）。
fn index_db_path(proj_path: &Path) -> PathBuf {
    let direct = proj_path.join(".codegraph").join("codegraph.db");
    if direct.is_file() {
        return direct;
    }
    let mut cur = proj_path.to_path_buf();
    loop {
        if cur.join(".git").exists() {
            break; // 已到仓库根：再往上不属于本项目
        }
        let Some(parent) = cur.parent() else {
            break;
        };
        let candidate = parent.join(".codegraph").join("codegraph.db");
        if candidate.is_file() {
            return candidate;
        }
        cur = parent.to_path_buf();
    }
    direct // 都没找到 → 回「按注册路径推导」的位置，错误文案里好定位
}

/// 当下可用性（EN-48）：`status == "ready"` **且**索引产物确在盘。
/// 纯只读判定（一次 stat），可在 list 里逐项现算——不落库、不做缓存。
pub fn index_usable(proj: &CgProjectDto) -> bool {
    proj.status == "ready" && index_db_path(Path::new(&proj.path)).is_file()
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

    /// 产物上传（公网多Agent P001 步骤4）：客户端本机 codegraph CLI index 后，
    /// 上传 `.codegraph/codegraph.db` + HEAD——服务端只存产物 + 声明式新鲜度
    /// （head/uploaded_at），无代码、无 git 凭证。CLI 是基础设施（CG_VERSION_PIN），
    /// 客户端宿主零依赖。
    ///
    /// name 不存在则新建条目（status=ready）；存在且为 upload 型则覆盖产物；
    /// repo 型拒绝覆盖（本机索引不归上传通道管）。
    pub async fn upload_artifact(
        &self,
        name: &str,
        head: &str,
        db_bytes: &[u8],
    ) -> Result<CgProjectDto, CgError> {
        // 校验：head 是 commit hash（7~40 位 hex，短/长 SHA 都收）
        let head = head.trim();
        if !(7..=40).contains(&head.len()) || !head.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(CgError::BadRequest(format!(
                "head 不是合法 commit hash（7~40 位 hex，收到 {} 字符）——客户端本机 `git rev-parse HEAD` 取",
                head.len()
            )));
        }
        // 校验：SQLite 魔数（坏产物当场拒——「能存进去但查不了」才是最差的体验）
        const SQLITE_MAGIC: &[u8] = b"SQLite format 3\x00";
        if db_bytes.len() < SQLITE_MAGIC.len() || &db_bytes[..SQLITE_MAGIC.len()] != SQLITE_MAGIC {
            return Err(CgError::BadRequest(
                "db 不是 SQLite 文件（缺「SQLite format 3」魔数）——请上传 codegraph CLI 产出的 \
                 .codegraph/codegraph.db 本体（原始二进制，不要压缩/文本化）"
                    .into(),
            ));
        }
        // 上限 256MB（单用户系统，一次 HTTP body 可承载；再大说明仓库该拆了）
        if db_bytes.len() > 256 * 1024 * 1024 {
            return Err(CgError::BadRequest(format!(
                "db 超限（{} MB > 256 MB）——拆分仓库或精简索引范围后重传",
                db_bytes.len() / 1024 / 1024
            )));
        }

        let existing: Option<(Uuid, String)> =
            sqlx::query_as("SELECT id, source_kind FROM cg_projects WHERE name = $1")
                .bind(name)
                .fetch_optional(&self.pool)
                .await?;
        let id = match existing {
            Some((id, kind)) => {
                if kind != "upload" {
                    return Err(CgError::BadRequest(format!(
                        "项目 {name} 是 repo 型（服务端本机索引）——产物上传只作用于 upload 型条目；\
                         请换名注册，或先 delete 再以 upload 重建"
                    )));
                }
                id
            }
            None => {
                let nid = Uuid::now_v7();
                let dir = self
                    .root
                    .join("uploads")
                    .join(nid.to_string())
                    .join(".codegraph");
                sqlx::query_as::<_, CgProjectDto>(
                    "INSERT INTO cg_projects (id, name, path, source_uri, status, source_kind) \
                     VALUES ($1, $2, $3, $4, 'ready', 'upload') RETURNING *",
                )
                .bind(nid)
                .bind(name)
                .bind(
                    dir.parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default()
                        .as_str(),
                )
                .bind(format!("upload://{head}"))
                .fetch_one(&self.pool)
                .await?;
                nid
            }
        };

        // 原子落盘：temp + rename（半写的 db 不该被查询看到）
        let dir = self
            .root
            .join("uploads")
            .join(id.to_string())
            .join(".codegraph");
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| CgError::Storage(e.to_string()))?;
        let tmp = dir.join("codegraph.db.tmp");
        let dst = dir.join("codegraph.db");
        tokio::fs::write(&tmp, db_bytes)
            .await
            .map_err(|e| CgError::Storage(e.to_string()))?;
        tokio::fs::rename(&tmp, &dst)
            .await
            .map_err(|e| CgError::Storage(e.to_string()))?;

        // 声明式新鲜度：head + uploaded_at，status 直接 ready（产物确在盘）
        sqlx::query(
            "UPDATE cg_projects SET head = $2, uploaded_at = now(), status = 'ready', \
             error = NULL, last_synced_at = now(), updated_at = now() WHERE id = $1",
        )
        .bind(id)
        .bind(head)
        .execute(&self.pool)
        .await?;
        self.get(id).await
    }

    /// 索引新鲜度：**优先比快照戳**（`snapshot_head` vs HEAD——「图基于哪个 commit」对
    /// 「代码在哪个 commit」，直接判据），无快照戳时才退回「HEAD 提交时间 vs last_indexed」。
    /// 只读 .git（git log），不新增后台扫描。HEAD 不可读（非 git 仓库/权限）→ 只报快照戳 +
    /// stale=null，不冒充最新。
    pub async fn freshness_for(&self, proj: &CgProjectDto) -> serde_json::Value {
        let last_indexed = proj
            .stats
            .as_ref()
            .and_then(|s| s.get("last_indexed").cloned())
            .unwrap_or(serde_json::Value::Null);

        // HEAD：git log -1 --format=%H %cI（容器 runtime 已装 git）
        let head = tokio::process::Command::new("git")
            .args(["log", "-1", "--format=%H %cI"])
            .current_dir(Path::new(&proj.path))
            .output()
            .await
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        let Some(head_line) = head else {
            // HEAD 不可读（非 git 仓库/bind-mount 摘走/跨机器）：退回 stats 里持久化的
            // 构建时快照戳（EN-26「未标注的过期才是 bug」）——诚实标注「图是哪个 commit 的」，
            // 不冒充最新。
            let snapshot_head = proj
                .stats
                .as_ref()
                .and_then(|s| s.get("head"))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            return serde_json::json!({
                "snapshot_head": snapshot_head,
                "built_at": last_indexed,
                "stale": serde_json::Value::Null,
                "hint": "当前 repo .git 不可读，无法校验新鲜度——上方 snapshot_head 是索引构建时的 commit；结构对不上请先 codegraph sync（有 repo 侧）或重新 index",
            });
        };
        let mut parts = head_line.split_whitespace();
        let hash = parts.next().unwrap_or("").to_string();
        let head_time = parts
            .next()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&chrono::Utc));
        let Some(head_time) = head_time else {
            return serde_json::json!({
                "head": hash, "last_indexed": last_indexed, "stale": serde_json::Value::Null,
                "hint": "HEAD 时间解析失败——新鲜度未知",
            });
        };

        // last_indexed 解析（CLI 格式容错）——**降级为非致命**：它只是「无快照戳时的兜底判据」，
        // 解析不出来不再早退（有快照戳时本来就轮不到它说话）。
        let indexed_at = last_indexed.as_str().and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|t| t.with_timezone(&chrono::Utc))
        });
        // 快照戳：索引构建时刻的 HEAD（stats.head，index/sync 时写入）。
        let snapshot_head = proj
            .stats
            .as_ref()
            .and_then(|s| s.get("head"))
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        // 陈旧判据（与《文档工作流》既有口径对齐）：**优先比快照戳**——「图基于哪个 commit」
        // vs「代码在哪个 commit」是直接判据。时间戳只是间接代理：CLI 的 lastIndexed 指「上次
        // 全量 index」，**sync 不更新它**（sync 只改索引内容 + last_synced_at + 快照戳），
        // 拿它判必然假阳性——sync 完仍报 stale，hint 又叫用户再 sync（死循环）。故仅兜底。
        let stale = match snapshot_head.as_str() {
            Some(s) if !s.is_empty() => serde_json::json!(s != hash),
            _ => match indexed_at {
                Some(t) => serde_json::json!(head_time > t),
                None => serde_json::Value::Null, // 两样判据都缺 → 未知，不虚报
            },
        };
        let mut v = serde_json::json!({
            "head": hash,
            "snapshot_head": snapshot_head,
            "head_committed_at": head_time.to_rfc3339(),
            "last_indexed": indexed_at.map(|t| t.to_rfc3339()),
            "stale": stale,
        });
        if stale == serde_json::json!(true) {
            v["hint"] = serde_json::json!(
                "索引落后于代码——图基于 snapshot_head、代码在 head，建议先 codegraph sync"
            );
        } else if stale.is_null() {
            v["hint"] = serde_json::json!(
                "快照戳与 last_indexed 都缺——无法校验新鲜度（重新 index 可补上快照戳）"
            );
        }
        v
    }

    pub async fn list(&self) -> Result<Vec<CgProjectDto>, CgError> {
        let mut rows =
            sqlx::query_as::<_, CgProjectDto>("SELECT * FROM cg_projects ORDER BY created_at DESC")
                .fetch_all(&self.pool)
                .await?;
        // usable 是派生字段（EN-48）：库里没有它的列，每次读取按磁盘事实现算。
        for r in &mut rows {
            r.usable = index_usable(r);
        }
        Ok(rows)
    }

    pub async fn get(&self, id: Uuid) -> Result<CgProjectDto, CgError> {
        let mut row = sqlx::query_as::<_, CgProjectDto>("SELECT * FROM cg_projects WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| CgError::NotFound(format!("项目 {id} 不存在")))?;
        row.usable = index_usable(&row);
        Ok(row)
    }

    /// 读索引入口的**唯一门禁**（EN-48）：query / full_graph / graph 都过这里，
    /// 按病因给不同且可行动的错误——不再是「同一故障三个入口三种表现」。
    ///
    /// 判定顺序即病因优先级：版本 → 状态 → 路径 → 产物。返回索引库路径（已确认在盘）。
    fn ensure_ready(&self, proj: &CgProjectDto) -> Result<PathBuf, CgError> {
        if proj.status == "version_mismatch" {
            return Err(CgError::VersionMismatch {
                need: CG_VERSION_PIN.into(),
                got: "unknown".into(),
            });
        }
        if proj.status != "ready" {
            let hint = match proj.status.as_str() {
                "registered" => "从未建过索引——先执行 codegraph index",
                "indexing" => "索引仍在进行中——稍后 codegraph list 确认 ready",
                "error" => "上次索引失败——看 codegraph list 的 error 字段，修好后重新 index",
                _ => "先执行 codegraph index",
            };
            return Err(CgError::BadRequest(format!(
                "项目未就绪（{}）——{hint}",
                proj.status
            )));
        }
        // ① 路径：目录被删/搬走，或条目来自容器形态（宿主看不到该路径）
        let path = Path::new(&proj.path);
        if !path.exists() {
            return Err(CgError::NotFound(format!(
                "项目路径不存在（{}）——目录可能已被删除或移动（容器形态下注册的路径在宿主上不可见）；\
                 请重新注册，或删除该条目",
                proj.path
            )));
        }
        // ② 产物：status=ready 只是「历史上成功过」，`.codegraph/` 一旦随目录消失不会自己回来
        let db_path = index_db_path(path);
        if !db_path.is_file() {
            return Err(CgError::NotFound(format!(
                "索引产物已丢失（{}）——`.codegraph/` 是 codegraph CLI 的未跟踪产物，\
                 目录被重新 clone / 清理后不会自己回来（注册状态仍是 ready，因为那只是历史记录）；\
                 请重新执行 codegraph index",
                db_path.display()
            )));
        }
        Ok(db_path)
    }

    /// 失效条目对账（EN-48）：扫描全部项目，把「自称 ready 但产物已丢失 / 路径已不存在」的
    /// 条目落到 error（附具体病因），使 list 不再把幽灵条目冒充可用资产。
    ///
    /// 只改状态、不动登记与源码——重新 index 即可恢复 ready，可逆。
    ///
    /// 自愈（EN-48 残留）：返回体 `needs_rebuild` 列出「路径仍在、仅产物丢失」的条目——
    /// 调用方（HTTP/MCP 层）对它们自动入队重建 job；「路径不存在」的幽灵无法自愈，
    /// 只标 error 等人重新注册或删除。
    pub async fn gc(&self) -> Result<serde_json::Value, CgError> {
        let rows = self.list().await?;
        let mut marked = Vec::new();
        let mut needs_rebuild = Vec::new();
        for proj in rows.iter().filter(|p| p.status == "ready" && !p.usable) {
            let (reason, healable) = if !Path::new(&proj.path).exists() {
                (
                    format!(
                        "项目路径不存在（{}）——目录已删除或移动（容器形态注册的路径在宿主上不可见）；\
                         请重新注册或删除该条目",
                        proj.path
                    ),
                    false,
                )
            } else if proj.source_kind == "upload" {
                (
                    format!(
                        "上传产物已丢失（{}）——upload 型条目服务端无源码，无法本机重建；\
                         请客户端本机 codegraph index 后重新 upload",
                        index_db_path(Path::new(&proj.path)).display()
                    ),
                    false,
                )
            } else {
                (
                    format!(
                        "索引产物已丢失（{}）——请重新执行 codegraph index",
                        index_db_path(Path::new(&proj.path)).display()
                    ),
                    true,
                )
            };
            sqlx::query(
                "UPDATE cg_projects SET status = 'error', error = $2, updated_at = now() \
                 WHERE id = $1",
            )
            .bind(proj.id)
            .bind(&reason)
            .execute(&self.pool)
            .await?;
            marked.push(serde_json::json!({
                "name": proj.name, "path": proj.path, "reason": reason,
            }));
            if healable {
                needs_rebuild.push(serde_json::json!({
                    "id": proj.id, "name": proj.name, "path": proj.path,
                }));
            }
        }
        Ok(serde_json::json!({
            "scanned": rows.len(),
            "marked_invalid": marked.len(),
            "items": marked,
            "needs_rebuild": needs_rebuild,
            "note": "置为 error 的条目：needs_rebuild 里的（路径仍在、仅产物丢失）已由服务端自动入队重建；\
                     路径不存在的幽灵条目需人工重新注册或删除",
        }))
    }

    /// 建索引（registered → indexing → ready/error）。超时 10min。
    pub async fn index(&self, id: Uuid) -> Result<CgProjectDto, CgError> {
        let proj = self.get(id).await?;
        self.ensure_version().await?;

        self.set_status(id, "indexing", None, None).await?;
        // 命令选择按「索引产物 db 是否存在」（EN-48 残留修复）：db 在 → index（增量）；
        // db 不在 → init（重建）。此前用 `.codegraph/` 目录存在性判断，但「目录在、db 丢」
        // （手动清理/备份不完整）时 CLI 的 index 会报 CodeGraph not initialized 而失败——
        // 自愈路径（gc 自动重建）恰恰专治产物丢失，必须用产物本体做判据。
        let cmd: &str = if index_db_path(Path::new(&proj.path)).exists() {
            "index"
        } else {
            "init"
        };
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
    /// 另附 `head`：index/sync 完成时刻的 HEAD commit hash（版本快照戳，EN-26）——
    /// 读得到 .git 才写；此后即使 repo 摘走，快照语义（「图是哪个 commit 的」）仍在 stats 里。
    async fn read_stats(&self, path: &Path) -> Option<serde_json::Value> {
        let out = run_cli(&["status", "--json"], Some(path), TIMEOUT_QUERY)
            .await
            .ok()?;
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        // 快照戳：构建时刻的 HEAD（读不到 = 非 git 仓库，留 null 不阻塞统计）
        let head = tokio::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(path)
            .output()
            .await
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        Some(serde_json::json!({
            "files": v.get("fileCount")?,
            "symbols": v.get("nodeCount")?,
            "edges": v.get("edgeCount")?,
            "by_kind": v.get("nodesByKind").cloned().unwrap_or(serde_json::json!({})),
            "last_indexed": v.get("lastIndexed").cloned().unwrap_or(serde_json::Value::Null),
            "head": head,
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
        // 正常路径下调用方已过 ensure_ready（产物必在盘，EN-48）；这里的检查只兜
        // 「校验后到读之间产物被删」的 TOCTOU——仍报同一类错，不静默降级成「假装没数据」。
        let db_path = index_db_path(path);
        if !db_path.is_file() {
            return Err(CgError::NotFound(format!(
                "索引产物已丢失（{}）——请重新执行 codegraph index",
                db_path.display()
            )));
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
        self.ensure_ready(&proj)?;
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
            let _ = tokio::fs::remove_dir_all(workdir).await; // 有意忽略：工作目录清理 best-effort
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
        let db_path = self.ensure_ready(&proj)?;
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
        self.ensure_ready(&proj)?;
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

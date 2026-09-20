//! `bridge` 的实现切片（架构治理 2026-09-21：自 bridge.rs 纯搬移，零行为变化）。

use super::*;

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
    pub(crate) name: String,
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) filePath: String,
    #[serde(default)]
    pub(crate) startLine: i64,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct CallersShape {
    #[serde(default)]
    pub(crate) callers: Vec<CgSymbolRef>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct CalleesShape {
    #[serde(default)]
    pub(crate) callees: Vec<CgSymbolRef>,
}

/// 索引产物路径（`<仓库根>/.codegraph/codegraph.db`）——**唯一出处**（EN-48）。
///
/// 注意 CLI 的「家」是**仓库根**而非 cwd：在仓库子目录里跑 `init`/`index`，它把
/// `.gitignore` 留在 cwd、数据库写到 git 根去（2026-09-15 实测：在 `server/` 下索引，
/// db 落在 `engram/.codegraph/codegraph.db`）。所以注册路径若是仓库内子目录，产物在父级。
/// 查找顺序：注册路径自己 → 沿父目录向上，走到含 `.git` 的那一级为止（不越界到仓库外）。
pub(crate) fn index_db_path(proj_path: &Path) -> PathBuf {
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

pub(crate) fn truncate(s: &str, max: usize) -> String {
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
pub(crate) fn classify(e: CgError) -> engram_jobs::types::JobError {
    use engram_jobs::types::JobError;
    match e {
        CgError::Timeout(_, _) | CgError::CliUnavailable(_) => JobError::Retryable(e.to_string()),
        CgError::Storage(m) => JobError::Retryable(format!("存储暂时不可用: {m}")),
        other => JobError::Permanent(other.to_string()),
    }
}

/// CLI 执行原语：spawn + 超时 + 错误归一（测试通过 `#[cfg(test)]` 注入模拟）。
pub(crate) type CliOutput = std::process::Output;

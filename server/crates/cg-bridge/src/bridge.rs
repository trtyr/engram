//! CLI 子进程桥 + 项目生命周期 + 查询代理。

mod dest;
mod index;
mod model;
mod precompute;
mod query;
mod version;
pub(crate) use dest::*;
pub use index::*;
pub use model::*;
// attach_positions（初布局坐标注入）在 query.rs 里以 `use super::*` 取用，故此处再导出
pub(crate) use precompute::attach_positions;

use std::path::{Path, PathBuf};
use std::time::Duration;

use uuid::Uuid;

/// pin 的 codegraph 版本（D0005：上游 breaking change 防护）。
pub const CG_VERSION_PIN: &str = "1.5.0";

/// CLI 修复提示（可照抄：装 + 锁版）——服务端错误文案与前端状态条共用同一事实源。
/// 出处：《codegraph 上云 · README》§上游与安装（2026-09-21 核实）。
pub fn cli_fix_hint(pin: &str) -> String {
    format!(
        "CLI 修复（可照抄）：装并锁到 {pin} —— \
         `curl -fsSL https://raw.githubusercontent.com/colbymchenry/codegraph/main/install.sh | sh` \
         或 `npm i -g @colbymchenry/codegraph@{pin}`；已装但版本不符 → \
         `codegraph upgrade {pin}`（升级后产物 schema 会变：需重新 `codegraph index` 并重传 upload 型条目）"
    )
}

/// 本服务在 pin 版本（1.5.0）下**实测**的 extraction 版本
/// （`project_metadata.indexed_with_extraction_version`，实测 24）。
///
/// **不是硬门**：CLI 版本不变、上游抽取逻辑也会升级（同一个 db 的图语义随之变化），而服务端无法
/// 稳定持有「期望值」（服务端 CLI 未同步升级时会误拒）——故只做**告警**（写
/// `stats.extraction_warning`），不阻断入库（R3 已定口径，2026-09-21）。
pub const CG_EXTRACTION_VERSION_SEEN: &str = "24";

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
mod tests;

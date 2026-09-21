//! 代码图谱入口收敛（2026-09-21）：注册落盘路径的解析与校验。
//!
//! 两条入口的落盘规则（用户拍定，见《代码图谱入口收敛 · README》）：
//! - **默认**：`<codegraph 根>/<项目名>/`（重名自动加 `-2`/`-3`）——服务端自建，删条目连目录清；
//! - **自定义**：`<父目录>/<仓库名>/`——用户指定，删条目**只删注册与产物、目录保留**；
//!   父目录必须在白名单根之内（默认 = 数据根；用 `AGENT_MEMORY_CG_DEST_ROOTS` 放开）。
//!
//! 本模块只做**纯函数**（路径推导/净化/校验），副作用（建目录、clone、落库）留在 `index.rs`。

use super::*;

/// 自定义落盘白名单根的环境变量（逗号分隔的绝对路径）。
/// 未设或为空 → 默认只允许「数据根」（= codegraph 根的上层那一级）。
pub const DEST_ROOTS_ENV: &str = "AGENT_MEMORY_CG_DEST_ROOTS";

/// clone 超时（秒）：网络挂起不许把 HTTP 请求一起挂死。
pub(crate) const CLONE_TIMEOUT_SECS: u64 = 120;

/// 是否「git 仓库地址」形态——URI-only 入口的判据。
///
/// 接受：`http(s)://`、`ssh://`、`git://`、`file://`、scp-like（`git@host:path`）、`.git` 结尾。
/// 拒绝：任何普通文件系统路径（含 Windows 盘符 `D:\...`）——文案指引改走「上传产物」入口。
pub(crate) fn looks_like_git_uri(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    if ["http://", "https://", "ssh://", "git://", "file://"]
        .iter()
        .any(|p| t.starts_with(p))
    {
        return true;
    }
    // scp-like：`user@host:path`——`@` 前不含路径分隔符（否则像本地相对路径）
    if let Some(at) = t.find('@')
        && !t[..at].contains(['/', '\\'])
        && t[at + 1..].contains(':')
    {
        return true;
    }
    t.ends_with(".git")
}

/// 从仓库地址推导目录名：去查询串/锚点/尾斜杠/`.git`，取最后一段（scp-like 取 `:` 之后）。
/// 返回 `None` = 推不出可用名字（调用方回落项目名）。
pub(crate) fn repo_name_from_uri(uri: &str) -> Option<String> {
    let t = uri.trim().trim_end_matches('/');
    let t = t.split(['?', '#']).next().unwrap_or(t);
    let t = t.trim_end_matches(".git");
    let last = t.rsplit(['/', ':']).next().unwrap_or("");
    let cleaned = sanitize_dir_name(last);
    (!cleaned.is_empty()).then_some(cleaned)
}

/// 目录名净化：只保留 ASCII 字母数字与 `.` `_` `-`（其余换 `-`），并把首尾的 `-`/`.` 剪掉。
/// 中文/空格项目名 → 空串（调用方回落 `project`），避免奇怪路径。
pub(crate) fn sanitize_dir_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches(['-', '.'])
        .to_string()
}

/// 允许的落盘根（已 canonicalize；解析不出的丢弃）。
///
/// env `AGENT_MEMORY_CG_DEST_ROOTS` 优先（逗号分隔）；未设/为空 → 默认「数据根」。
pub(crate) fn allowed_dest_roots(codegraph_root: &Path) -> Vec<PathBuf> {
    let from_env: Vec<PathBuf> = std::env::var(DEST_ROOTS_ENV)
        .ok()
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default();
    let roots = if from_env.is_empty() {
        codegraph_root
            .parent()
            .map(|p| vec![p.to_path_buf()])
            .unwrap_or_default()
    } else {
        from_env
    };
    roots.iter().filter_map(|r| r.canonicalize().ok()).collect()
}

/// 默认落盘：`<codegraph 根>/<净化项目名>`；已占用则依次试 `-2`/`-3`…（上限 50，再不行加 uuid）。
pub(crate) fn default_dest(codegraph_root: &Path, name: &str) -> PathBuf {
    let base = {
        let s = sanitize_dir_name(name);
        if s.is_empty() {
            "project".to_string()
        } else {
            s
        }
    };
    let first = codegraph_root.join(&base);
    if !first.exists() {
        return first;
    }
    for n in 2..=50 {
        let cand = codegraph_root.join(format!("{base}-{n}"));
        if !cand.exists() {
            return cand;
        }
    }
    codegraph_root.join(format!("{base}-{}", Uuid::now_v7()))
}

/// 自定义落盘校验：父目录必须**绝对**且存在 → 落在白名单根内 → 目标目录不存在或为空。
/// 返回目标绝对路径（`<父目录>/<仓库名>`）。
pub(crate) fn custom_dest(
    codegraph_root: &Path,
    parent: &str,
    repo_name: &str,
) -> Result<PathBuf, CgError> {
    let p = Path::new(parent.trim());
    if !p.is_absolute() {
        return Err(CgError::BadRequest(format!(
            "自定义落盘路径要写**绝对路径**（服务端视角）：收到 {parent:?}——例如 /Users/you/Code"
        )));
    }
    let parent_canon = p.canonicalize().map_err(|_| {
        CgError::BadRequest(format!(
            "自定义落盘路径的父目录不存在或不可达：{}——请先在服务端机器上创建它（注意是**服务端**的文件系统）",
            p.display()
        ))
    })?;
    let roots = allowed_dest_roots(codegraph_root);
    if !roots.iter().any(|r| parent_canon.starts_with(r)) {
        let list = roots
            .iter()
            .map(|r| r.display().to_string())
            .collect::<Vec<_>>()
            .join("、");
        return Err(CgError::BadRequest(format!(
            "自定义落盘路径越界：{} 不在允许的根之内（当前允许：{list}）——\
             需要写到别处请给服务端设 {DEST_ROOTS_ENV}（逗号分隔的绝对路径）",
            parent_canon.display()
        )));
    }
    let target = parent_canon.join(repo_name);
    if target.exists() {
        let empty = std::fs::read_dir(&target)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if !empty {
            return Err(CgError::BadRequest(format!(
                "目标目录已存在且非空：{}——服务端不覆盖已有目录；请换父目录、改名或先清空",
                target.display()
            )));
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_uri_forms_accepted_and_paths_rejected() {
        for ok in [
            "https://github.com/you/repo",
            "https://github.com/you/repo.git",
            "http://gitlab.local/group/proj.git",
            "ssh://git@host:2222/x.git",
            "git@github.com:you/repo.git",
            "file:///tmp/x",
        ] {
            assert!(looks_like_git_uri(ok), "应接受：{ok}");
        }
        for bad in [
            "/Users/you/Code/engram",
            "./relative/repo",
            "D:\\Code\\Rust\\engram",
            "",
            "   ",
        ] {
            assert!(!looks_like_git_uri(bad), "应拒绝：{bad:?}");
        }
    }

    #[test]
    fn repo_name_derived_from_uri() {
        assert_eq!(
            repo_name_from_uri("https://github.com/you/repo.git").as_deref(),
            Some("repo")
        );
        assert_eq!(
            repo_name_from_uri("https://github.com/you/repo/").as_deref(),
            Some("repo")
        );
        assert_eq!(
            repo_name_from_uri("git@github.com:you/my_repo.git").as_deref(),
            Some("my_repo")
        );
        assert_eq!(
            repo_name_from_uri("https://h/x/y?tab=readme").as_deref(),
            Some("y")
        );
        // 中文仓库名净化后为空 → None（调用方回落项目名）
        assert_eq!(repo_name_from_uri("https://h/x/中文"), None);
    }

    #[test]
    fn sanitize_keeps_safe_chars_only() {
        assert_eq!(sanitize_dir_name("engram-server"), "engram-server");
        assert_eq!(sanitize_dir_name("my repo!"), "my-repo");
        assert_eq!(sanitize_dir_name("中文名"), "");
        assert_eq!(sanitize_dir_name("--x--"), "x");
    }

    #[test]
    fn default_dest_dedups_with_suffix() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("codegraph");
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(default_dest(&root, "demo"), root.join("demo"));
        std::fs::create_dir_all(root.join("demo")).unwrap();
        assert_eq!(default_dest(&root, "demo"), root.join("demo-2"));
        std::fs::create_dir_all(root.join("demo-2")).unwrap();
        assert_eq!(default_dest(&root, "demo"), root.join("demo-3"));
        // 中文项目名 → 回落 project
        assert_eq!(default_dest(&root, "中文"), root.join("project"));
    }

    #[test]
    fn custom_dest_rejects_relative_missing_and_occupied() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("codegraph");
        std::fs::create_dir_all(&root).unwrap();
        let parent = tmp.path().join("repos");
        std::fs::create_dir_all(&parent).unwrap();

        // 相对路径拒绝
        assert!(custom_dest(&root, "repos", "r").is_err(), "相对路径应拒");
        // 父目录不存在拒绝
        let missing = tmp.path().join("nope");
        assert!(
            custom_dest(&root, missing.to_string_lossy().as_ref(), "r").is_err(),
            "父目录缺失应拒"
        );
        // 目标非空拒绝
        let occupied = parent.join("r");
        std::fs::create_dir_all(&occupied).unwrap();
        std::fs::write(occupied.join("keep.txt"), b"x").unwrap();
        let err = custom_dest(&root, parent.to_string_lossy().as_ref(), "r").unwrap_err();
        assert!(err.to_string().contains("非空"), "{err}");
        // 空目录放行
        std::fs::remove_file(occupied.join("keep.txt")).unwrap();
        let ok = custom_dest(&root, parent.to_string_lossy().as_ref(), "r").unwrap();
        assert_eq!(ok, parent.canonicalize().unwrap().join("r"));
    }

    #[test]
    fn allowed_roots_default_is_data_root_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("app").join("codegraph");
        std::fs::create_dir_all(&root).unwrap();
        let roots = allowed_dest_roots(&root);
        assert_eq!(roots.len(), 1, "{roots:?}");
        assert_eq!(roots[0], tmp.path().join("app").canonicalize().unwrap());
    }
}

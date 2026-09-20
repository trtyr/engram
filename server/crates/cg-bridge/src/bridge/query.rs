//! `bridge` 的实现切片（架构治理 2026-09-21：自 bridge.rs 纯搬移，零行为变化）。

use super::*;

impl CgBridge {
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

    /// explore 符号大纲（R 报告 P0-3）：直接读索引库（.codegraph/codegraph.db，
    /// 只读——与 full_graph 同哲学），按「路径/文件名含 target 或符号名含 target」取
    /// 符号清单（name/kind/行号/签名截断），不返回任何源码正文。
    /// 返回 None = 大纲数据源不可用或零命中（调用方落回 CLI explore）。
    pub(crate) async fn explore_outline(
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
        let job =
            tokio::task::spawn_blocking(move || -> Result<Option<serde_json::Value>, CgError> {
                explore_outline_query(&db_str, &target)
            });
        job.await
            .map_err(|e| CgError::Parse(format!("索引读取线程崩溃: {e}")))?
    }

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
        let (args, timeout) = build_query_args(kind, target, depth);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = run_cli(&arg_refs, Some(path), timeout).await?;
        let stdout = String::from_utf8_lossy(&out.stdout);

        Self::format_query_result(kind, &stdout)
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

    /// 文件级全图：把全部跨文件依赖边按文件聚合（imports/calls/instantiates/references，
    /// 排除 contains 与自环）。这是「整个项目的调用图」的正确粒度——符号级动辄几百节点不可读，
    /// 文件级通常几十个节点正好。数据源：.codegraph/codegraph.db（只读打开，schema 随 pin 稳定）。
    pub async fn full_graph(&self, id: Uuid) -> Result<serde_json::Value, CgError> {
        let proj = self.get(id).await?;
        let db_path = self.ensure_ready(&proj)?;
        // 阻塞读小库（<10MB）：spawn_blocking 防占 worker
        let db_str = db_path.to_string_lossy().into_owned();
        let v = tokio::task::spawn_blocking(move || full_graph_query(&db_str))
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
    /// 结果格式化：explore/node 取截断文本，其余按 JSON 归一。
    fn format_query_result(kind: QueryKind, stdout: &str) -> Result<serde_json::Value, CgError> {
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
}

/// explore 大纲查询（阻塞段）：LIKE 通配转义 + 200 条上限，产出文件清单与符号大纲。
fn explore_outline_query(db_str: &str, target: &str) -> Result<Option<serde_json::Value>, CgError> {
    let conn =
        rusqlite::Connection::open_with_flags(&db_str, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
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
    Ok(outline_json(&rows, target)?)
}

/// 文件级依赖图（阻塞段）：按 (源文件,目标文件) 聚合 import 边，产出 nodes/edges。
fn full_graph_query(db_str: &str) -> Result<serde_json::Value, CgError> {
    let conn =
        rusqlite::Connection::open_with_flags(&db_str, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
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
}

/// explore 大纲的 JSON 组装（符号截断 + 文件清单去重保序 + 200 上限提示）。
fn outline_json(
    rows: &[(String, String, String, i64, i64, Option<String>)],
    target: &str,
) -> Result<Option<serde_json::Value>, CgError> {
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
    for (_, _, fp, _, _, _) in rows {
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
}

/// 按查询类型构造 CLI 参数与超时（explore 无 --json；impact 支持 depth）。
fn build_query_args(kind: QueryKind, target: &str, depth: Option<u32>) -> (Vec<String>, Duration) {
    match kind {
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
    }
}

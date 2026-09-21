//! `bridge` 的实现切片：**索引收尾时的初布局**（架构治理 2026-09-21 的分片约定：一片一组操作）。
//!
//! 口径（goal 契约 t3）：服务端在 index/sync 收尾时算文件级初布局、落
//! `<仓库>/.codegraph/layout.json`，让大库前端**首帧就位**（客户端只做 alpha=0.3 微调）。
//! **布局失败绝不影响索引成功**——只在 `stats.layout_warning` 留一条可读原因。

use super::*;

/// 布局迭代上限（`AGENT_MEMORY_CG_LAYOUT_ITERS` 可覆盖；测试/低配机可调小）。
/// 默认 `DEFAULT_ITERATIONS`（300，对应 alpha 从 1 衰减到 0.001）。
fn layout_iterations() -> u32 {
    std::env::var("AGENT_MEMORY_CG_LAYOUT_ITERS")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(crate::layout::DEFAULT_ITERATIONS)
}

impl CgBridge {
    /// 算初布局并落盘。返回 `(stats.layout, stats.layout_warning)`——**两者至多一个有值**。
    ///
    /// 阻塞算力放 `spawn_blocking`（几千节点的 BH 布局是纯 CPU；别占着 async worker）。
    pub(crate) async fn refresh_layout(
        &self,
        proj_path: &Path,
    ) -> (Option<serde_json::Value>, Option<String>) {
        let db = index_db_path(proj_path);
        if !db.is_file() {
            return (None, Some("索引产物不在盘——跳过初布局".into()));
        }
        let iters = layout_iterations();
        let res =
            tokio::task::spawn_blocking(move || crate::layout::compute_and_write(&db, iters)).await;
        match res {
            Ok(Ok((f, path))) => (
                Some(serde_json::json!({
                    "version": f.version,
                    "algorithm": f.algorithm,
                    "nodes": f.nodes,
                    "edges": f.edges,
                    "iterations": f.iterations,
                    "path": path.to_string_lossy(),
                    "generated_at": f.generated_at,
                })),
                None,
            ),
            Ok(Err(e)) => (
                None,
                Some(format!(
                    "初布局计算失败（不影响索引结果，前端会自行收敛）：{e}"
                )),
            ),
            Err(e) => (None, Some(format!("初布局线程崩溃（不影响索引结果）：{e}"))),
        }
    }

    /// 把初布局结果并进 stats（`layout` 或 `layout_warning` 二选一），供 index/sync 收尾统一调用。
    pub(crate) async fn merge_layout_into_stats(
        &self,
        proj_path: &Path,
        stats: &mut Option<serde_json::Value>,
    ) {
        let (meta, warn) = self.refresh_layout(proj_path).await;
        let obj = stats.get_or_insert_with(|| serde_json::json!({}));
        let Some(map) = obj.as_object_mut() else {
            return;
        };
        match (meta, warn) {
            (Some(meta), _) => {
                map.remove("layout_warning");
                map.insert("layout".to_string(), meta);
            }
            (None, Some(w)) => {
                map.remove("layout");
                map.insert("layout_warning".to_string(), serde_json::json!(w));
            }
            (None, None) => {}
        }
    }
}

/// 把初布局坐标注入图响应节点（t4）：**有坐标才带 `x`/`y`，没有就不给字段**——
/// 前端据此区分「首帧就位」与「自行收敛」，绝不填假坐标。
///
/// 找键方式按图类型各就各位：文件级图 `node.id` 就是文件路径；符号级图走 `node.filePath`
/// （中心符号没有文件路径就落空，不影响其他节点）——于是「同一文件的符号」天然聚在文件位置上。
pub(crate) fn attach_positions(graph: &mut serde_json::Value, layout: &crate::layout::LayoutFile) {
    let Some(nodes) = graph.get_mut("nodes").and_then(|n| n.as_array_mut()) else {
        return;
    };
    for node in nodes {
        let Some(obj) = node.as_object_mut() else {
            continue;
        };
        let key = obj
            .get("filePath")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| obj.get("id").and_then(|v| v.as_str()));
        let Some(pos) = key.and_then(|k| layout.positions.get(k)) else {
            continue;
        };
        obj.insert("x".to_string(), serde_json::json!(pos[0]));
        obj.insert("y".to_string(), serde_json::json!(pos[1]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPool;

    /// 惰性池：这些用例只碰磁盘（不碰库），与 `limit_tests` 同法。
    fn bridge(root: &Path) -> CgBridge {
        CgBridge::new(
            PgPool::connect_lazy("postgres://nobody@127.0.0.1:1/none").unwrap(),
            root,
        )
    }

    /// t3 契约：**布局失败不影响索引**——只在 stats 留 `layout_warning`，原有字段照旧保留。
    #[tokio::test]
    async fn layout_failure_only_writes_warning() {
        let missing = Path::new("/tmp/cg-layout-probe-does-not-exist");
        let b = bridge(missing);
        let mut stats = Some(serde_json::json!({"files": 12, "edges": 34}));
        b.merge_layout_into_stats(missing, &mut stats).await;

        let s = stats.expect("stats 不该被清空");
        assert!(s.get("layout").is_none(), "失败时不该有 layout: {s}");
        let warn = s["layout_warning"].as_str().expect("应留一条可读 warning");
        assert!(
            warn.contains("索引产物不在盘"),
            "warning 应说清病因: {warn}"
        );
        assert_eq!(s["files"], 12, "原有统计字段必须保留");
        assert_eq!(s["edges"], 34);
    }

    /// 成功路径：算好落盘 → stats 带 `layout` 元信息，且抹掉上次遗留的 `layout_warning`。
    #[tokio::test]
    async fn layout_success_writes_meta_and_clears_warning() {
        let dir = tempfile::tempdir().unwrap();
        let cgdir = dir.path().join(".codegraph");
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

        let b = bridge(dir.path());
        let mut stats = Some(serde_json::json!({"files": 2, "layout_warning": "上次失败"}));
        b.merge_layout_into_stats(dir.path(), &mut stats).await;

        let s = stats.expect("stats 不该被清空");
        assert!(
            s.get("layout_warning").is_none(),
            "成功后应抹掉旧 warning: {s}"
        );
        assert_eq!(s["layout"]["nodes"], 2, "布局元信息应落 stats: {s}");
        assert_eq!(s["layout"]["algorithm"], crate::layout::LAYOUT_ALGORITHM);
        assert!(cgdir.join("layout.json").is_file(), "layout.json 应落盘");
    }

    /// 坐标注入：文件级按 node.id、符号级按 node.filePath；找不到就**不给字段**（不填假坐标）。
    #[test]
    fn attach_positions_matches_both_graph_shapes() {
        let mut positions = std::collections::HashMap::new();
        positions.insert("a.rs".to_string(), [1.0, 2.0]);
        positions.insert("b.rs".to_string(), [3.0, 4.0]);
        let l = crate::layout::LayoutFile {
            version: crate::layout::LAYOUT_VERSION,
            algorithm: crate::layout::LAYOUT_ALGORITHM.to_string(),
            iterations: 1,
            nodes: 2,
            edges: 1,
            generated_at: chrono::Utc::now(),
            positions,
        };
        let mut files_graph = serde_json::json!({
            "mode": "files",
            "nodes": [{"id": "a.rs"}, {"id": "unknown.rs"}],
            "edges": [],
        });
        attach_positions(&mut files_graph, &l);
        assert_eq!(files_graph["nodes"][0]["x"], 1.0);
        assert!(
            files_graph["nodes"][1].get("x").is_none(),
            "没坐标就不给字段"
        );

        let mut symbol_graph = serde_json::json!({
            "symbol": "load",
            "nodes": [{"id": "load@p.rs:1", "filePath": "b.rs"}, {"id": "load@x.rs"}],
            "edges": [],
        });
        attach_positions(&mut symbol_graph, &l);
        assert_eq!(
            symbol_graph["nodes"][0]["x"], 3.0,
            "符号级按 filePath 沾文件坐标"
        );
        assert!(symbol_graph["nodes"][1].get("x").is_none());
    }
}

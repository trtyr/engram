use super::*;

fn ids(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("src/f{i}.rs")).collect()
}

/// 小图 N 轮收敛：无 NaN、坐标有限、确实散开（不是全挤在原点）。
#[test]
fn small_graph_converges_without_nan() {
    let ids = ids(30);
    let edges: Vec<(usize, usize, f64)> = (0..29).map(|i| (i, i + 1, 1.0)).collect();
    let pos = run_layout(&ids, &edges, DEFAULT_ITERATIONS);
    assert_eq!(pos.len(), 30);
    for (i, p) in pos.iter().enumerate() {
        assert!(
            p[0].is_finite() && p[1].is_finite(),
            "第 {i} 个坐标非法: {p:?}"
        );
    }
    let span = pos
        .iter()
        .map(|p| p[0].abs().max(p[1].abs()))
        .fold(0.0f64, f64::max);
    assert!(span > 10.0, "布局没有散开（最大坐标 {span}）");
}

/// 可复现：同一输入两次跑出完全相同的坐标（带种子的抖动 + 确定力律）。
#[test]
fn layout_is_reproducible() {
    let ids = ids(40);
    let edges: Vec<(usize, usize, f64)> = (0..39).map(|i| (i, i + 1, 1.0)).collect();
    let a = run_layout(&ids, &edges, 60);
    let b = run_layout(&ids, &edges, 60);
    assert_eq!(a, b, "同输入两次布局应完全一致");
}

/// 零边图不 panic（全孤立点：只有向心+斥力）。
#[test]
fn graph_without_edges_is_safe() {
    let pos = run_layout(&ids(5), &[], 30);
    assert_eq!(pos.len(), 5);
    assert!(pos.iter().all(|p| p[0].is_finite() && p[1].is_finite()));
}

/// 四叉树在所有点坐标完全重合时也必须终止（深度上限兜底）。
#[test]
fn quadtree_survives_coincident_points() {
    let px = vec![3.0; 20];
    let py = vec![4.0; 20];
    let charge = vec![REPEL_STRENGTH; 20];
    let t = QuadTree::build(&px, &py, &charge);
    let mut rng = Lcg::new(7);
    let (dx, dy) = t.apply(0, 0, &mut rng);
    assert!(dx.is_finite() && dy.is_finite());
}

/// layout.json 读写往返：写入后可读回，且版本/节点数/坐标一致。
#[test]
fn layout_file_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("layout.json");
    let mut positions = HashMap::new();
    positions.insert("src/a.rs".to_string(), [1.5, -2.5]);
    positions.insert("src/b.rs".to_string(), [3.0, 4.0]);
    let f = LayoutFile {
        version: LAYOUT_VERSION,
        algorithm: LAYOUT_ALGORITHM.into(),
        iterations: 42,
        nodes: 2,
        edges: 1,
        generated_at: chrono::Utc::now(),
        positions,
    };
    std::fs::write(&p, serde_json::to_vec(&f).unwrap()).unwrap();
    let back = read_layout(&p).expect("能读回");
    assert_eq!(back.version, LAYOUT_VERSION);
    assert_eq!(back.nodes, 2);
    assert_eq!(back.positions["src/a.rs"], [1.5, -2.5]);
}

/// 缺文件 / 版本不认识 → None（调用方按「没有布局」处理，不报错）。
#[test]
fn read_layout_is_lenient() {
    let dir = tempfile::tempdir().unwrap();
    assert!(read_layout(&dir.path().join("nope.json")).is_none());
    let p = dir.path().join("layout.json");
    std::fs::write(&p, b"{\"version\":999,\"positions\":{}}").unwrap();
    assert!(read_layout(&p).is_none(), "版本不认识应视作没有布局");
}

/// 空图（零跨文件边）→ 明确 Err，不落盘。
#[test]
fn empty_graph_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("codegraph.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE files(path TEXT PRIMARY KEY); \
             CREATE TABLE nodes(id TEXT PRIMARY KEY, file_path TEXT); \
             CREATE TABLE edges(source TEXT, target TEXT, kind TEXT);",
    )
    .unwrap();
    drop(conn);
    let err = compute_and_write(&db, 10).expect_err("空图应报错");
    assert!(matches!(err, CgError::Parse(_)), "{err:?}");
    assert!(!layout_path(&db).exists(), "失败不该留下 layout.json");
}

/// 端到端：造一个 3 文件的小索引库 → 算出布局并落盘，节点数/坐标齐全。
#[test]
fn compute_and_write_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("codegraph.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE files(path TEXT PRIMARY KEY); \
             CREATE TABLE nodes(id TEXT PRIMARY KEY, file_path TEXT); \
             CREATE TABLE edges(source TEXT, target TEXT, kind TEXT); \
             INSERT INTO files(path) VALUES ('a.rs'), ('b.rs'), ('c.rs'); \
             INSERT INTO nodes(id, file_path) VALUES ('n1','a.rs'), ('n2','b.rs'), ('n3','c.rs'); \
             INSERT INTO edges(source, target, kind) VALUES ('n1','n2','calls'), \
                ('n2','n3','calls'), ('n1','n3','references'), ('n1','n1','contains');",
    )
    .unwrap();
    drop(conn);
    let (f, path) = compute_and_write(&db, 40).expect("应能算出布局");
    assert_eq!(f.nodes, 3, "contains 与自环被排除后剩 3 个文件");
    assert_eq!(f.edges, 3, "a→b、b→c、a→c 共 3 条");
    assert!(path.exists(), "layout.json 应落盘");
    assert_eq!(path, layout_path(&db));
    let back = read_layout(&path).unwrap();
    assert_eq!(back.positions.len(), 3);
    for p in back.positions.values() {
        assert!(p[0].is_finite() && p[1].is_finite());
    }
}

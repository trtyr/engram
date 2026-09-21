//! CodeGraph **文件级初布局**（服务端预计算，2026-09-21）
//!
//! ## 为什么要有它
//! 大库（safeline-2：3721 文件 / 2.8 万边）全在客户端从随机起点收敛，打开瞬间要等好几秒才成形；
//! 服务端在 index/sync 收尾时先把布局算出来落盘，前端首帧就位、只做微调（客户端 alpha=0.3 唤醒）。
//!
//! ## 选型 spike 结论（crate vs 自写 Verlet）
//! 候选：`forceatlas2 0.8.0` / `fa2 0.3.0`（crates.io 均在）、自写 Verlet 积分。
//! **结论：自写**，理由三条（按权重）：
//! 1. **语义必须与客户端一致**。前端物理跑的是 d3-force fork（`web/src/components/ForceGraph/sim.ts`），
//!    常数逐条对齐 Obsidian 实测值。服务端若用 FA2 的力律（degree 加权引力、scalingRatio 语义的斥力、
//!    LinLog 距离），预计算出来的是**另一个吸引子上的形状**——首帧就位后客户端会明显「跳一下」再收敛，
//!    等于把「打开即收敛」变成「打开先跳」。自写 d3 同款力律 → 初布局只是同一个吸引子上更早的状态。
//! 2. **常数能一一对上**：d3 的 alphaDecay/alphaMin/velocityDecay/linkDistance/linkStrength/charge
//!    可直接表达我们的实测常数；FA2 只有 scalingRatio/slowDown/gravity，表达不了 `linkDistance=250`
//!    与 `repel=−1000`。
//! 3. **依赖面**：自写约 300 行（四叉树 + 5 个力），零新依赖；两个候选 crate 都是单人小库，
//!    引入即多一层版本与许可证维护面。
//!
//! ## 与 d3-force 的**有意偏离**（照抄优先，这两处必须偏离并记录）
//! - **抖动用带种子的 LCG**（d3 用 `Math.random()`）：布局必须**可复现**（同库两次 index 出同一份
//!   layout.json，diff 得动），否则每次 index 都全量变坐标，前端「记住的位置」也失去意义。
//! - **forceCollide 用均匀网格而非四叉树**：碰撞半径 60 是固定的、只影响近邻，网格 O(n) 比四叉树更省；
//!   力律（线性重叠量 × 强度 × alpha，两侧对分）与 d3 一致。
//!
//! 其余（phyllotaxis 初值、`alpha += (0−alpha)·alphaDecay`、`v *= velocityDecay` 后再位移、
//! Barnes-Hut θ=0.9 + 电荷加权质心、link 的 bias=度数占比）都按 d3-force 3.x 逐条对齐。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::CgError;

// ── 常数：与前端共享引擎逐条对齐（web/src/components/ForceGraph/sim.ts 的 OBSIDIAN） ──

/// forceLink 默认连线长度（Obsidian 实测 250）
const LINK_DISTANCE: f64 = 250.0;
/// 连线拉力（Obsidian 滑杆默认位 1 过曲线 → 1.0）
const LINK_STRENGTH: f64 = 1.0;
/// 向心强度（Obsidian 中心滑杆默认 0.5187 过 curve(v,0.01) → 0.1）
const CENTER_STRENGTH: f64 = 0.1;
/// 斥力（Obsidian 斥力滑杆默认 10 → −(10³)）
const REPEL_STRENGTH: f64 = -1000.0;
/// 速度衰减（Obsidian 实测 0.6）
const VELOCITY_DECAY: f64 = 0.6;
/// forceCollide 半径 / 强度（Obsidian 实测 60 / 0.5）
const COLLIDE_RADIUS: f64 = 60.0;
const COLLIDE_STRENGTH: f64 = 0.5;
/// 停机阈值（Obsidian 实测 0.001）
const ALPHA_MIN: f64 = 0.001;
/// Barnes-Hut 近似阈值（d3 默认 0.9）
const THETA: f64 = 0.9;
/// 近邻力下限（防除零爆炸；Obsidian 实测 30）
const DISTANCE_MIN: f64 = 30.0;
/// 默认迭代上限（alpha 从 1 衰减到 0.001 约 300 tick）
pub const DEFAULT_ITERATIONS: u32 = 300;
/// layout.json 结构版本（前端读之前先看它，不认就忽略）
pub const LAYOUT_VERSION: u32 = 1;
/// 算法标识（前端/文档据此判断坐标语义）
pub const LAYOUT_ALGORITHM: &str = "d3-force-verlet";

/// `alphaDecay = 1 − 0.001^(1/300)`（Obsidian 实测：约 300 tick 收敛到 alphaMin）。
fn alpha_decay() -> f64 {
    1.0 - ALPHA_MIN.powf(1.0 / 300.0)
}

/// 带种子的线性同余发生器（仅用于「两点重合时的抖动」——见模块头「有意偏离」）。
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Lcg(seed | 1)
    }
    /// d3 的 `jiggle()`：±5e-7 量级的确定方向抖动（非零即够）。
    fn jiggle(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f64 / (1u64 << 31) as f64 - 0.5) * 1e-6
    }
}

/// 四叉树节点：叶子装点（`point >= 0`）或为空（`EMPTY`）；内部节点 `INTERNAL` + 四个子索引。
const EMPTY: i32 = -2;
const INTERNAL: i32 = -1;

#[derive(Clone, Copy)]
struct Quad {
    children: [i32; 4],
    /// 聚合电荷（本格内全部点的 strength 之和）
    value: f64,
    /// 电荷加权质心
    x: f64,
    y: f64,
    /// 本格边长
    size: f64,
    /// 叶子持有的点索引（EMPTY / INTERNAL / >=0）
    point: i32,
}

impl Default for Quad {
    fn default() -> Self {
        Quad {
            children: [EMPTY; 4],
            value: 0.0,
            x: 0.0,
            y: 0.0,
            size: 1.0,
            point: EMPTY,
        }
    }
}

/// Barnes-Hut 四叉树（d3-force manyBody 同款：θ 近似 + 电荷加权质心）。
struct QuadTree {
    quads: Vec<Quad>,
    /// 每格的「格内原点」（左下角）——分裂时按半宽推子格，供落点判格用
    origins: Vec<(f64, f64)>,
    px: Vec<f64>,
    py: Vec<f64>,
    charge: Vec<f64>,
}

impl QuadTree {
    fn build(px: &[f64], py: &[f64], charge: &[f64]) -> Self {
        let n = px.len();
        let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for i in 0..n {
            minx = minx.min(px[i]);
            miny = miny.min(py[i]);
            maxx = maxx.max(px[i]);
            maxy = maxy.max(py[i]);
        }
        let cx = (minx + maxx) / 2.0;
        let cy = (miny + maxy) / 2.0;
        // 正方形边域（外扩 1%，保证边界点严格落格内）
        let size = ((maxx - minx).max(maxy - miny)).max(1e-6) * 1.01;
        let mut t = QuadTree {
            quads: vec![Quad {
                size,
                ..Default::default()
            }],
            origins: vec![(cx - size / 2.0, cy - size / 2.0)],
            px: px.to_vec(),
            py: py.to_vec(),
            charge: charge.to_vec(),
        };
        for i in 0..n {
            t.insert(0, i as i32, 0);
        }
        t.accumulate(0);
        t
    }

    /// 子格索引：0=左下 / 1=右下 / 2=左上 / 3=右上（与 `ensure_children` 的 k%2 / k/2 对应）。
    fn child_index(&self, qi: usize, x: f64, y: f64) -> usize {
        let half = self.quads[qi].size / 2.0;
        let (ox, oy) = self.origins[qi];
        let mx = if x < ox + half { 0 } else { 1 };
        let my = if y < oy + half { 0 } else { 2 };
        mx + my
    }

    fn ensure_children(&mut self, qi: usize) {
        let q = self.quads[qi];
        let half = q.size / 2.0;
        let (ox, oy) = self.origins[qi];
        let mut created = [EMPTY; 4];
        for (k, slot) in created.iter_mut().enumerate() {
            if q.children[k] >= 0 {
                *slot = q.children[k];
                continue;
            }
            let (dx, dy) = ((k % 2) as f64, (k / 2) as f64);
            self.quads.push(Quad {
                size: half,
                ..Default::default()
            });
            let idx = (self.quads.len() - 1) as i32;
            self.origins.push((ox + dx * half, oy + dy * half));
            *slot = idx;
        }
        self.quads[qi].children = created;
    }

    /// 插点：叶空则落点、叶有点则分裂下推、内部节点则继续下降。深度上限防病态重合（见模块头）。
    fn insert(&mut self, qi: usize, p: i32, depth: u32) {
        if depth > 48 {
            return; // 病态重合（坐标完全相同）：该点不参与聚合，力近似误差可忽略
        }
        let state = self.quads[qi].point;
        if state == EMPTY {
            self.quads[qi].point = p;
            return;
        }
        if state >= 0 {
            // 分裂：旧点下推，再插新点
            let old = state;
            self.quads[qi].point = INTERNAL;
            self.ensure_children(qi);
            let (oc, pc) = (
                self.child_index(qi, self.px[old as usize], self.py[old as usize]),
                self.child_index(qi, self.px[p as usize], self.py[p as usize]),
            );
            let ochild = self.quads[qi].children[oc];
            let pchild = self.quads[qi].children[pc];
            self.insert(ochild as usize, old, depth + 1);
            self.insert(pchild as usize, p, depth + 1);
            return;
        }
        // 内部：下降
        self.ensure_children(qi);
        let c = self.child_index(qi, self.px[p as usize], self.py[p as usize]);
        let child = self.quads[qi].children[c];
        self.insert(child as usize, p, depth + 1);
    }

    /// 后序聚合：电荷之和 + 电荷加权质心（d3 accumulate 同款）。
    fn accumulate(&mut self, qi: usize) {
        let state = self.quads[qi].point;
        if state >= 0 {
            let p = state as usize;
            self.quads[qi].x = self.px[p];
            self.quads[qi].y = self.py[p];
            self.quads[qi].value = self.charge[p];
            return;
        }
        if state == EMPTY {
            self.quads[qi].value = 0.0;
            return;
        }
        let children = self.quads[qi].children;
        let (mut strength, mut weight, mut cx, mut cy) = (0.0, 0.0, 0.0, 0.0);
        for c in children {
            if c < 0 {
                continue;
            }
            let ci = c as usize;
            self.accumulate(ci);
            let v = self.quads[ci].value;
            let w = v.abs();
            strength += v;
            weight += w;
            cx += w * self.quads[ci].x;
            cy += w * self.quads[ci].y;
        }
        self.quads[qi].value = strength;
        if weight > 0.0 {
            self.quads[qi].x = cx / weight;
            self.quads[qi].y = cy / weight;
        }
    }

    /// 对点 n 施加斥力（d3 manyBody.apply 同款：近邻直接算、远处按质心近似、跳过自己）。
    fn apply(&self, qi: usize, n: usize, rng: &mut Lcg) -> (f64, f64) {
        let q = self.quads[qi];
        if q.value == 0.0 {
            return (0.0, 0.0);
        }
        let mut x = q.x - self.px[n];
        let mut y = q.y - self.py[n];
        let mut l = x * x + y * y;
        if THETA * THETA * l > q.size * q.size {
            // 够远 → 当单体
            if x == 0.0 {
                x = rng.jiggle();
                l += x * x;
            }
            if y == 0.0 {
                y = rng.jiggle();
                l += y * y;
            }
            if l < DISTANCE_MIN * DISTANCE_MIN {
                l = (DISTANCE_MIN * DISTANCE_MIN * l).sqrt();
            }
            return (x * q.value / l, y * q.value / l);
        }
        if q.point >= 0 {
            let j = q.point as usize;
            if j == n {
                return (0.0, 0.0);
            }
            let mut x = self.px[j] - self.px[n];
            let mut y = self.py[j] - self.py[n];
            let mut l = x * x + y * y;
            if x == 0.0 {
                x = rng.jiggle();
                l += x * x;
            }
            if y == 0.0 {
                y = rng.jiggle();
                l += y * y;
            }
            if l < DISTANCE_MIN * DISTANCE_MIN {
                l = (DISTANCE_MIN * DISTANCE_MIN * l).sqrt();
            }
            let w = self.charge[j] / l;
            return (x * w, y * w);
        }
        let mut ax = 0.0;
        let mut ay = 0.0;
        for c in q.children {
            if c < 0 {
                continue;
            }
            let (dx, dy) = self.apply(c as usize, n, rng);
            ax += dx;
            ay += dy;
        }
        (ax, ay)
    }
}

/// forceCollide（均匀网格版）：半径 60、强度 0.5，重叠量线性推开、两侧对分。
fn apply_collide(px: &mut [f64], py: &mut [f64], vx: &mut [f64], vy: &mut [f64], alpha: f64) {
    let n = px.len();
    let cell = 2.0 * COLLIDE_RADIUS;
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let key = |x: f64, y: f64| ((x / cell).floor() as i64, (y / cell).floor() as i64);
    for i in 0..n {
        grid.entry(key(px[i], py[i])).or_default().push(i);
    }
    let sum_r = 2.0 * COLLIDE_RADIUS;
    for i in 0..n {
        let (kx, ky) = key(px[i], py[i]);
        for gx in (kx - 1)..=(kx + 1) {
            for gy in (ky - 1)..=(ky + 1) {
                let Some(bucket) = grid.get(&(gx, gy)) else {
                    continue;
                };
                for &j in bucket {
                    if j <= i {
                        continue;
                    }
                    let dx = px[j] - px[i];
                    let dy = py[j] - py[i];
                    let d2 = dx * dx + dy * dy;
                    if d2 >= sum_r * sum_r {
                        continue;
                    }
                    let d = d2.sqrt();
                    let (ux, uy) = if d > 0.0 {
                        (dx / d, dy / d)
                    } else {
                        (1.0, 0.0)
                    };
                    let push = (sum_r - d) * COLLIDE_STRENGTH * alpha * 0.5;
                    vx[i] -= ux * push;
                    vy[i] -= uy * push;
                    vx[j] += ux * push;
                    vy[j] += uy * push;
                }
            }
        }
    }
}

/// 跑布局：输入已归一化的边（`(源下标, 目标下标, 权重)`），返回与 `ids` 同序的坐标。
///
/// 权重只用于**连线拉力**（复用同一强度，权重差异交给前端线宽/大小表达）——服务端初布局
/// 追求的是「形状对」，不是「权重精确」。
pub fn run_layout(ids: &[String], edges: &[(usize, usize, f64)], max_iters: u32) -> Vec<[f64; 2]> {
    let n = ids.len();
    let mut rng = Lcg::new(0x9E37_79B9_7F4A_7C15);
    // d3 initializeNodes：radius = 10·√(0.5+i)、angle = i·π(3−√5)
    const INITIAL_RADIUS: f64 = 10.0;
    let initial_angle = std::f64::consts::PI * (3.0 - 5f64.sqrt());
    let mut px = Vec::with_capacity(n);
    let mut py = Vec::with_capacity(n);
    for i in 0..n {
        let radius = INITIAL_RADIUS * (0.5 + i as f64).sqrt();
        let angle = i as f64 * initial_angle;
        // 加一点按下标递变的微偏移：保证任意两点坐标不同（四叉树分裂终止条件）
        px.push(radius * angle.cos() + i as f64 * 1e-7);
        py.push(radius * angle.sin() + i as f64 * 1e-7);
    }
    let mut vx = vec![0.0f64; n];
    let mut vy = vec![0.0f64; n];
    let mut degree = vec![0.0f64; n];
    for (a, b, _) in edges {
        degree[*a] += 1.0;
        degree[*b] += 1.0;
    }
    let charge = vec![REPEL_STRENGTH; n];
    let decay = alpha_decay();
    let mut alpha = 1.0f64;
    let mut iters = 0u32;
    while alpha >= ALPHA_MIN && iters < max_iters {
        alpha -= alpha * decay; // d3：alpha += (alphaTarget − alpha)·alphaDecay（alphaTarget=0）
        iters += 1;

        // 1) forceLink（bias = 源度数占比：枢纽少动）
        for (a, b, _) in edges {
            let (a, b) = (*a, *b);
            let mut x = px[b] + vx[b] - px[a] - vx[a];
            let mut y = py[b] + vy[b] - py[a] - vy[a];
            let mut l = (x * x + y * y).sqrt();
            if l == 0.0 {
                x = rng.jiggle();
                y = rng.jiggle();
                l = (x * x + y * y).sqrt().max(1e-9);
            }
            let k = (l - LINK_DISTANCE) / l * alpha * LINK_STRENGTH;
            x *= k;
            y *= k;
            let denom = degree[a] + degree[b];
            let bias = if denom > 0.0 { degree[a] / denom } else { 0.5 };
            vx[b] -= x * bias;
            vy[b] -= y * bias;
            vx[a] += x * (1.0 - bias);
            vy[a] += y * (1.0 - bias);
        }

        // 2) forceManyBody（Barnes-Hut）
        let tree = QuadTree::build(&px, &py, &charge);
        for i in 0..n {
            let (dx, dy) = tree.apply(0, i, &mut rng);
            vx[i] += dx * alpha;
            vy[i] += dy * alpha;
        }

        // 3) forceX / forceY（向心）
        for i in 0..n {
            vx[i] += (0.0 - px[i]) * CENTER_STRENGTH * alpha;
            vy[i] += (0.0 - py[i]) * CENTER_STRENGTH * alpha;
        }

        // 4) forceCollide
        apply_collide(&mut px, &mut py, &mut vx, &mut vy, alpha);

        // 5) 位移（d3：先 v *= velocityDecay 再 x += v）
        for i in 0..n {
            vx[i] *= VELOCITY_DECAY;
            vy[i] *= VELOCITY_DECAY;
            px[i] += vx[i];
            py[i] += vy[i];
        }
    }
    px.into_iter().zip(py).map(|(x, y)| [x, y]).collect()
}

/// 文件级依赖图：`(节点路径按首次出现序, 边 (源下标, 目标下标, 权重))`。
pub type FileGraph = (Vec<String>, Vec<(usize, usize, f64)>);

/// 从索引库读文件级依赖图（与 `query.rs::full_graph_query` **同一粒度口径**）：跨文件、非 contains
/// 的边按 (源文件,目标文件) 聚合计数；节点按首次出现顺序（与图响应一致，便于前后端同一初值）。
/// 额外加 `sf.path, tf.path` 次序键——布局要求**可复现**（图响应那边保持原样，不改口径）。
pub fn file_graph(db_path: &Path) -> Result<FileGraph, CgError> {
    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| CgError::Parse(format!("索引库打开失败: {e}")))?;
    let mut stmt = conn
        .prepare(
            "SELECT sf.path, tf.path, count(*) AS w FROM edges e \
             JOIN nodes ns ON ns.id = e.source JOIN files sf ON sf.path = ns.file_path \
             JOIN nodes nt ON nt.id = e.target JOIN files tf ON tf.path = nt.file_path \
             WHERE sf.path != tf.path AND e.kind != 'contains' \
             GROUP BY sf.path, tf.path ORDER BY w DESC, sf.path, tf.path",
        )
        .map_err(|e| CgError::Parse(format!("索引查询失败: {e}")))?;
    let rows: Vec<(String, String, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| CgError::Parse(format!("索引查询失败: {e}")))?
        .collect::<Result<_, _>>()
        .map_err(|e| CgError::Parse(format!("索引读取失败: {e}")))?;
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut ids: Vec<String> = Vec::new();
    let mut edges: Vec<(usize, usize, f64)> = Vec::new();
    for (from, to, w) in rows {
        let a = *index.entry(from.clone()).or_insert_with(|| {
            ids.push(from);
            ids.len() - 1
        });
        let b = *index.entry(to.clone()).or_insert_with(|| {
            ids.push(to);
            ids.len() - 1
        });
        edges.push((a, b, w as f64));
    }
    Ok((ids, edges))
}

/// 布局文件（`<仓库>/.codegraph/layout.json`）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LayoutFile {
    /// 结构版本（LAYOUT_VERSION）
    pub version: u32,
    /// 算法标识（LAYOUT_ALGORITHM）
    pub algorithm: String,
    pub iterations: u32,
    pub nodes: usize,
    pub edges: usize,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    /// 文件路径 → [x, y]
    pub positions: HashMap<String, [f64; 2]>,
}

/// layout.json 的落点：与 codegraph.db 同目录。
pub fn layout_path(db_path: &Path) -> PathBuf {
    db_path.with_file_name("layout.json")
}

/// 算 + 落盘（阻塞；调用方放 spawn_blocking）。返回落点与元信息。
pub fn compute_and_write(db_path: &Path, max_iters: u32) -> Result<(LayoutFile, PathBuf), CgError> {
    let (ids, edges) = file_graph(db_path)?;
    if ids.is_empty() {
        return Err(CgError::Parse(
            "文件级依赖图为空（没有跨文件边）——不落布局".into(),
        ));
    }
    let pos = run_layout(&ids, &edges, max_iters);
    if pos.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
        return Err(CgError::Parse("布局计算出现非法坐标（NaN/Inf）".into()));
    }
    let file = LayoutFile {
        version: LAYOUT_VERSION,
        algorithm: LAYOUT_ALGORITHM.to_string(),
        iterations: max_iters,
        nodes: ids.len(),
        edges: edges.len(),
        generated_at: chrono::Utc::now(),
        positions: ids.into_iter().zip(pos).collect(),
    };
    let out = layout_path(db_path);
    let tmp = out.with_extension("json.tmp");
    let bytes =
        serde_json::to_vec(&file).map_err(|e| CgError::Parse(format!("布局序列化失败: {e}")))?;
    std::fs::write(&tmp, bytes).map_err(|e| CgError::Storage(format!("布局落盘失败: {e}")))?;
    std::fs::rename(&tmp, &out).map_err(|e| CgError::Storage(format!("布局改名失败: {e}")))?;
    Ok((file, out))
}

/// 读布局（不存在 / 损坏 / 版本不认 → None：调用方按「没有布局」处理）。
pub fn read_layout(path: &Path) -> Option<LayoutFile> {
    let bytes = std::fs::read(path).ok()?;
    let f: LayoutFile = serde_json::from_slice(&bytes).ok()?;
    (f.version == LAYOUT_VERSION).then_some(f)
}

#[cfg(test)]
mod tests {
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
}

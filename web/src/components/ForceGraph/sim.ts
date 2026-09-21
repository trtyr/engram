/**
 * 共享图谱引擎 · 常数与纯函数（2026-09-21）
 *
 * **常数与公式全部照抄 Obsidian 真机实测值**（`obsidian.asar` 逆向：`sim.js` + `graph.json` schema，
 * 见 obsidian-desktop-internals.md §13；交互口径见 help.obsidian.md/plugins/graph）。不自行发明——
 * 要偏离必须先写理由。本文件**不依赖 DOM / Worker / sigma**，纯函数可单测（sim.test.ts）。
 *
 * 架构（Obsidian 同款）：渲染与物理分离——sigma(WebGL) 只画，物理在 Web Worker 里跑 d3-force fork，
 * 每 tick 以 Float32 transferable 回传坐标。本文件定义两侧共用的协议与数学。
 */

/** Obsidian 实测常数（逐条有出处，勿随手改）。 */
export const OBSIDIAN = {
  /** 初始 alpha / 停机阈值 / 数据变化后的 reheat alpha */
  alphaInit: 1,
  alphaMin: 0.001,
  alphaReheat: 0.3,
  /** 速度衰减（越大越"黏"，动得越稳） */
  velocityDecay: 0.6,
  /** forceLink 默认连线长度（滑杆 30..500 的默认位） */
  linkDistance: 250,
  /** forceManyBody 的最小作用距离 */
  repelDistanceMin: 30,
  /** 四叉树精度（d3 默认 0.9） */
  theta: 0.9,
  /** forceCollide 默认半径与强度 */
  collide: { radius: 60, strength: 0.5 },
  /** 滑杆默认位（存的**不是**力值本身：中心/连线拉力过曲线、斥力过立方） */
  slider: { center: 0.5187, repel: 10, linkStrength: 1, linkDistance: 250 },
  /** 节点半径：nodeSizeMult × clamp(3·√(weight+1), 8, 30) */
  nodeRadius: { k: 3, min: 8, max: 30 },
  /** 缩放：clamp [1/128, 8]，每帧指数插值 0.85 */
  zoom: { min: 1 / 128, max: 8, lerp: 0.85 },
} as const

/**
 * `alphaDecay = 1 − 0.001^(1/300)`（Obsidian 实测：约 300 tick 收敛到 alphaMin）。
 * d3 的 simulation 在 `alpha < alphaMin` 时自动停表 —— 这就是「不动就真静止」的来源。
 */
export const ALPHA_DECAY = 1 - Math.pow(OBSIDIAN.alphaMin, 1 / 300)

/** 滑杆位置 → 力值曲线（Obsidian：centerStrength / linkStrength 用它；curve(0)=0，curve(1)=1）。 */
export function curve(v: number, k = 0.01): number {
  const x = Math.min(1, Math.max(0, Number.isFinite(v) ? v : 0))
  return (Math.pow(k, 1 - x) - k) / (1 - k)
}

/** 斥力：滑杆 0..20 → 力 = −v³（Obsidian 原样：默认 v=10 ⇒ −1000）。 */
export function repelFromSlider(v: number): number {
  const x = Math.max(0, Number.isFinite(v) ? v : 0)
  return -(x ** 3)
}

/** 节点半径：`mult × clamp(3·√(weight+1), 8, 30)`（weight = 被引用+引用数）。 */
export function nodeRadius(weight: number, mult = 1): number {
  const w = Number.isFinite(weight) && weight > 0 ? weight : 0
  const base = OBSIDIAN.nodeRadius.k * Math.sqrt(w + 1)
  return Math.min(OBSIDIAN.nodeRadius.max, Math.max(OBSIDIAN.nodeRadius.min, base)) * mult
}

/**
 * 文字透明度：`clamp(log2(scale) + 1 − textFadeMultiplier, 0, 1)`。
 *
 * `scale` = **放大倍数**（越大越放大；sigma 里传 `1 / camera.ratio`，因为 sigma 的 ratio 越大越缩小）。
 * 于是：放大 → alpha→1（标签浮现）；缩小到 1/8 → alpha=0（标签隐没，避免缩小时一片标签糊在一起）。
 * `textFadeMultiplier` 就是 Obsidian 齿轮里的「文字淡出」滑杆。
 */
export function labelAlpha(scale: number, fadeMultiplier: number): number {
  const s = scale > 0 && Number.isFinite(scale) ? scale : 1
  const a = Math.log2(s) + 1 - fadeMultiplier
  return Math.min(1, Math.max(0, a))
}

/** 标签尺寸倍数：`√(1/scale)`（Obsidian 原样）。 */
export function labelScale(scale: number): number {
  const s = scale > 0 && Number.isFinite(scale) ? scale : 1
  return Math.sqrt(1 / s)
}

// ── 力参数（滑杆位 → 下发给 worker 的力值） ──────────────────────────────────

export interface ForceParams {
  /** 向心（Obsidian Center force）：滑杆 0..1 */
  center: number
  /** 斥力（Repel force）：滑杆 0..20 */
  repel: number
  /** 连线拉力（Link force）：滑杆 0..1 */
  linkStrength: number
  /** 连线长度（Link distance）：滑杆 30..500，直传 */
  linkDistance: number
}

export const DEFAULT_FORCES: ForceParams = {
  center: OBSIDIAN.slider.center,
  repel: OBSIDIAN.slider.repel,
  linkStrength: OBSIDIAN.slider.linkStrength,
  linkDistance: OBSIDIAN.slider.linkDistance,
}

/** 滑杆位 → worker 侧真正的力（曲线与立方在此换算，worker 只收力值）。 */
export function forcesToWorker(p: ForceParams): WorkerForces {
  return {
    centerStrength: curve(p.center),
    repel: repelFromSlider(p.repel),
    linkStrength: curve(p.linkStrength),
    linkDistance: p.linkDistance,
  }
}

export interface WorkerForces {
  centerStrength: number
  repel: number
  linkStrength: number
  linkDistance: number
}

// ── 显示参数 ────────────────────────────────────────────────────────────────

export interface DisplayParams {
  /** 节点大小倍率 0.1..5（乘在 nodeRadius 上） */
  nodeSize: number
  /** 连线粗细倍率 0.1..5 */
  lineSize: number
  /** 文字淡出阈值 −3..3（越大标签越晚出现） */
  textFade: number
  /** 箭头 */
  arrows: boolean
}

export const DEFAULT_DISPLAY: DisplayParams = {
  nodeSize: 1,
  lineSize: 1,
  textFade: 0,
  arrows: false,
}

// ── LOD（本引擎自有策略：Obsidian 只有文字淡出 + 过滤，没有降级档；我们的大图更多） ──

export interface LodPlan {
  mode: 'full' | 'simplified'
  nodeLimit: number
  edgeLimit: number
  /** 为什么降级（给用户看的一句话） */
  reason?: string
}

export const LOD_THRESHOLDS = {
  /**
   * **自动降级**阈值（超过才降级）：只留给真正的巨图。
   * 定这个数的口径：safeline-2（3721 节点 / 28000 边）必须**开图即出完整图**（goal req 3），
   * 故阈值抬到它之上；同时巨图仍要有兜底（req 5）。
   */
  fullNodes: 6000,
  fullEdges: 60000,
  /** 简化档保留量（按度数 Top-N + 两端都在集合内的边） */
  simplifyNodes: 1500,
  simplifyEdges: 9000,
  /** 「只看主干」显式入口的可见门槛（超过就允许手动切简化档） */
  suggestNodes: 1200,
} as const

/** 简化档计划（自动降级与「只看主干」共用同一套 Top-N 规则）。 */
function simplifiedPlan(nodeCount: number, edgeCount: number, explicit: boolean): LodPlan {
  const reason = explicit
    ? `只看主干：度数最高的 ${Math.min(nodeCount, LOD_THRESHOLDS.simplifyNodes)} 个节点（点「渲染全图」看全量）`
    : `${nodeCount} 个节点 / ${edgeCount} 条边 —— 先画度数最高的 ${LOD_THRESHOLDS.simplifyNodes} 个节点，可点「渲染全图」看全量`
  return {
    mode: 'simplified',
    nodeLimit: Math.min(nodeCount, LOD_THRESHOLDS.simplifyNodes),
    edgeLimit: Math.min(edgeCount, LOD_THRESHOLDS.simplifyEdges),
    reason,
  }
}

/**
 * 决定要不要降级。
 * - `forced='full'`：用户点了「渲染全图」——最高优先级，永远不降。
 * - `forced='simplified'`：用户点了「只看主干」——显式降级。
 * - 默认 auto：只在**超过** `LOD_THRESHOLDS` 的巨图上自动降级。
 */
export function decideLod(
  nodeCount: number,
  edgeCount: number,
  forced: 'full' | 'simplified' | undefined = undefined,
): LodPlan {
  const full: LodPlan = { mode: 'full', nodeLimit: nodeCount, edgeLimit: edgeCount }
  if (forced === 'full') return full
  if (forced === 'simplified') return simplifiedPlan(nodeCount, edgeCount, true)
  const tooBig = nodeCount > LOD_THRESHOLDS.fullNodes || edgeCount > LOD_THRESHOLDS.fullEdges
  if (!tooBig) return full
  return simplifiedPlan(nodeCount, edgeCount, false)
}

export interface SimNode {
  id: string
  /** 度数（被引用+引用）——半径与 LOD 排序都靠它 */
  weight?: number
  /** 固定锚点（如圈子的「我」）：不参与物理 */
  fixed?: boolean
  x?: number
  y?: number
}

export interface SimEdge {
  source: string
  target: string
  weight?: number
}

/** 度数统计（半径为「被引用越多越大」，故与方向无关）。 */
export function degreesOf(edges: SimEdge[]): Map<string, number> {
  const deg = new Map<string, number>()
  for (const e of edges) {
    deg.set(e.source, (deg.get(e.source) ?? 0) + 1)
    deg.set(e.target, (deg.get(e.target) ?? 0) + 1)
  }
  return deg
}

/** 简化图：按度数取 Top-N 节点，边只保留两端都在集合内的（保持可读性）。 */
export function simplifyGraph(
  nodes: SimNode[],
  edges: SimEdge[],
  plan: LodPlan,
): { nodes: SimNode[]; edges: SimEdge[] } {
  if (plan.mode === 'full' || nodes.length <= plan.nodeLimit) {
    return { nodes, edges: edges.slice(0, plan.edgeLimit) }
  }
  const deg = degreesOf(edges)
  const ranked = [...nodes].sort(
    (a, b) => (deg.get(b.id) ?? 0) - (deg.get(a.id) ?? 0) || a.id.localeCompare(b.id),
  )
  // 输出按度数降序（确定性：热门节点排在前面，便于渲染层按序铺初始位置）
  const kept = ranked.slice(0, plan.nodeLimit)
  const keep = new Set(kept.map((n) => n.id))
  const keptEdges = edges
    .filter((e) => keep.has(e.source) && keep.has(e.target))
    .slice(0, plan.edgeLimit)
  return { nodes: kept, edges: keptEdges }
}

// ── Worker 协议（主线程 ⇄ 物理） ────────────────────────────────────────────

export interface InitMsg {
  type: 'init'
  nodes: { id: string; weight: number; fixed: boolean; x?: number; y?: number }[]
  edges: { source: string; target: string; weight: number }[]
  forces: WorkerForces
  /** 有服务端初布局时给 alphaReheat（只微调），没有才给 alphaInit（自行收敛） */
  alpha: number
}

export type SimInMsg =
  | InitMsg
  | { type: 'params'; forces: WorkerForces }
  | { type: 'pin'; id: string; x: number; y: number }
  | { type: 'unpin'; id: string }
  | { type: 'reheat'; alpha?: number }
  | { type: 'stop' }

export type SimOutMsg =
  | { type: 'tick'; alpha: number; positions: Float32Array }
  | { type: 'end'; alpha: number; ticks: number }

import { describe, expect, it } from 'vitest'
import {
  forceLink,
  forceManyBody,
  forceSimulation,
  forceX,
  forceY,
  type SimulationNodeDatum,
} from 'd3-force'
import {
  ALPHA_DECAY,
  DEFAULT_DISPLAY,
  DEFAULT_FORCES,
  LOD_THRESHOLDS,
  OBSIDIAN,
  curve,
  decideLod,
  degreesOf,
  forcesToWorker,
  labelAlpha,
  labelScale,
  nodeRadius,
  repelFromSlider,
  simplifyGraph,
} from './sim'

describe('Obsidian 常数（照抄实测值，勿随手改）', () => {
  it('alphaDecay = 1 − 0.001^(1/300)（约 300 tick 收敛）', () => {
    expect(ALPHA_DECAY).toBeCloseTo(1 - Math.pow(0.001, 1 / 300), 10)
    expect(ALPHA_DECAY).toBeGreaterThan(0.02)
    expect(ALPHA_DECAY).toBeLessThan(0.03)
  })

  it('默认滑杆位换算出的力 = Obsidian 实测的力值', () => {
    const f = forcesToWorker(DEFAULT_FORCES)
    // centerStrength：滑杆 0.5187 → 曲线 → ≈ 0.1（Obsidian 实测 0.1）
    expect(f.centerStrength).toBeCloseTo(0.1, 2)
    // 斥力：滑杆 10 → −(10³) = −1000（Obsidian 实测 −1000）
    expect(f.repel).toBe(-1000)
    expect(f.linkStrength).toBeCloseTo(1, 5)
    expect(f.linkDistance).toBe(250)
    expect(OBSIDIAN.velocityDecay).toBe(0.6)
    expect(OBSIDIAN.collide).toEqual({ radius: 60, strength: 0.5 })
    expect(OBSIDIAN.alphaMin).toBe(0.001)
  })
})

describe('curve(v, 0.01)（滑杆位 → 力值）', () => {
  it('端点固定：curve(0)=0、curve(1)=1', () => {
    expect(curve(0)).toBeCloseTo(0, 10)
    expect(curve(1)).toBeCloseTo(1, 10)
  })

  it('单调递增且越界自动夹到 [0,1]', () => {
    expect(curve(0.2)).toBeLessThan(curve(0.6))
    expect(curve(-1)).toBeCloseTo(0, 10)
    expect(curve(2)).toBeCloseTo(1, 10)
  })
})

describe('斥力：滑杆值过 −v³', () => {
  it('−v³：默认 10 → −1000；0 → 0；20 → −8000', () => {
    expect(repelFromSlider(10)).toBe(-1000)
    expect(repelFromSlider(0)).toBe(-0)
    expect(repelFromSlider(20)).toBe(-8000)
    expect(repelFromSlider(-5)).toBe(-0)
  })
})

describe('节点半径 clamp(3·√(weight+1), 8, 30) × 倍率', () => {
  it('下限 8：weight=0 → 3 → 夹到 8', () => {
    expect(nodeRadius(0)).toBe(8)
  })
  it('上限 30：weight 很大 → 夹到 30', () => {
    expect(nodeRadius(1000)).toBe(30)
    expect(nodeRadius(80)).toBeCloseTo(27, 6) // 3·√81 = 27，还没到顶
  })
  it('中间段按 3·√(w+1)：weight=24 → 3·5 = 15', () => {
    expect(nodeRadius(24)).toBeCloseTo(15, 6)
  })
  it('倍率线性叠加：weight=24 × 0.5 → 7.5（倍率不受 clamp 二次限制）', () => {
    expect(nodeRadius(24, 0.5)).toBeCloseTo(7.5, 6)
  })
})

describe('文字淡出：clamp(log2(scale) + 1 − multiplier, 0, 1)', () => {
  it('scale=1、multiplier=0 → 1（全亮）', () => {
    expect(labelAlpha(1, 0)).toBeCloseTo(1, 6)
  })
  it('scale = 放大倍数：放大→浮现、缩小到 1/8→隐没', () => {
    expect(labelAlpha(4, 0)).toBe(1) // 放大 4 倍：log2(4)+1 = 3 → 夹到 1
    expect(labelAlpha(1 / 8, 0)).toBe(0) // 缩小到 1/8：log2(1/8)+1 = −2 → 夹到 0
  })
  it('multiplier 越大越早隐（阈值可调）', () => {
    expect(labelAlpha(1, 0)).toBeGreaterThan(labelAlpha(1, 0.5))
    expect(labelAlpha(1, 3)).toBe(0)
  })
  it('标签尺寸倍数 = √(1/scale)', () => {
    expect(labelScale(1)).toBeCloseTo(1, 6)
    expect(labelScale(4)).toBeCloseTo(0.5, 6)
  })
})

describe('LOD：超阈值降级 + 逃生门', () => {
  it('小图直接全量', () => {
    const p = decideLod(300, 1500)
    expect(p.mode).toBe('full')
    expect(p.nodeLimit).toBe(300)
  })
  it('safeline-2 量级（3721 节点 / 28000 边）**不降级**——开图即出完整图（goal req 3）', () => {
    const p = decideLod(3721, 28000)
    expect(p.mode).toBe('full')
    expect(p.nodeLimit).toBe(3721)
  })
  it('真巨图（超阈值）→ 简化档，并给出人话原因', () => {
    const p = decideLod(9000, 90000)
    expect(p.mode).toBe('simplified')
    expect(p.nodeLimit).toBe(LOD_THRESHOLDS.simplifyNodes)
    expect(p.reason).toContain('渲染全图')
  })
  it('边超阈值同样降级', () => {
    expect(decideLod(100, 70000).mode).toBe('simplified')
  })
  it('显式 forced=full（用户点了「渲染全图」）优先于巨图降级', () => {
    expect(decideLod(9000, 90000, 'full').mode).toBe('full')
  })
  it('显式 forced=simplified（用户点「只看主干」）——即使图不大也降，且提示说明是用户选择', () => {
    const p = decideLod(3721, 28000, 'simplified')
    expect(p.mode).toBe('simplified')
    expect(p.reason).toContain('只看主干')
  })
})

describe('度数统计与简化图', () => {
  const edges = [
    { source: 'a', target: 'b' },
    { source: 'a', target: 'c' },
    { source: 'b', target: 'c' },
    { source: 'c', target: 'd' },
  ]
  it('度数：c=3、a=2、b=2、d=1', () => {
    const deg = degreesOf(edges)
    expect(deg.get('c')).toBe(3)
    expect(deg.get('a')).toBe(2)
    expect(deg.get('d')).toBe(1)
  })
  it('简化图按度数 Top-N 取节点，且只留两端都在集合内的边', () => {
    const nodes = ['a', 'b', 'c', 'd'].map((id) => ({ id }))
    const plan = { mode: 'simplified' as const, nodeLimit: 3, edgeLimit: 10 }
    const out = simplifyGraph(nodes, edges, plan)
    expect(out.nodes.map((n) => n.id)).toEqual(['c', 'a', 'b'])
    // d 出局 → c-d 边被丢弃
    expect(out.edges.every((e) => e.source !== 'd' && e.target !== 'd')).toBe(true)
    expect(out.edges).toHaveLength(3)
  })
  it('全量档不改节点集合（边只按 edgeLimit 截断）', () => {
    const nodes = ['a', 'b', 'c', 'd'].map((id) => ({ id }))
    const out = simplifyGraph(nodes, edges, { mode: 'full', nodeLimit: 10, edgeLimit: 2 })
    expect(out.nodes).toHaveLength(4)
    expect(out.edges).toHaveLength(2)
  })
})

describe('默认参数与 Obsidian 滑杆区间一致', () => {
  it('默认位就是 Obsidian 的默认位', () => {
    expect(DEFAULT_FORCES.center).toBe(OBSIDIAN.slider.center)
    expect(DEFAULT_FORCES.repel).toBe(10)
    expect(DEFAULT_FORCES.linkStrength).toBe(1)
    expect(DEFAULT_FORCES.linkDistance).toBe(250)
    expect(DEFAULT_DISPLAY.textFade).toBe(0)
    expect(DEFAULT_DISPLAY.nodeSize).toBe(1)
    expect(DEFAULT_DISPLAY.lineSize).toBe(1)
  })
})

/** 斥力（与 worker 同款：Obsidian 滑杆默认位 10 过 −v³）——此处直接写值，避免 import 私有常量 */
const REPEL_STRENGTH_FOR_TEST = -1000

describe('静停（不动就真静止）：与 worker 同参数下 ~300 tick 自行停表', () => {
  it('d3 simulation 在 alpha < alphaMin 时触发 end 并停止产生 tick', async () => {
    interface TNode extends SimulationNodeDatum {
      id: string
    }
    const N = 60
    const nodes: TNode[] = Array.from({ length: N }, (_, i) => ({
      id: `n${i}`,
      x: Math.random() * 10,
      y: Math.random() * 10,
    }))
    const links = Array.from({ length: N - 1 }, (_, i) => ({ source: `n${i}`, target: `n${i + 1}` }))
    const sim = forceSimulation<TNode>(nodes)
      .force(
        'link',
        forceLink<TNode, { source: string; target: string }>(links)
          .id((d) => d.id)
          .distance(OBSIDIAN.linkDistance)
          .strength(1),
      )
      .force(
        'charge',
        forceManyBody<TNode>()
          .strength(REPEL_STRENGTH_FOR_TEST)
          .distanceMin(OBSIDIAN.repelDistanceMin),
      )
      .force('x', forceX<TNode>(0).strength(0.1))
      .force('y', forceY<TNode>(0).strength(0.1))
      .velocityDecay(OBSIDIAN.velocityDecay)
      .alphaDecay(ALPHA_DECAY)
      .alphaMin(OBSIDIAN.alphaMin)

    let ticks = 0
    sim.on('tick', () => {
      ticks += 1
    })
    await new Promise<void>((resolve) => {
      sim.on('end', () => resolve())
    })
    // Obsidian 口径：约 300 tick 收敛（允许 ±25% 浮动，只证明「真的会停」而不是永续跑）
    expect(ticks).toBeGreaterThan(200)
    expect(ticks).toBeLessThan(400)
    // 停表后再等一会：不应再有 tick（worker 侧此时已无任何定时器工作）
    const after = ticks
    await new Promise((r) => setTimeout(r, 120))
    expect(ticks).toBe(after)
    sim.stop()
    // ~300 tick × ~17ms 帧间隔 ≈ 5s+，给足超时（本用例就是验证「真的会停」）
  }, 30_000)
})

/// <reference lib="webworker" />
/**
 * 共享图谱引擎 · 物理 Worker（d3-force fork，照 Obsidian `sim.js` 的力配置与常数）
 *
 * 为什么不放主线程：d3-force 每 tick 是 O(n log n) 的整图遍历，3721 节点 / 2.8 万边时单次
 * 就是几十毫秒——放主线程必卡。这里在 worker 里跑，每 tick 把坐标以 **Float32 transferable**
 * 回传，主线程只做 sigma 渲染。
 *
 * 「不动就真静止」：d3 的 simulation 在 `alpha < alphaMin(=0.001)` 时自动停表（alphaDecay 取
 * `1 − 0.001^(1/300)`，约 300 tick）——停表后不再有任何 tick，CPU 归零；交互时 reheat 重启。
 */
import {
  forceCollide,
  forceLink,
  forceManyBody,
  forceSimulation,
  forceX,
  forceY,
  type Simulation,
  type SimulationLinkDatum,
  type SimulationNodeDatum,
} from 'd3-force'
import { ALPHA_DECAY, OBSIDIAN, type SimInMsg, type SimOutMsg, type WorkerForces } from './sim'

interface WNode extends SimulationNodeDatum {
  id: string
  weight: number
  fixed: boolean
}
interface WLink extends SimulationLinkDatum<WNode> {
  weight: number
}

let sim: Simulation<WNode, WLink> | null = null
let nodes: WNode[] = []
let byId = new Map<string, WNode>()
let ticks = 0

const post = (msg: SimOutMsg, transfer: Transferable[] = []) =>
  (self as unknown as Worker).postMessage(msg, transfer)

function sendTick() {
  ticks += 1
  const positions = new Float32Array(nodes.length * 2)
  for (let i = 0; i < nodes.length; i++) {
    positions[i * 2] = nodes[i].x ?? 0
    positions[i * 2 + 1] = nodes[i].y ?? 0
  }
  post({ type: 'tick', alpha: sim?.alpha() ?? 0, positions }, [positions.buffer])
}

function applyForces(f: WorkerForces) {
  if (!sim) return
  sim.force('x', forceX<WNode>(0).strength(f.centerStrength))
  sim.force('y', forceY<WNode>(0).strength(f.centerStrength))
  sim.force(
    'charge',
    forceManyBody<WNode>().strength(f.repel).distanceMin(OBSIDIAN.repelDistanceMin).theta(OBSIDIAN.theta),
  )
  // forceCollide：Obsidian 实测 半径 60 / 强度 0.5（防节点叠成一坨）
  sim.force(
    'collide',
    forceCollide<WNode>(OBSIDIAN.collide.radius).strength(OBSIDIAN.collide.strength),
  )
  const link = sim.force('link') as ReturnType<typeof forceLink<WNode, WLink>> | undefined
  if (link) link.distance(f.linkDistance).strength(f.linkStrength)
}

self.onmessage = (ev: MessageEvent<SimInMsg>) => {
  const msg = ev.data
  switch (msg.type) {
    case 'init': {
      sim?.stop()
      ticks = 0
      nodes = msg.nodes.map((n) => ({
        id: n.id,
        weight: n.weight,
        fixed: n.fixed,
        x: n.x,
        y: n.y,
        // 固定锚点（如圈子的「我」）用 fx/fy 钉死在给定位置
        ...(n.fixed ? { fx: n.x ?? 0, fy: n.y ?? 0 } : {}),
      }))
      byId = new Map(nodes.map((n) => [n.id, n]))
      const links: WLink[] = msg.edges.map((e) => ({
        source: e.source,
        target: e.target,
        weight: e.weight,
      }))
      sim = forceSimulation<WNode, WLink>(nodes)
        .force(
          'link',
          forceLink<WNode, WLink>(links)
            .id((d) => d.id)
            .distance(msg.forces.linkDistance)
            .strength(msg.forces.linkStrength),
        )
        .velocityDecay(OBSIDIAN.velocityDecay)
        .alphaDecay(ALPHA_DECAY)
        .alphaMin(OBSIDIAN.alphaMin)
        .alpha(msg.alpha)
        .on('tick', sendTick)
        .on('end', () => post({ type: 'end', alpha: sim?.alpha() ?? 0, ticks }))
      applyForces(msg.forces)
      sendTick() // 先给一帧（有初布局时这就是「打开即收敛」的那一帧）
      break
    }
    case 'params': {
      applyForces(msg.forces)
      if (sim) {
        sim.alpha(Math.max(sim.alpha(), OBSIDIAN.alphaReheat))
        sim.restart()
      }
      break
    }
    case 'pin': {
      const n = byId.get(msg.id)
      if (n && sim) {
        n.fx = msg.x
        n.fy = msg.y
        sim.alpha(Math.max(sim.alpha(), OBSIDIAN.alphaReheat))
        sim.restart()
      }
      break
    }
    case 'unpin': {
      const n = byId.get(msg.id)
      if (n && sim) {
        if (n.fixed) {
          n.fx = n.x ?? 0
          n.fy = n.y ?? 0
        } else {
          n.fx = null
          n.fy = null
        }
        sim.alpha(Math.max(sim.alpha(), OBSIDIAN.alphaReheat))
        sim.restart()
      }
      break
    }
    case 'reheat': {
      if (sim) {
        sim.alpha(msg.alpha ?? OBSIDIAN.alphaInit)
        sim.restart()
      }
      break
    }
    case 'stop': {
      sim?.stop()
      break
    }
  }
}

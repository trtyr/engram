/**
 * 圈子图（记忆星系）v4：Obsidian 手感重写（2026-09-18，用户反馈「死板卡密集」）。
 *
 * 病根与对症（Obsidian 图谱原理：WebGL 渲染 + 永续力模拟 + 力参数可调）：
 * - 死板 → 力模拟永续运行（能量模型：交互唤醒、衰减自动降频待机微颤），
 *   拖拽松手后邻居真实回弹，图永远是活的；
 * - 卡 → 每帧 1 次 FA2 迭代（原先 2 帧 3 次反而重）+ skipIndexation + 低能量 10 帧一次待机；
 * - 密集 → 「我-实体」轮辐边不再建图（ME 只作 fixed 空间锚点），图谱只由实体间
 *   真实共现/关系构成；向心力默认调松。
 *
 * 交互：拖拽跟手（拖拽期关 captor 防相机 pan——v3 病根见 git 历史）、hover 邻居高亮
 * （nodeReducer/edgeReducer 动态淡出非邻居）、右上角三滑杆（斥力/向心/惯性）实时调参、
 * 节点坐标 localStorage 持久化（重开不重新爆炸，「重排布局」清坐标重新炸开）。
 */
import { useEffect, useRef } from 'react'
import Graph from 'graphology'
import Sigma from 'sigma'
import forceAtlas2 from 'graphology-layout-forceatlas2'
import type { EntityGraph as GraphData } from '@/lib/api'
import { useThemeTick } from '@/lib/theme'
import { ENTITY_KIND_COLOR } from '@/lib/ui'

/** 实体类型色：见 lib/ui.ts ENTITY_KIND_COLOR（与 WikiGraph 调色板同源） */
const KIND_COLOR = ENTITY_KIND_COLOR

const ME = '__me__'
const POS_KEY = 'galaxy-positions-v4'

/** 可调参数（Obsidian「力」面板同思路：斥力/向心/惯性三滑杆） */
interface ForceParams {
  scalingRatio: number // 斥力——节点互散的强度（越大越散）
  gravity: number // 向心力——往中心收的强度
  slowDown: number // 惯性——越大动得越慢越稳
}
const DEFAULT_PARAMS: ForceParams = { scalingRatio: 12, gravity: 1.2, slowDown: 4 }

function themeColors() {
  const cs = getComputedStyle(document.documentElement)
  return {
    fg: cs.getPropertyValue('--foreground').trim() || '#0a0a0a',
    muted: cs.getPropertyValue('--muted-foreground').trim() || '#71717a',
    border: cs.getPropertyValue('--border').trim() || '#e5e5e5',
  }
}

/** 节点坐标持久化：id → {x,y}（跨会话记住布局，重开不重炸） */
function loadPositions(): Map<string, { x: number; y: number }> {
  try {
    const raw = localStorage.getItem(POS_KEY)
    if (!raw) return new Map()
    return new Map(Object.entries(JSON.parse(raw)) as [string, { x: number; y: number }][])
  } catch {
    return new Map()
  }
}
function savePositions(g: Graph) {
  try {
    const out: Record<string, { x: number; y: number }> = {}
    g.forEachNode((id, attr) => {
      out[id] = { x: attr.x, y: attr.y }
    })
    localStorage.setItem(POS_KEY, JSON.stringify(out))
  } catch {
    /* 存不下就算了（隐私模式等） */
  }
}

export default function EntityGalaxy({
  graph,
  onSelect,
  onGoPersona,
}: {
  graph: GraphData
  onSelect: (id: string) => void
  onGoPersona: () => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const sigmaRef = useRef<Sigma | null>(null)
  const themeTick = useThemeTick()
  const theme = useRef(themeColors()) // eslint-disable-line react-hooks/exhaustive-deps
  theme.current = themeColors()
  const paramsRef = useRef<ForceParams>({ ...DEFAULT_PARAMS })
  /** 主 effect 把 wake 挂上来，滑杆拖动时唤醒模拟（不重建图、不闪屏） */
  const wakeRef = useRef<(min?: number) => void>(null)

  useEffect(() => {
    const el = ref.current
    if (!el) return
    const g = new Graph({ multi: false })
    // 度数统计：共现 + 关系都算连接——枢纽自然更大（Obsidian 同款视觉层级）
    const deg = new Map<string, number>()
    const bump = (a: string, b: string) => {
      deg.set(a, (deg.get(a) ?? 0) + 1)
      deg.set(b, (deg.get(b) ?? 0) + 1)
    }
    for (const e of graph.edges) bump(e.a, e.b)
    for (const r of graph.relations) bump(r.from_id, r.to_id)

    // 用户锚：fixed 空间锚点（不连边——轮辐边是「密集成菊花团」的病根）。
    // 布局只由实体间真实关系驱动，围绕锚点自然成形；点击「我」跳画像。
    g.addNode(ME, {
      label: '我',
      size: 7,
      color: theme.current.fg,
      x: 0,
      y: 0,
      fixed: true,
    })
    const saved = loadPositions()
    for (const n of graph.nodes) {
      const d = deg.get(n.id) ?? 0
      const pos = saved.get(n.id)
      g.addNode(n.id, {
        label: n.name,
        size: 4 + Math.min(d * 0.8, 8),
        color: KIND_COLOR[n.kind] ?? theme.current.muted,
        // 有记忆坐标 → 原位复活；否则贴着中心随机散（big bang 开场）
        x: pos?.x ?? Math.random() * 8 - 4,
        y: pos?.y ?? Math.random() * 8 - 4,
      })
    }
    // 边：无向细线、柔和色；同一对只画一条（graphology single-graph 同对 addEdge 会抛错）
    const edgeKey = (a: string, b: string) => (a < b ? `${a}|${b}` : `${b}|${a}`)
    const seen = new Set<string>()
    const addEdge = (a: string, b: string, weight: number, source?: string) => {
      if (!g.hasNode(a) || !g.hasNode(b)) return
      const k = edgeKey(a, b)
      if (seen.has(k)) return
      seen.add(k)
      // 常识边（world_knowledge，收录哲学线层级模型）：LLM 世界知识补的语境边——
      // 半透明细线与记忆边视觉分层；第 2 层实体（backfill 拉入、无记忆挂链）天然止步，
      // BFS≤2 剪枝由数据流性质保证（常识边永不上升级为记忆边）
      const isKnowledge = source === 'world_knowledge'
      g.addEdge(a, b, {
        size: isKnowledge ? Math.min(0.3 + weight * 0.2, 1) : Math.min(0.5 + weight * 0.4, 2),
        color: isKnowledge ? `${theme.current.border}55` : theme.current.border,
        weight,
      })
    }
    for (const e of graph.edges) addEdge(e.a, e.b, e.weight)
    for (const r of graph.relations) addEdge(r.from_id, r.to_id, r.weight, r.source)

    // hover/drag 状态：邻居高亮 + 非邻淡化（Obsidian 标志性交互）
    const state = { hover: null as string | null, drag: null as string | null }
    const neighbors = (id: string) => new Set(g.neighbors(id))

    const sigma = new Sigma(g, el, {
      labelRenderedSizeThreshold: 3,
      labelFont: 'Geist Variable',
      labelColor: { color: theme.current.muted },
      labelWeight: '500',
      defaultEdgeType: 'line',
      renderLabels: true,
      minCameraRatio: 0.2,
      maxCameraRatio: 4,
      allowInvalidContainer: true,
      nodeReducer: (node, data) => {
        const active = state.hover ?? state.drag
        if (!active || node === active) return data
        if (neighbors(active).has(node)) return { ...data, zIndex: 1 }
        // 非邻居：淡化 + 藏标签（焦点感）
        return { ...data, color: `${data.color}33`, label: null, zIndex: 0 }
      },
      edgeReducer: (edge, data) => {
        const active = state.hover ?? state.drag
        if (!active) return data
        const [a, b] = g.extremities(edge)
        if (a === active || b === active) return { ...data, size: Math.max(data.size ?? 1, 1.5) }
        return { ...data, hidden: true }
      },
    })
    sigmaRef.current = sigma

    // ——永续力模拟（活图的核心）：能量模型——
    // energy 高 = 每帧迭代；衰减到低 = 每 10 帧一次待机微颤（视觉静止但活着）；
    // 任何交互把 energy 顶回去。永不彻底停机——「整张网会呼吸」的手感来源。
    let energy = 1
    let frame = 0
    let raf = 0
    const wake = (min = 0.6) => {
      energy = Math.max(energy, min)
    }
    wakeRef.current = wake
    const tick = () => {
      frame += 1
      const idle = energy < 0.12
      if (frame % (idle ? 10 : 1) === 0) {
        forceAtlas2.assign(g, {
          iterations: 1,
          settings: {
            gravity: paramsRef.current.gravity,
            scalingRatio: paramsRef.current.scalingRatio,
            slowDown: paramsRef.current.slowDown,
            barnesHutOptimize: g.order > 300,
            edgeWeightInfluence: 0.3,
          },
        })
        energy *= 0.995
        sigma.refresh({ skipIndexation: true })
      }
      raf = requestAnimationFrame(tick)
    }
    raf = requestAnimationFrame(tick)

    // 位置持久化：能量进入待机时保存一次 + 卸载时保存
    let idleSaved = false
    const saveTick = setInterval(() => {
      if (energy < 0.12 && !idleSaved) {
        savePositions(g)
        idleSaved = true
      } else if (energy >= 0.12) {
        idleSaved = false
      }
    }, 3000)

    // ——拖拽：节点跟手 + 松手回弹（FA2 接管，邻居真实晃动）——
    // 拖拽期间关 captor（相机 pan/缩放暂停）防「拖不动/拖飞」——v3 病根
    let dragNode: string | null = null
    const captor = sigma.getMouseCaptor()
    sigma.on('downNode', (e) => {
      dragNode = e.node
      state.drag = dragNode
      wake(0.8)
      captor.enabled = false
      sigma.refresh({ skipIndexation: true })
    })
    const onMove = (ev: MouseEvent) => {
      if (!dragNode) return
      const rect = el.getBoundingClientRect()
      const pos = sigma.viewportToGraph({ x: ev.clientX - rect.left, y: ev.clientY - rect.top })
      g.setNodeAttribute(dragNode, 'x', pos.x)
      g.setNodeAttribute(dragNode, 'y', pos.y)
      wake(0.5) // 拖动持续供能——邻居被拽着走
      sigma.refresh({ skipIndexation: true })
    }
    const onUp = () => {
      if (dragNode) {
        g.setNodeAttribute(dragNode, 'fixed', false) // 松手：FA2 接管回弹
        wake(0.9) // 松手弹一下
      }
      dragNode = null
      state.drag = null
      captor.enabled = true
      sigma.refresh({ skipIndexation: true })
    }
    el.addEventListener('mousemove', onMove)
    el.addEventListener('mouseup', onUp)

    // hover 高亮：轻唤醒（淡出由 reducer 即时生效）
    sigma.on('enterNode', ({ node }) => {
      state.hover = node
      wake(0.15)
      sigma.refresh({ skipIndexation: true })
    })
    sigma.on('leaveNode', () => {
      state.hover = null
      sigma.refresh({ skipIndexation: true })
    })

    sigma.on('clickNode', ({ node }) => {
      if (node === ME) onGoPersona()
      else onSelect(node)
    })

    return () => {
      savePositions(g)
      clearInterval(saveTick)
      cancelAnimationFrame(raf)
      wakeRef.current = null
      el.removeEventListener('mousemove', onMove)
      el.removeEventListener('mouseup', onUp)
      sigma.kill()
      sigmaRef.current = null
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [graph, themeTick])

  // 滑杆：改 paramsRef + 唤醒模拟（图不重建，拖杆即时反馈）
  const slider = (key: keyof ForceParams, label: string, min: number, max: number, step: number) => (
    <label key={key} className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
      {label}
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        defaultValue={DEFAULT_PARAMS[key]}
        onChange={(e) => {
          paramsRef.current[key] = Number(e.target.value)
          wakeRef.current?.(0.8)
        }}
        className="h-1 w-16 cursor-pointer accent-foreground"
      />
    </label>
  )

  return (
    <div className="relative min-h-0 w-full flex-1">
      <div
        ref={ref}
        role="img"
        aria-label="实体关系图谱：节点为记忆里的实体，连线表示共现关系。按住节点拖动松手回弹，悬停高亮邻居，滚轮缩放，点击节点看档案。"
        tabIndex={0}
        className="min-h-0 h-full w-full rounded-lg border border-border bg-card wiki-graph-canvas"
      />
      {/* 参数面板（Obsidian「力」面板同思路）：三滑杆实时调 */}
      <div className="absolute top-3 right-3 flex flex-col gap-1.5 rounded-lg border border-border bg-card/90 px-3 py-2 shadow-sm backdrop-blur">
        {slider('scalingRatio', '斥力', 2, 40, 1)}
        {slider('gravity', '向心', 0, 8, 0.2)}
        {slider('slowDown', '惯性', 1, 20, 1)}
      </div>
      <div className="absolute bottom-3 right-3 flex gap-2">
        <button
          type="button"
          onClick={() => {
            localStorage.removeItem(POS_KEY)
            wakeRef.current?.(1) // 满能量重新炸开
          }}
          className="rounded-md border border-border bg-card px-2 py-1 text-xs text-muted-foreground shadow-sm transition-colors hover:border-foreground/30 hover:text-foreground"
        >
          重排布局
        </button>
        <button
          type="button"
          onClick={() => sigmaRef.current?.getCamera().animatedReset({ duration: 300 })}
          className="rounded-md border border-border bg-card px-2 py-1 text-xs text-muted-foreground shadow-sm transition-colors hover:border-foreground/30 hover:text-foreground"
        >
          重置视图
        </button>
      </div>
    </div>
  )
}

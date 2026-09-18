/**
 * Wiki 链接图谱 v2：sigma.js 真渲染 + Obsidian 手感（2026-09-18 移植 EntityGalaxy v4 方案）。
 *
 * 病根与对症（用户反馈与圈子页同源）：
 * - 死板 → 原先根本没有力布局（位置 = localStorage 缓存或纯随机撒点）——
 *   现在永续 FA2 活布局（能量模型：交互唤醒、衰减待机微颤），随机初值自动散成合理结构；
 * - 卡 → hover 高亮原先「全图 forEachNode/forEachEdge 改属性 + 全量 refresh」——
 *   O(N+E) 属性写入每动一次鼠标跑一遍。换成 sigma 声明式 reducer（零属性写入，即时生效）；
 * - 手感 → 拖拽松手真实回弹（FA2 接管）、三滑杆实时调参、布局稳定自动记忆。
 *
 * 保留：社区/type 双着色、凝聚度图例、洞察联动高亮（highlightSlugs）、
 * 强边编码（weight≥6 绿色加粗）、缩放控件、点击节点跳页。
 */
import { useEffect, useMemo, useRef, useState } from 'react'
import Graph from 'graphology'
import Sigma from 'sigma'
import { useNavigate } from 'react-router-dom'
import { Maximize, ZoomIn, ZoomOut } from 'lucide-react'
import forceAtlas2 from 'graphology-layout-forceatlas2'
import type { GraphDto } from '@/lib/api'
import { useThemeTick } from '@/lib/theme'
import { Empty, Tabs } from '@/components/ui-bits'

const TYPE_COLOR: Record<string, string> = {
  entity: '#e6772e',
  concept: '#3b82f6',
  source: '#8b5cf6',
  synthesis: '#10b981',
  comparison: '#f59e0b',
  queries: '#06b6d4',
  overview: '#6b7280',
  index: '#374151',
  log: '#374151',
  purpose: '#ec4899',
}

/** 12 色社区调色板（数据编码色，亮暗通用中明度） */
const COMMUNITY_PALETTE = [
  '#ef4444', '#f97316', '#eab308', '#84cc16', '#22c55e', '#14b8a6',
  '#06b6d4', '#3b82f6', '#6366f1', '#a855f7', '#d946ef', '#f43f5e',
]

/** 布局持久化 localStorage key */
const POS_KEY = 'engram-wiki-graph-pos-v2'

/** 可调参数（与 EntityGalaxy 同款三滑杆） */
interface ForceParams {
  scalingRatio: number
  gravity: number
  slowDown: number
}
const DEFAULT_PARAMS: ForceParams = { scalingRatio: 12, gravity: 1, slowDown: 4 }

/** 主题相关色（边/高亮/标签/淡化/强边）从 token 取。 */
function themeColors() {
  const css = getComputedStyle(document.documentElement)
  const v = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback
  return {
    edge: v('--border', '#e5e5e5'),
    edgeHighlighted: v('--muted-foreground', '#636365'),
    edgeStrong: v('--success', '#3dd68c'),
    highlight: v('--foreground', '#0a0a0a'),
    label: v('--foreground', '#0a0a0a'),
  }
}

type ColorMode = 'community' | 'type'

function loadPositions(): Record<string, { x: number; y: number }> {
  try {
    return JSON.parse(localStorage.getItem(POS_KEY) ?? '{}') ?? {}
  } catch {
    return {}
  }
}
function savePositions(g: Graph) {
  try {
    const pos: Record<string, { x: number; y: number }> = {}
    g.forEachNode((slug, attrs) => {
      pos[slug] = { x: attrs.x as number, y: attrs.y as number }
    })
    localStorage.setItem(POS_KEY, JSON.stringify(pos))
  } catch {
    /* 缓存失败不阻塞 */
  }
}

/** 高亮的 slug 集合（洞察卡片点击联动） */
export default function WikiGraph({
  graph,
  highlightSlugs,
}: {
  graph: GraphDto
  highlightSlugs?: string[]
}) {
  const ref = useRef<HTMLDivElement>(null)
  const sigmaRef = useRef<Sigma | null>(null)
  const nav = useNavigate()
  const [mode, setMode] = useState<ColorMode>('community')
  const themeTick = useThemeTick()
  const paramsRef = useRef<ForceParams>({ ...DEFAULT_PARAMS })
  const wakeRef = useRef<(min?: number) => void>(null)

  const nodeColor = useMemo(() => {
    return (pageType: string, community: number) => {
      if (mode === 'type') return TYPE_COLOR[pageType] ?? '#6b7280'
      return COMMUNITY_PALETTE[community % COMMUNITY_PALETTE.length]
    }
  }, [mode])

  useEffect(() => {
    if (!ref.current || graph.nodes.length === 0) return
    const el = ref.current
    const g = new Graph({ multi: false })
    const highlightSet = new Set(highlightSlugs ?? [])
    const slugSet = new Set(graph.nodes.map((n) => n.slug))
    const tc = themeColors()
    // 度数 → 尺寸 + 邻接表（hover reducer 用）
    const degree = new Map<string, number>()
    const neighbors = new Map<string, Set<string>>()
    for (const e of graph.edges) {
      degree.set(e.from_slug, (degree.get(e.from_slug) ?? 0) + 1)
      degree.set(e.to_slug, (degree.get(e.to_slug) ?? 0) + 1)
      if (!neighbors.has(e.from_slug)) neighbors.set(e.from_slug, new Set())
      if (!neighbors.has(e.to_slug)) neighbors.set(e.to_slug, new Set())
      neighbors.get(e.from_slug)!.add(e.to_slug)
      neighbors.get(e.to_slug)!.add(e.from_slug)
    }
    const cached = loadPositions()
    for (const n of graph.nodes) {
      if (!g.hasNode(n.slug)) {
        const highlighted = highlightSet.size > 0 && highlightSet.has(n.slug)
        const deg = degree.get(n.slug) ?? 0
        const pos = cached[n.slug]
        g.addNode(n.slug, {
          label: n.title,
          // 有记忆坐标 → 原位复活；否则贴中心随机散（活布局会自动散成结构）
          x: pos?.x ?? Math.random() * 8 - 4,
          y: pos?.y ?? Math.random() * 8 - 4,
          // 小一号 + 半透明（B3≈70%）：页面多节点密，实色大点会糊
          size: highlighted ? 11 : 3.5 + Math.min(deg * 0.9, 5),
          color: highlighted ? tc.highlight : `${nodeColor(n.page_type, n.community ?? 0)}B3`,
          pageType: n.page_type,
          community: n.community ?? 0,
        })
      }
    }
    for (const e of graph.edges) {
      if (slugSet.has(e.from_slug) && slugSet.has(e.to_slug) && !g.hasEdge(e.from_slug, e.to_slug)) {
        g.addEdge(e.from_slug, e.to_slug, { weight: e.weight })
      }
    }

    // hover/drag 状态（reducer 声明式消费——不再遍历改属性）
    const state = { hover: null as string | null, drag: null as string | null }
    const isNb = (a: string, b: string) => neighbors.get(a)?.has(b) ?? false
    /** 淡化色：截掉可能存在的 alpha 位再拼（节点基础色已带 B3 透明度） */
    const dimColor = (c: string) => `${c.slice(0, 7)}33`

    const renderer = new Sigma(g, el, {
      renderEdgeLabels: false,
      defaultEdgeType: 'line',
      labelRenderedSizeThreshold: 3.5,
      labelFont: "'Geist Variable', sans-serif",
      labelSize: 12,
      labelWeight: '500',
      labelColor: { color: tc.label },
      allowInvalidContainer: true,
      minCameraRatio: 0.2,
      maxCameraRatio: 4,
      nodeReducer: (slug, data) => {
        const active = state.hover ?? state.drag
        if (!active) return data
        if (slug === active || isNb(active, slug)) return { ...data, zIndex: 1 }
        return { ...data, color: dimColor(data.color as string), label: null, zIndex: 0 }
      },
      edgeReducer: (edge, data) => {
        const active = state.hover ?? state.drag
        const weight = (g.getEdgeAttribute(edge, 'weight') as number) ?? 1
        const [from, to] = g.extremities(edge)
        if (!active) {
          // 静息态：细而淡（页面多边多——粗深边会糊成毛线团；强边也只给半透明绿提示）
          return {
            ...data,
            color: weight >= 6 ? `${tc.edgeStrong}77` : `${tc.edge}55`,
            size: 0.15 + Math.min(weight / 6, 1) * 0.9,
          }
        }
        if (from === active || to === active) {
          return { ...data, color: tc.edgeHighlighted, size: Math.max(1.5, data.size ?? 1) }
        }
        return { ...data, hidden: true }
      },
    })
    sigmaRef.current = renderer

    // ——永续力模拟（活图核心；同 EntityGalaxy v4）——
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
        renderer.refresh({ skipIndexation: true })
      }
      raf = requestAnimationFrame(tick)
    }
    raf = requestAnimationFrame(tick)

    let idleSaved = false
    const saveTick = setInterval(() => {
      if (energy < 0.12 && !idleSaved) {
        savePositions(g)
        idleSaved = true
      } else if (energy >= 0.12) {
        idleSaved = false
      }
    }, 3000)

    // ——拖拽：跟手 + 松手回弹（captor-disable 正解同 EntityGalaxy）——
    let dragNode: string | null = null
    const captor = renderer.getMouseCaptor()
    renderer.on('downNode', (e) => {
      dragNode = e.node
      state.drag = dragNode
      wake(0.8)
      captor.enabled = false
      renderer.refresh({ skipIndexation: true })
    })
    const onMove = (ev: MouseEvent) => {
      if (!dragNode) return
      const rect = el.getBoundingClientRect()
      const pos = renderer.viewportToGraph({ x: ev.clientX - rect.left, y: ev.clientY - rect.top })
      g.setNodeAttribute(dragNode, 'x', pos.x)
      g.setNodeAttribute(dragNode, 'y', pos.y)
      wake(0.5)
      renderer.refresh({ skipIndexation: true })
    }
    const onUp = () => {
      if (dragNode) {
        g.setNodeAttribute(dragNode, 'fixed', false)
        wake(0.9)
      }
      dragNode = null
      state.drag = null
      captor.enabled = true
      renderer.refresh({ skipIndexation: true })
    }
    el.addEventListener('mousemove', onMove)
    el.addEventListener('mouseup', onUp)

    renderer.on('enterNode', ({ node }) => {
      state.hover = node
      wake(0.15)
      renderer.refresh({ skipIndexation: true })
    })
    renderer.on('leaveNode', () => {
      state.hover = null
      renderer.refresh({ skipIndexation: true })
    })
    renderer.on('clickNode', ({ node }) => {
      nav(`/wiki?page=${encodeURIComponent(node)}`)
    })

    return () => {
      savePositions(g)
      clearInterval(saveTick)
      cancelAnimationFrame(raf)
      wakeRef.current = null
      el.removeEventListener('mousemove', onMove)
      el.removeEventListener('mouseup', onUp)
      renderer.kill()
      sigmaRef.current = null
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [graph, nav, nodeColor, highlightSlugs, themeTick])

  if (graph.nodes.length === 0) {
    return <Empty text="图谱为空（ingest 后生成）" />
  }

  const communityCount = new Set(graph.nodes.map((n) => n.community ?? 0)).size
  const sparseComms = (graph.communities ?? []).filter((c) => c.sparse)

  const zoomBtn =
    'flex size-7 items-center justify-center rounded-md border border-border bg-card text-muted-foreground shadow-sm transition-colors hover:border-foreground/30 hover:text-foreground'

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
    <div className="flex h-full min-h-[520px] flex-col space-y-3" data-testid="wiki-graph-root">
      <div className="flex flex-wrap items-center gap-3">
        <Tabs
          items={[
            { value: 'community', label: `社区着色（${communityCount} 簇）` },
            { value: 'type', label: '类型着色' },
          ]}
          value={mode}
          onChange={setMode}
        />
        {sparseComms.length > 0 && (
          <span className="text-xs text-warning">
            {sparseComms.length} 个稀疏社区
          </span>
        )}
      </div>
      <div className="relative min-h-0 flex-1">
        <div
          ref={ref}
          role="img"
          aria-label="Wiki 知识图谱：节点为互链页面，连线为链接强度（越粗越强）。活布局：拖拽松手回弹，hover 高亮邻居，滚轮缩放，点击节点跳转页面。"
          tabIndex={0}
          className="wiki-graph-canvas h-full min-h-[480px] w-full rounded-lg border border-border bg-card"
          data-testid="wiki-graph-canvas"
        />
        {/* 参数面板（活布局三滑杆） */}
        <div className="absolute top-3 right-3 flex flex-col gap-1.5 rounded-lg border border-border bg-card/90 px-3 py-2 shadow-sm backdrop-blur">
          {slider('scalingRatio', '斥力', 2, 40, 1)}
          {slider('gravity', '向心', 0, 8, 0.2)}
          {slider('slowDown', '惯性', 1, 20, 1)}
        </div>
        {/* 缩放控件 */}
        <div className="absolute bottom-3 right-3 flex flex-col gap-1">
          <button
            type="button"
            title="放大"
            aria-label="放大"
            className={zoomBtn}
            onClick={() => sigmaRef.current?.getCamera().animatedZoom({ duration: 200, factor: 1.4 })}
          >
            <ZoomIn className="size-3.5" />
          </button>
          <button
            type="button"
            title="缩小"
            aria-label="缩小"
            className={zoomBtn}
            onClick={() => sigmaRef.current?.getCamera().animatedZoom({ duration: 200, factor: 1 / 1.4 })}
          >
            <ZoomOut className="size-3.5" />
          </button>
          <button
            type="button"
            title="适应屏幕"
            aria-label="适应屏幕"
            className={zoomBtn}
            onClick={() => sigmaRef.current?.getCamera().animatedReset({ duration: 300 })}
          >
            <Maximize className="size-3.5" />
          </button>
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
        {mode === 'type'
          ? Object.entries(TYPE_COLOR)
              .filter(([t]) => graph.nodes.some((n) => n.page_type === t))
              .map(([t, c]) => (
                <span key={t} className="flex items-center gap-1.5">
                  <span className="inline-block size-2 rounded-full" style={{ background: c }} />
                  {t}
                </span>
              ))
          : (graph.communities ?? [])
              .filter((c) => graph.nodes.some((n) => (n.community ?? 0) === c.id))
              .map((c) => (
                <span key={c.id} className="flex items-center gap-1.5">
                  <span
                    className="inline-block size-2 rounded-full"
                    style={{ background: COMMUNITY_PALETTE[c.id % COMMUNITY_PALETTE.length] }}
                  />
                  #{c.id}（{graph.nodes.filter((n) => (n.community ?? 0) === c.id).length} 页
                  {c.cohesion > 0 ? `，凝聚 ${c.cohesion.toFixed(2)}` : ''}）
                </span>
              ))}
        <span className="ml-auto">活布局 · 拖拽回弹 · hover 高亮 · 点击跳转</span>
      </div>
    </div>
  )
}

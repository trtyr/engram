/**
 * Wiki 链接图谱：sigma.js 真渲染（对齐 llm_wiki，Obsidian 图视图）。
 * 社区/type 双模式着色 + 凝聚度图例 + 洞察节点高亮。
 * Obsidian 化交互（wiki-audit P1/P2 全落地）：
 *   - hover 邻居高亮（非邻居淡化）
 *   - 节点拖拽（captor-disable，位置落 localStorage 防布局跳动）
 *   - 缩放控件（放大/缩小/适应屏幕）
 *   - 边按 weight 编码粗细与颜色（强边更粗更亮）
 */
import { useEffect, useMemo, useRef, useState } from 'react'
import Graph from 'graphology'
import Sigma from 'sigma'
import { useNavigate } from 'react-router-dom'
import { Maximize, ZoomIn, ZoomOut } from 'lucide-react'
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

/** 拖拽后节点位置缓存的 localStorage key */
const POS_KEY = 'engram-wiki-graph-pos'

/** 主题相关色（边/高亮/标签/淡化/强边）从 token 取。 */
function themeColors() {
  const css = getComputedStyle(document.documentElement)
  const v = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback
  return {
    edge: v('--border', '#e5e5e5'),
    edgeHighlighted: v('--muted-foreground', '#636365'),
    edgeStrong: v('--success', '#3dd68c'),
    dim: v('--muted', '#f4f4f4'),
    highlight: v('--foreground', '#0a0a0a'),
    label: v('--foreground', '#0a0a0a'),
    grid: v('--border', '#e5e5e5'),
  }
}

type ColorMode = 'community' | 'type'

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

  const nodeColor = useMemo(() => {
    return (pageType: string, community: number) => {
      if (mode === 'type') return TYPE_COLOR[pageType] ?? '#6b7280'
      return COMMUNITY_PALETTE[community % COMMUNITY_PALETTE.length]
    }
  }, [mode])

  useEffect(() => {
    if (!ref.current || graph.nodes.length === 0) return
    const el = ref.current
    const g = new Graph<Record<string, unknown>, Record<string, unknown>>({ multi: false })
    const highlightSet = new Set(highlightSlugs ?? [])
    const slugSet = new Set(graph.nodes.map((n) => n.slug))
    // 度数 → 尺寸（重要性编码：连接越多节点越大，标签越可见）
    const degree = new Map<string, number>()
    // 邻接表（hover 邻居高亮用）
    const neighbors = new Map<string, Set<string>>()
    for (const e of graph.edges) {
      degree.set(e.from_slug, (degree.get(e.from_slug) ?? 0) + 1)
      degree.set(e.to_slug, (degree.get(e.to_slug) ?? 0) + 1)
      if (!neighbors.has(e.from_slug)) neighbors.set(e.from_slug, new Set())
      if (!neighbors.has(e.to_slug)) neighbors.set(e.to_slug, new Set())
      neighbors.get(e.from_slug)!.add(e.to_slug)
      neighbors.get(e.to_slug)!.add(e.from_slug)
    }
    const tc = themeColors()
    // 位置缓存：拖拽后记住，刷新不再跳
    let cached: Record<string, { x: number; y: number }> = {}
    try {
      cached = JSON.parse(localStorage.getItem(POS_KEY) ?? '{}') ?? {}
    } catch {
      cached = {}
    }
    for (const n of graph.nodes) {
      if (!g.hasNode(n.slug)) {
        const highlighted = highlightSet.size > 0 && highlightSet.has(n.slug)
        const deg = degree.get(n.slug) ?? 0
        const pos = cached[n.slug]
        g.addNode(n.slug, {
          label: n.title,
          x: pos?.x ?? Math.random() * 100,
          y: pos?.y ?? Math.random() * 100,
          size: highlighted ? 13 : 5 + Math.min(deg * 1.4, 8),
          color: highlighted ? tc.highlight : nodeColor(n.page_type, n.community ?? 0),
          nodeType: n.page_type,
          community: n.community ?? 0,
        })
      }
    }
    // 边 weight 编码：强边（直接链接/同源）更粗、success 绿；弱边细灰
    const edgeColor = (weight: number, highlighted: boolean) => {
      if (highlighted) return tc.edgeHighlighted
      return weight >= 6 ? tc.edgeStrong : tc.edge
    }
    const edgeSize = (weight: number, highlighted: boolean) => {
      if (highlighted) return 3
      return 0.5 + Math.min(weight / 6, 1) * 2.5
    }
    for (const e of graph.edges) {
      if (slugSet.has(e.from_slug) && slugSet.has(e.to_slug) && !g.hasEdge(e.from_slug, e.to_slug)) {
        const highlighted = highlightSet.size > 0 && highlightSet.has(e.from_slug) && highlightSet.has(e.to_slug)
        g.addEdge(e.from_slug, e.to_slug, {
          color: edgeColor(e.weight, highlighted),
          size: edgeSize(e.weight, highlighted),
          weight: e.weight,
        })
      }
    }
    const renderer = new Sigma(g, el, {
      renderEdgeLabels: false,
      defaultEdgeType: 'line',
      labelRenderedSizeThreshold: 3.5,
      labelFont: "'Geist Variable', sans-serif",
      labelSize: 12,
      labelWeight: '500',
      labelColor: { color: tc.label },
    })
    sigmaRef.current = renderer
    renderer.on('clickNode', ({ node }) => {
      nav(`/wiki?page=${encodeURIComponent(node)}`)
    })

    // —— hover 邻居高亮：悬停节点时，邻居保持原色，其余淡化 ——
    renderer.on('enterNode', ({ node }) => {
      const nb = neighbors.get(node) ?? new Set<string>()
      g.forEachNode((slug, attrs) => {
        const keep = slug === node || nb.has(slug)
        g.setNodeAttribute(
          slug,
          'color',
          keep
            ? highlightSet.size > 0 && highlightSet.has(slug)
              ? tc.highlight
              : nodeColor(attrs.nodeType as string, attrs.community as number)
            : tc.dim,
        )
      })
      g.forEachEdge((_, _attrs, from, to) => {
        const keep = from === node || to === node || nb.has(from) || nb.has(to)
        g.setEdgeAttribute(_, 'color', keep ? tc.edgeHighlighted : tc.dim)
        g.setEdgeAttribute(_, 'size', keep ? 2.5 : 0.4)
      })
      if (el) el.style.cursor = 'pointer'
    })
    renderer.on('leaveNode', () => {
      g.forEachNode((slug, attrs) => {
        g.setNodeAttribute(
          slug,
          'color',
          highlightSet.size > 0 && highlightSet.has(slug)
            ? tc.highlight
            : nodeColor(attrs.nodeType as string, attrs.community as number),
        )
      })
      g.forEachEdge((_, attrs) => {
        g.setEdgeAttribute(_, 'color', edgeColor(attrs.weight as number, false))
        g.setEdgeAttribute(_, 'size', edgeSize(attrs.weight as number, false))
      })
      if (el) el.style.cursor = 'default'
    })

    // —— 节点拖拽：按住圆点移动，期间关掉相机 captor（v3 正解，见 EntityGalaxy）——
    let dragNode: string | null = null
    const captor = renderer.getMouseCaptor()
    renderer.on('downNode', (e) => {
      dragNode = e.node
      captor.enabled = false
    })
    const onMove = (ev: MouseEvent) => {
      if (!dragNode) return
      const rect = el.getBoundingClientRect()
      const pos = renderer.viewportToGraph({ x: ev.clientX - rect.left, y: ev.clientY - rect.top })
      g.setNodeAttribute(dragNode, 'x', pos.x)
      g.setNodeAttribute(dragNode, 'y', pos.y)
      renderer.refresh({ skipIndexation: true })
    }
    const onUp = () => {
      dragNode = null
      captor.enabled = true
      const pos: Record<string, { x: number; y: number }> = {}
      g.forEachNode((slug, attrs) => {
        pos[slug] = { x: attrs.x as number, y: attrs.y as number }
      })
      try {
        localStorage.setItem(POS_KEY, JSON.stringify(pos))
      } catch {
        /* 缓存失败不阻塞交互 */
      }
    }
    el.addEventListener('mousemove', onMove)
    el.addEventListener('mouseup', onUp)

    return () => {
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

  return (
    <div className="space-y-3" data-testid="wiki-graph-root">
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
      <div className="relative">
        <div
          ref={ref}
          role="img"
          aria-label="Wiki 知识图谱：节点为互链页面，连线为链接强度（越粗越强）。hover 高亮邻居，按住节点可拖动，滚轮缩放，点击节点跳转页面。"
          tabIndex={0}
          className="wiki-graph-canvas h-[480px] w-full rounded-lg border border-border bg-card"
          data-testid="wiki-graph-canvas"
        />
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
        <span className="ml-auto">节点大小 = 连接数 · 边越粗越强 · hover 高亮邻居 · 点击跳转</span>
      </div>
    </div>
  )
}

/**
 * Wiki 链接图谱：sigma.js 真渲染（对齐 llm_wiki）。
 * 社区/type 双模式着色 + 凝聚度图例 + 洞察节点高亮。
 */
import { useEffect, useMemo, useRef, useState } from 'react'
import Graph from 'graphology'
import Sigma from 'sigma'
import { useNavigate } from 'react-router-dom'
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

/** 主题相关色（边/高亮/标签）从 token 取。 */
function themeColors() {
  const css = getComputedStyle(document.documentElement)
  const v = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback
  return {
    edge: v('--border', '#e5e5e5'),
    edgeHighlighted: v('--muted-foreground', '#636365'),
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
    const g = new Graph<Record<string, unknown>, Record<string, unknown>>({ multi: false })
    const highlightSet = new Set(highlightSlugs ?? [])
    const slugSet = new Set(graph.nodes.map((n) => n.slug))
    // 度数 → 尺寸（重要性编码：连接越多节点越大，标签越可见）
    const degree = new Map<string, number>()
    for (const e of graph.edges) {
      degree.set(e.from_slug, (degree.get(e.from_slug) ?? 0) + 1)
      degree.set(e.to_slug, (degree.get(e.to_slug) ?? 0) + 1)
    }
    const tc = themeColors()
    for (const n of graph.nodes) {
      if (!g.hasNode(n.slug)) {
        const highlighted = highlightSet.size > 0 && highlightSet.has(n.slug)
        const deg = degree.get(n.slug) ?? 0
        g.addNode(n.slug, {
          label: n.title,
          x: Math.random() * 100,
          y: Math.random() * 100,
          size: highlighted ? 13 : 5 + Math.min(deg * 1.4, 8),
          color: highlighted ? tc.highlight : nodeColor(n.page_type, n.community ?? 0),
          nodeType: n.page_type,
        })
      }
    }
    for (const e of graph.edges) {
      if (slugSet.has(e.from_slug) && slugSet.has(e.to_slug) && !g.hasEdge(e.from_slug, e.to_slug)) {
        const highlighted = highlightSet.size > 0 && highlightSet.has(e.from_slug) && highlightSet.has(e.to_slug)
        g.addEdge(e.from_slug, e.to_slug, {
          color: highlighted ? tc.edgeHighlighted : tc.edge,
          size: highlighted ? 3 : 1,
        })
      }
    }
    const renderer = new Sigma(g, ref.current, {
      renderEdgeLabels: false,
      defaultEdgeType: 'line',
      labelRenderedSizeThreshold: 3.5,
      labelFont: "'Geist Variable', sans-serif",
      labelSize: 12,
      labelWeight: '500',
      labelColor: { color: tc.label },
    })
    renderer.on('clickNode', ({ node }) => {
      nav(`/wiki?page=${encodeURIComponent(node)}`)
    })
    renderer.on('enterNode', () => {
      if (ref.current) ref.current.style.cursor = 'pointer'
    })
    renderer.on('leaveNode', () => {
      if (ref.current) ref.current.style.cursor = 'default'
    })
    return () => renderer.kill()
  }, [graph, nav, nodeColor, highlightSlugs, themeTick])

  if (graph.nodes.length === 0) {
    return <Empty text="图谱为空（ingest 后生成）" />
  }

  const communityCount = new Set(graph.nodes.map((n) => n.community ?? 0)).size
  const sparseComms = (graph.communities ?? []).filter((c) => c.sparse)

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
      <div
        ref={ref}
        className="wiki-graph-canvas h-[480px] w-full rounded-lg border border-border bg-card"
        data-testid="wiki-graph-canvas"
      />
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
        <span className="ml-auto">节点大小 = 连接数 · 点击节点跳转页面</span>
      </div>
    </div>
  )
}

/**
 * Wiki 链接图谱：sigma.js 真渲染（对齐 llm_wiki）。
 * 社区/type 双模式着色 + 凝聚度图例 + 洞察节点高亮。
 */
import { useEffect, useMemo, useRef, useState } from 'react'
import Graph from 'graphology'
import Sigma from 'sigma'
import { useNavigate } from 'react-router-dom'
import type { GraphDto } from '@/lib/api'
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

/** 12 色社区调色板（llm_wiki 对齐） */
const COMMUNITY_PALETTE = [
  '#ef4444', '#f97316', '#eab308', '#84cc16', '#22c55e', '#14b8a6',
  '#06b6d4', '#3b82f6', '#6366f1', '#a855f7', '#d946ef', '#f43f5e',
]

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
    for (const n of graph.nodes) {
      if (!g.hasNode(n.slug)) {
        const highlighted = highlightSet.size > 0 && highlightSet.has(n.slug)
        g.addNode(n.slug, {
          label: n.title,
          x: Math.random() * 100,
          y: Math.random() * 100,
          size: highlighted ? 12 : 6,
          color: highlighted ? '#ffffff' : nodeColor(n.page_type, n.community ?? 0),
          nodeType: n.page_type,
        })
      }
    }
    for (const e of graph.edges) {
      if (slugSet.has(e.from_slug) && slugSet.has(e.to_slug) && !g.hasEdge(e.from_slug, e.to_slug)) {
        const highlighted = highlightSet.size > 0 && highlightSet.has(e.from_slug) && highlightSet.has(e.to_slug)
        g.addEdge(e.from_slug, e.to_slug, {
          color: highlighted ? '#ffffff' : '#4b5563aa',
          size: highlighted ? 3 : 1.2,
        })
      }
    }
    const renderer = new Sigma(g, ref.current, {
      renderEdgeLabels: false,
      defaultEdgeType: 'line',
      labelRenderedSizeThreshold: 8,
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
  }, [graph, nav, nodeColor, highlightSlugs])

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
          <span className="text-xs text-orange-400">
            {sparseComms.length} 个稀疏社区
          </span>
        )}
      </div>
      <div ref={ref} className="h-[480px] w-full rounded-xl border border-border bg-card" data-testid="wiki-graph-canvas" />
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
        <span className="ml-auto">点击节点跳转页面</span>
      </div>
    </div>
  )
}

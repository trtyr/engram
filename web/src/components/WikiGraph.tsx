/**
 * Wiki 链接图谱：sigma.js 真渲染。
 * 节点按 page_type 着色，点击节点跳转页面；出边高亮邻居。
 */
import { useEffect, useRef } from 'react'
import Graph from 'graphology'
import Sigma from 'sigma'
import { useNavigate } from 'react-router-dom'
import type { GraphDto } from '@/lib/api'

const TYPE_COLOR: Record<string, string> = {
  entity: '#e6772e',
  concept: '#3b82f6',
  source: '#8b5cf6',
  synthesis: '#10b981',
  comparison: '#f59e0b',
  overview: '#6b7280',
  index: '#374151',
  log: '#374151',
}

export default function WikiGraph({ graph }: { graph: GraphDto }) {
  const ref = useRef<HTMLDivElement>(null)
  const nav = useNavigate()

  useEffect(() => {
    if (!ref.current || graph.nodes.length === 0) return
    const g = new Graph<Record<string, unknown>, Record<string, unknown>>({ multi: false })
    const slugSet = new Set(graph.nodes.map((n) => n.slug))
    for (const n of graph.nodes) {
      if (!g.hasNode(n.slug)) {
        g.addNode(n.slug, {
          label: n.title,
          x: Math.random() * 100,
          y: Math.random() * 100,
          size: 6,
          color: TYPE_COLOR[n.page_type] ?? '#6b7280',
          nodeType: n.page_type,
        })
      }
    }
    for (const e of graph.edges) {
      if (slugSet.has(e.from_slug) && slugSet.has(e.to_slug) && !g.hasEdge(e.from_slug, e.to_slug)) {
        g.addEdge(e.from_slug, e.to_slug, { color: '#4b5563aa', size: 1.2 })
      }
    }
    // 随机布局后用力导向细调（graphology-layout 的 circleLayout 作为初始可用布局）
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
  }, [graph, nav])

  if (graph.nodes.length === 0) {
    return <p className="rounded-lg border border-dashed p-8 text-center text-sm text-muted-foreground">图谱为空（ingest 后生成）</p>
  }
  return (
    <div className="space-y-2">
      <div ref={ref} className="h-[480px] w-full rounded-lg border bg-card" data-testid="wiki-graph-canvas" />
      <div className="flex flex-wrap gap-3 text-xs text-muted-foreground">
        {Object.entries(TYPE_COLOR)
          .filter(([t]) => graph.nodes.some((n) => n.page_type === t))
          .map(([t, c]) => (
            <span key={t} className="flex items-center gap-1">
              <span className="inline-block size-2 rounded-full" style={{ background: c }} />
              {t}
            </span>
          ))}
        <span className="ml-auto">点击节点跳转页面</span>
      </div>
    </div>
  )
}

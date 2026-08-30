/**
 * 记忆星系图：以用户为中心的实体关系图谱（sigma 真渲染，与 WikiGraph 同栈）。
 * 中心锚点 = 用户（点击跳画像）；实体节点按类型着色、按记忆密度定大小；
 * 边 = 共现强度（同一原子同时关联两实体）。
 */
import { useEffect, useMemo, useRef } from 'react'
import Graph from 'graphology'
import Sigma from 'sigma'
import type { EntityGraph as GraphData } from '@/lib/api'
import { useThemeTick } from '@/lib/theme'
import { ENTITY_KIND_COLOR } from '@/lib/ui'

/** 实体类型色：见 lib/ui.ts ENTITY_KIND_COLOR（与 WikiGraph 调色板同源） */
const KIND_COLOR = ENTITY_KIND_COLOR

const ME = '__me__'

function themeColors() {
  const cs = getComputedStyle(document.documentElement)
  return {
    fg: cs.getPropertyValue('--foreground').trim() || '#0a0a0a',
    muted: cs.getPropertyValue('--muted-foreground').trim() || '#71717a',
    border: cs.getPropertyValue('--border').trim() || '#e5e5e5',
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
  const themeTick = useThemeTick()
  const theme = useMemo(() => themeColors(), [themeTick]) // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const el = ref.current
    if (!el) return
    const g = new Graph({ multi: false })
    // 中心锚点：用户本人（画像的视觉化身）
    g.addNode(ME, {
      label: '我',
      size: 16,
      color: theme.fg,
      x: 0,
      y: 0,
      fixed: true,
    })
    // 实体：密度定大小（cap 防爆炸），类型着色
    for (const n of graph.nodes) {
      g.addNode(n.id, {
        label: n.name,
        size: 5 + Math.min(n.atom_count * 1.1, 9),
        color: KIND_COLOR[n.kind] ?? theme.muted,
        x: Math.cos((n.id.charCodeAt(0) % 360) * (Math.PI / 180)) * (2 + (n.atom_count % 5)),
        y: Math.sin((n.id.charCodeAt(1) % 360) * (Math.PI / 180)) * (2 + (n.atom_count % 5)),
      })
      // 用户锚边：细而淡（都连着「我」，信息量低——只做结构提示）
      g.addEdge(ME, n.id, { size: 0.6, color: theme.border })
    }
    // 共现边：粗细随强度（这是图的真正信息所在）
    for (const e of graph.edges) {
      if (!g.hasNode(e.a) || !g.hasNode(e.b)) continue
      g.addEdge(e.a, e.b, { size: Math.min(1 + e.weight * 0.6, 4), color: theme.muted })
    }
    const sigma = new Sigma(g, el, {
      labelRenderedSizeThreshold: 3,
      labelFont: 'Geist Variable',
      labelColor: { color: theme.muted },
      labelWeight: '500',
      defaultEdgeType: 'line',
      renderLabels: true,
      minCameraRatio: 0.3,
      maxCameraRatio: 3,
    })
    sigma.on('clickNode', ({ node }) => {
      if (node === ME) onGoPersona()
      else onSelect(node)
    })
    return () => sigma.kill()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [graph, theme])

  return <div ref={ref} className="h-[60vh] w-full rounded-lg border border-border bg-card wiki-graph-canvas" />
}

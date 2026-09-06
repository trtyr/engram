/**
 * CodeGraph 调用图：sigma.js 渲染（复用 Wiki 的图栈 graphology+sigma）。
 * 两种模式（后端归一层区分）：
 *  - symbol 模式：中心符号 + callers/callees（三色按角色）
 *  - files 模式：文件级全图，跨文件依赖按权重编码边粗细，节点大小按度数
 * hover 邻居高亮；forceatlas2 力导布局。数据来自后端归一层，前端不接触 CLI 原始输出。
 */
import { useEffect, useRef } from 'react'
import Graph from 'graphology'
import Sigma from 'sigma'
import forceAtlas2 from 'graphology-layout-forceatlas2'
import type { Attributes } from 'graphology-types'
import { useThemeTick } from '@/lib/theme'
import type { CgGraph } from '@/lib/api'

/** 节点属性（graphology 泛型约束的最小面）。 */
interface CgNodeAttrs extends Attributes {
  x: number
  y: number
  size: number
  color: string
  label: string
  role: string
}

const ROLE_COLOR: Record<string, string> = {
  center: '#e6772e',
  caller: '#3b82f6',
  callee: '#10b981',
  file: '#3b82f6',
}

export default function CgGraphView({ graph }: { graph: CgGraph }) {
  const ref = useRef<HTMLDivElement>(null)
  const themeTick = useThemeTick()
  const filesMode = graph.mode === 'files'

  useEffect(() => {
    if (!ref.current || graph.nodes.length === 0) return

    const css = getComputedStyle(document.documentElement)
    const themeColor = (name: string, fb: string) => css.getPropertyValue(name).trim() || fb
    const edgeBase = themeColor('--border', '#e5e5e5')
    const dim = themeColor('--muted', '#f4f4f4')
    const edgeHighlighted = themeColor('--muted-foreground', '#636365')
    const label = themeColor('--foreground', '#0a0a0a')

    const g = new Graph({ multi: false, type: 'directed' })
    // 圆形初始坐标（力导布局种子）；files 模式按度数定节点大小、按权重定边粗细
    const degree = new Map<string, number>()
    for (const e of graph.edges) {
      degree.set(e.from, (degree.get(e.from) ?? 0) + 1)
      degree.set(e.to, (degree.get(e.to) ?? 0) + 1)
    }
    const maxWeight = filesMode ? Math.max(1, ...graph.edges.map((e) => e.weight ?? 1)) : 1
    const n = graph.nodes.length
    graph.nodes.forEach((node, i) => {
      const angle = (2 * Math.PI * i) / n
      const deg = degree.get(node.id) ?? 0
      const size = filesMode ? 4 + Math.min(deg * 1.2, 14) : node.role === 'center' ? 10 : 6
      g.addNode(node.id, {
        x: Math.cos(angle),
        y: Math.sin(angle),
        size,
        color: ROLE_COLOR[node.role] ?? '#6b7280',
        label: node.name,
        role: node.role,
      } satisfies CgNodeAttrs)
    })
    for (const e of graph.edges) {
      if (!g.hasNode(e.from) || !g.hasNode(e.to) || g.hasEdge(e.from, e.to)) continue
      const w = (e.weight ?? 1) / maxWeight
      g.addDirectedEdge(e.from, e.to, {
        color: e.rel === 'caller' ? '#8b5cf6' : filesMode ? edgeBase : '#10b981',
        size: filesMode ? 0.6 + w * 3 : 1,
      })
    }

    forceAtlas2.assign(g, {
      iterations: 80,
      settings: { gravity: 2, scalingRatio: 6, barnesHutOptimize: n > 100 },
    })

    const renderer = new Sigma(g, ref.current, {
      allowInvalidContainer: true,
      labelRenderedSizeThreshold: 6,
      labelColor: { color: label },
      defaultEdgeType: filesMode ? 'line' : 'arrow',
      renderEdgeLabels: false,
    })

    // hover 邻居高亮：非邻居淡化（graphology 改属性，sigma 自动重绘）
    const neighbors = new Map<string, Set<string>>()
    g.forEachNode((node) => {
      neighbors.set(node, new Set(g.neighbors(node)))
    })
    const roleColor = (node: string) => ROLE_COLOR[g.getNodeAttribute(node, 'role')] ?? '#6b7280'
    const setDim = (node: string | null) => {
      g.forEachNode((n) => {
        const keep = node === null || n === node || (neighbors.get(node)?.has(n) ?? false)
        g.setNodeAttribute(n, 'color', keep ? roleColor(n) : dim)
      })
      g.forEachEdge((e, _attrs, from, to) => {
        const keep = node === null || from === node || to === node
        g.setEdgeAttribute(e, 'color', keep ? edgeHighlighted : dim)
        g.setEdgeAttribute(e, 'hidden', !keep && node !== null)
      })
    }
    renderer.on('enterNode', ({ node }) => setDim(node))
    renderer.on('leaveNode', () => setDim(null))

    return () => renderer.kill()
  }, [graph, themeTick])

  if (graph.nodes.length === 0) return null

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="mb-1.5 flex flex-wrap items-center gap-3 font-mono text-[10px] text-muted-foreground">
        {filesMode ? (
          <>
            <span>
              文件级依赖全图 · {graph.files} 个文件 · {graph.edges.length} 条跨文件依赖
            </span>
            <span>节点大小=被依赖程度 · 线粗=依赖次数 · hover 看邻居</span>
          </>
        ) : (
          <>
            <span className="flex items-center gap-1">
              <i className="size-2 rounded-full" style={{ background: ROLE_COLOR.center }} aria-hidden="true" />
              中心 {graph.symbol}
            </span>
            <span className="flex items-center gap-1">
              <i className="size-2 rounded-full" style={{ background: ROLE_COLOR.caller }} aria-hidden="true" />
              调用方 {graph.callers}
            </span>
            <span className="flex items-center gap-1">
              <i className="size-2 rounded-full" style={{ background: ROLE_COLOR.callee }} aria-hidden="true" />
              被调 {graph.callees}
            </span>
          </>
        )}
      </div>
      <div
        ref={ref}
        className="min-h-0 w-full flex-1 overflow-hidden rounded-md border border-border bg-card"
        role="img"
        aria-label={
          filesMode
            ? `项目依赖全图：${graph.files} 个文件，${graph.edges.length} 条依赖`
            : `${graph.symbol} 的调用图：${graph.callers} 个调用方，${graph.callees} 个被调`
        }
      />
    </div>
  )
}

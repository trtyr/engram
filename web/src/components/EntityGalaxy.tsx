/**
 * 圈子图（记忆星系）：以用户为中心的实体关系图谱（sigma 真渲染，与 WikiGraph 同栈）。
 * 力导向自组织（共现权重参与聚拢——一起出现多的实体真的抱团）+ 节点可拖拽。
 * 中心锚点 = 用户（fixed，点击跳画像）；实体节点按类型着色、按记忆密度定大小；
 * 边 = 共现强度（同一原子同时关联两实体）。
 */
import { useEffect, useMemo, useRef } from 'react'
import Graph from 'graphology'
import Sigma from 'sigma'
import forceAtlas2 from 'graphology-layout-forceatlas2'
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
  const sigmaRef = useRef<Sigma | null>(null)
  const themeTick = useThemeTick()
  const theme = useMemo(() => themeColors(), [themeTick]) // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const el = ref.current
    if (!el) return
    const g = new Graph({ multi: false })
    // 度数统计：共现 + 关系都算连接——Obsidian 图谱里连接多的枢纽自然更大
    const deg = new Map<string, number>()
    const bump = (a: string, b: string) => {
      deg.set(a, (deg.get(a) ?? 0) + 1)
      deg.set(b, (deg.get(b) ?? 0) + 1)
    }
    for (const e of graph.edges) bump(e.a, e.b)
    for (const r of graph.relations) bump(r.from_id, r.to_id)
    // 用户锚：普通节点（不 fixed，让力导向自然布局——Obsidian 没有固定中心）
    g.addNode(ME, {
      label: '我',
      size: 6,
      color: theme.fg,
      x: 0,
      y: 0,
    })
    // 实体：Obsidian 风格——小圆点，按度数定大小（枢纽更大），初始随机散布自然聚拢
    for (const n of graph.nodes) {
      const d = deg.get(n.id) ?? 0
      g.addNode(n.id, {
        label: n.name,
        size: 4 + Math.min(d * 0.8, 8),
        color: KIND_COLOR[n.kind] ?? theme.muted,
        x: Math.random() * 24 - 12,
        y: Math.random() * 24 - 12,
      })
      // 我-实体边：细而淡（都连着「我」，信息量低——只做结构提示）
      g.addEdge(ME, n.id, { size: 0.5, color: theme.border, weight: 0.1 })
    }
    // 边：Obsidian 风格——无向细线，统一柔和色（不区分箭头/关系色），
    // 同一对节点只画一条（graphology single-graph 下同对 addEdge 会抛错）
    const edgeKey = (a: string, b: string) => (a < b ? `${a}|${b}` : `${b}|${a}`)
    const seen = new Set<string>()
    for (const e of graph.edges) {
      if (!g.hasNode(e.a) || !g.hasNode(e.b)) continue
      const k = edgeKey(e.a, e.b)
      if (seen.has(k)) continue
      seen.add(k)
      g.addEdge(e.a, e.b, { size: Math.min(1 + e.weight * 0.4, 2.5), color: theme.border, weight: e.weight })
    }
    for (const r of graph.relations) {
      if (!g.hasNode(r.from_id) || !g.hasNode(r.to_id)) continue
      const k = edgeKey(r.from_id, r.to_id)
      if (seen.has(k)) continue
      seen.add(k)
      g.addEdge(r.from_id, r.to_id, { size: Math.min(1 + r.weight * 0.4, 2.5), color: theme.border, weight: r.weight })
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
    sigmaRef.current = sigma

    // 力导向自组织：约 2.5 秒的活布局（重力收拢 + 共现抱团），然后定格。
    // 「我」fixed 居中，实体围绕成簇——图不再是一张死画。
    let running = true
    const start = performance.now()
    const tick = () => {
      if (!running) return
      forceAtlas2.assign(g, {
        iterations: 3,
        settings: {
          gravity: 4,
          scalingRatio: 10,
          slowDown: 6,
          barnesHutOptimize: true,
          edgeWeightInfluence: 0.5,
        },
      })
      sigma.refresh()
      if (performance.now() - start < 2500) requestAnimationFrame(tick)
    }
    requestAnimationFrame(tick)

    // 节点拖拽：按住圆点即可挪动。
    // 病根（v3 实测）：按下节点时 sigma 相机的 stage-pan 同时启动——节点位移与
    // 相机平移相互抵消，视觉上"拖不动/拖飞"。v3 已删 body 鼠标事件与
    // preventSigmaDefault，正解是拖拽期间整体关掉鼠标 captor（相机 pan/缩放
    // 暂停），松手恢复——节点精确跟手。
    let dragNode: string | null = null
    const captor = sigma.getMouseCaptor()
    sigma.on('downNode', (e) => {
      dragNode = e.node
      g.setNodeAttribute(dragNode, 'highlighted', true)
      captor.enabled = false
    })
    const onMove = (ev: MouseEvent) => {
      if (!dragNode) return
      const rect = el.getBoundingClientRect()
      const pos = sigma.viewportToGraph({ x: ev.clientX - rect.left, y: ev.clientY - rect.top })
      g.setNodeAttribute(dragNode, 'x', pos.x)
      g.setNodeAttribute(dragNode, 'y', pos.y)
      sigma.refresh({ skipIndexation: true })
    }
    const onUp = () => {
      if (dragNode) g.removeNodeAttribute(dragNode, 'highlighted')
      dragNode = null
      captor.enabled = true
    }
    el.addEventListener('mousemove', onMove)
    el.addEventListener('mouseup', onUp)

    sigma.on('clickNode', ({ node }) => {
      if (node === ME) onGoPersona()
      else onSelect(node)
    })
    return () => {
      running = false
      el.removeEventListener('mousemove', onMove)
      el.removeEventListener('mouseup', onUp)
      sigma.kill()
      sigmaRef.current = null
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [graph, theme])

  return (
    <div className="relative min-h-0 w-full flex-1">
      <div
        ref={ref}
        role="img"
        aria-label="实体关系图谱：节点为记忆里的实体，连线表示共现关系。按住节点可拖动，滚轮缩放，点击节点看档案。"
        tabIndex={0}
        className="min-h-0 h-full w-full rounded-lg border border-border bg-card wiki-graph-canvas"
      />
      <button
        type="button"
        onClick={() => sigmaRef.current?.getCamera().animatedReset({ duration: 300 })}
        className="absolute bottom-3 right-3 rounded-md border border-border bg-card px-2 py-1 text-xs text-muted-foreground shadow-sm transition-colors hover:border-foreground/30 hover:text-foreground"
      >
        重置视图
      </button>
    </div>
  )
}

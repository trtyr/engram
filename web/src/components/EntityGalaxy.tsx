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
import { ENTITY_KIND_COLOR, REL_TYPE_COLOR } from '@/lib/ui'

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
    // 中心锚点：用户本人（画像的视觉化身；fixed = 力导向中稳居中央）
    g.addNode(ME, {
      label: '我',
      size: 16,
      color: theme.fg,
      x: 0,
      y: 0,
      fixed: true,
    })
    // 实体：密度定大小（cap 防爆炸），类型着色。社区聚类的轻量实现——
    // 同 kind 节点初始落在同一扇区（人物/项目/主题/群组/地点各占一角），
    // 力导向后同类型自然聚拢，实体多时不再是一团乱麻。
    const kindOrder = ['person', 'project', 'topic', 'group', 'place']
    const sectorAngle = (2 * Math.PI) / kindOrder.length
    for (const n of graph.nodes) {
      const sector = kindOrder.indexOf(n.kind)
      const base = sector >= 0 ? sector * sectorAngle : 0
      const r = 6 + Math.random() * 4
      const a = base + Math.random() * sectorAngle
      g.addNode(n.id, {
        label: n.name,
        size: 5 + Math.min(n.atom_count * 1.1, 9),
        color: KIND_COLOR[n.kind] ?? theme.muted,
        x: Math.cos(a) * r,
        y: Math.sin(a) * r,
      })
      // 用户锚边：细而淡（都连着「我」，信息量低——只做结构提示）
      g.addEdge(ME, n.id, { size: 0.6, color: theme.border, weight: 0.2 })
    }
    // 共现边：粗细随强度（这是图的真正信息所在）；weight 参与力导向——常一起出现的实体聚拢
    for (const e of graph.edges) {
      if (!g.hasNode(e.a) || !g.hasNode(e.b)) continue
      g.addEdge(e.a, e.b, { size: Math.min(1 + e.weight * 0.6, 4), color: theme.muted, weight: e.weight })
    }
    // 类型化关系边：有向箭头 + 关系色，与共现边（细灰线）区分——图升级成知识图谱
    for (const r of graph.relations) {
      if (!g.hasNode(r.from_id) || !g.hasNode(r.to_id)) continue
      g.addEdge(r.from_id, r.to_id, {
        size: Math.min(1 + r.weight * 0.6, 3),
        color: REL_TYPE_COLOR[r.rel_type] ?? theme.muted,
        type: 'arrow',
        weight: r.weight,
      })
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

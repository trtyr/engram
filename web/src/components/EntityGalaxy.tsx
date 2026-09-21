/**
 * 圈子图（记忆星系）——2026-09-21 起 = **共享引擎薄壳**。
 *
 * 物理（d3-force in Worker）、渲染（sigma）、交互（hover/拖拽/缩放）、参数面板、LOD、静停
 * 全在 `@/components/ForceGraph` 里；本文件只做「记忆星系语义 → 通用入参」的映射。
 *
 * 原能力逐条保留：
 * ① **「我」锚点**：固定在最中、**不连任何边**（旧版的「我-实体」轮辐边是「密集成菊花团」的病根），
 *    布局只由实体间真实共现/关系驱动；
 * ② **kind 配色**（`ENTITY_KIND_COLOR`）+ 度数越大越大；
 * ③ 点击实体 → 看档案（`onSelect`）；点击「我」 → 跳画像（`onGoPersona`）；
 * ④ **常识边视觉分层**：`source='world_knowledge'` 的边半透明细线（第 2 层实体天然止步于此）；
 * ⑤ 坐标持久化（「重排布局」= 清坐标重新炸开），由共享引擎统一负责。
 */
import { useMemo } from 'react'
import type { EntityGraph as GraphData } from '@/lib/api'
import { ENTITY_KIND_COLOR } from '@/lib/ui'
import { useThemeTick } from '@/lib/theme'
import ForceGraph from '@/components/ForceGraph/ForceGraph'

const KIND_COLOR = ENTITY_KIND_COLOR
const ME = '__me__'
/** 「我」的等效度数：让它比普通实体略大一点（半径公式 clamp(3·√(w+1),8,30) 的反推值） */
const ME_WEIGHT = 21

function themeColors() {
  const cs = getComputedStyle(document.documentElement)
  const v = (name: string, fb: string) => cs.getPropertyValue(name).trim() || fb
  return { fg: v('--foreground', '#0a0a0a'), muted: v('--muted-foreground', '#71717a'), border: v('--border', '#e5e5e5') }
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
  const themeTick = useThemeTick()

  const view = useMemo(() => {
    const tc = themeColors()
    const nodes = [
      {
        id: ME,
        label: '我',
        color: tc.fg,
        weight: ME_WEIGHT,
        // 锚点固定在最中——不连边（见文件头 ①）
        fixed: true,
        x: 0,
        y: 0,
      },
      ...graph.nodes.map((n) => ({
        id: n.id,
        label: n.name,
        color: KIND_COLOR[n.kind] ?? tc.muted,
      })),
    ]
    const edges = [
      ...graph.edges.map((e) => ({ source: e.a, target: e.b, weight: e.weight })),
      // 常识边（world_knowledge）：半透明细线，与记忆边视觉分层
      ...graph.relations.map((r) => ({
        source: r.from_id,
        target: r.to_id,
        weight: r.weight,
        color: r.source === 'world_knowledge' ? `${tc.border}55` : tc.border,
      })),
    ]
    return { nodes, edges }
    // themeTick：主题切换时重取 token 色
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [graph, themeTick])

  return (
    <ForceGraph
      nodes={view.nodes}
      edges={view.edges}
      persistKey="circle"
      testId="circle-galaxy-canvas"
      // 实体量级（数百）+ 记忆边界紧：初值把节点调小（滑杆语义，用户可再调）
      initialDisplay={{ nodeSize: 0.5 }}
      initialForces={{ center: 0.6, repel: 10, linkStrength: 1, linkDistance: 250 }}
      onPick={(id) => (id === ME ? onGoPersona() : onSelect(id))}
      className="relative min-h-0 flex-1"
      legend={
        <>
          {(Object.keys(KIND_COLOR) as string[]).slice(0, 8).map((k) => (
            <span key={k} className="flex items-center gap-1.5">
              <span
                className="inline-block size-2 rounded-full"
                style={{ background: KIND_COLOR[k] }}
                aria-hidden="true"
              />
              {k}
            </span>
          ))}
          <span className="ml-auto">点「我」看画像 · 点实体看档案 · 拖动松手回弹</span>
        </>
      }
    />
  )
}

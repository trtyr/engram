/**
 * Wiki 链接图谱（2026-09-21 起 = **共享引擎薄壳**）。
 *
 * 物理（d3-force in Worker）、渲染（sigma）、交互（hover/拖拽/缩放）、参数面板（力/显示）、
 * LOD 与静停全部在 `@/components/ForceGraph` 里——本文件只做「Wiki 语义 → 通用入参」的映射与图例，
 * 不再自己维护任何物理与交互代码。
 *
 * 保留的原能力（逐条）：社区/type **双着色**与切换、凝聚度/稀疏社区图例、
 * **洞察联动高亮**（`highlightSlugs`）、**强链接编码**（weight ≥ 6 绿色）、
 * 点击节点跳 `/wiki?page=<slug>`。Wiki 量级（数百节点）走**客户端自收敛**（不落 layout.json）。
 */
import { useMemo, useRef, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import type { GraphDto } from '@/lib/api'
import { Empty, Tabs } from '@/components/ui-bits'
import ForceGraph from '@/components/ForceGraph/ForceGraph'

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
  '#ef4444',
  '#f97316',
  '#eab308',
  '#84cc16',
  '#22c55e',
  '#14b8a6',
  '#06b6d4',
  '#3b82f6',
  '#6366f1',
  '#a855f7',
  '#d946ef',
  '#f43f5e',
]

/** 强链接阈值（原口径）与配色（原实现同色；`77` = 半透明，避免细边糊成毛线团） */
const STRONG_EDGE = 6
const STRONG_COLOR = '#3dd68c'

type ColorMode = 'community' | 'type'

/** 高亮的 slug 集合（洞察卡片点击联动） */
export default function WikiGraph({
  graph,
  highlightSlugs,
}: {
  graph: GraphDto
  highlightSlugs?: string[]
}) {
  const nav = useNavigate()
  const [mode, setMode] = useState<ColorMode>('community')
  /** 全屏目标：整块（含着色切换与图例）——交给共享引擎的全屏按钮 */
  const rootRef = useRef<HTMLDivElement>(null)

  const view = useMemo(() => {
    const colorOf = (pageType: string, community: number) =>
      mode === 'type'
        ? (TYPE_COLOR[pageType] ?? '#6b7280')
        : COMMUNITY_PALETTE[community % COMMUNITY_PALETTE.length]
    const slugs = new Set(graph.nodes.map((n) => n.slug))
    return {
      nodes: graph.nodes.map((n) => ({
        id: n.slug,
        label: n.title,
        // 半透明（B3≈70%）：页面多节点密，实色大点会糊
        color: `${colorOf(n.page_type, n.community ?? 0)}B3`,
      })),
      edges: graph.edges
        .filter((e) => slugs.has(e.from_slug) && slugs.has(e.to_slug))
        .map((e) => ({
          source: e.from_slug,
          target: e.to_slug,
          weight: e.weight,
          color: (e.weight ?? 1) >= STRONG_EDGE ? `${STRONG_COLOR}77` : undefined,
        })),
    }
  }, [graph, mode])

  if (graph.nodes.length === 0) {
    return <Empty text="图谱为空（ingest 后生成）" />
  }

  const communityCount = new Set(graph.nodes.map((n) => n.community ?? 0)).size
  const sparseComms = (graph.communities ?? []).filter((c) => c.sparse)

  return (
    <div
      ref={rootRef}
      className="flex h-full min-h-[520px] flex-col space-y-3"
      data-testid="wiki-graph-root"
    >
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
          <span className="text-xs text-warning">{sparseComms.length} 个稀疏社区</span>
        )}
      </div>
      <ForceGraph
        nodes={view.nodes}
        edges={view.edges}
        persistKey="wiki"
        testId="wiki-graph-canvas"
        fullscreenTargetRef={rootRef}
        highlightIds={highlightSlugs}
        // 页面量级（数百节点）+ 字体密集：初值把节点调小（滑杆语义，用户可再调）
        initialDisplay={{ nodeSize: 0.5 }}
        onPick={(slug) => nav(`/wiki?page=${encodeURIComponent(slug)}`)}
        className="relative min-h-0 flex-1"
        legend={
          <>
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
          </>
        }
      />
    </div>
  )
}

/**
 * 项目 ↔ 资产关系图谱（共享引擎第四位住户，2026-09-21）。
 *
 * 本文件只做「工作线 + 资产语义 → 通用入参」的映射（与 WikiGraph / EntityGalaxy / CodeGraph 同款），
 * 力导向、静停、LOD、全屏全在 `@/components/ForceGraph` 里——引擎一行不动。
 *
 * 语义（《项目与资产模型 · README》§2）：
 *   · 节点 = 项目（可切换按场景 / 单色）+ 资产（按类型着色）
 *   · 边 = part_of 隶属（项目 → 母项目，有向）/ related 相关 / uses 用到（项目 → 资产，有向）
 * 点节点跳详情/资产页；双击收窄到邻域（Local graph 语义，用引擎的 highlightIds）。
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { api, type ProjectGraphDto } from '@/lib/api'
import ForceGraph, {
  type ForceGraphEdge,
  type ForceGraphNode,
} from '@/components/ForceGraph/ForceGraph'
import { ErrorBox, Spinner } from '@/components/ui-bits'

/** 场景色（与项目列表页同源口径；值域事实源在后端 PROJECT_TYPES）。 */
const SCENE_DOT: Record<string, string> = {
  dev: '#3b82f6',
  ops: '#10b981',
  research: '#8b5cf6',
  study: '#f59e0b',
  life: '#06b6d4',
  create: '#ec4899',
}
/** 资产类型色（与资产页同源口径；值域事实源在后端 ASSET_KINDS）。 */
const KIND_DOT: Record<string, string> = {
  host: '#3b82f6',
  cloud: '#8b5cf6',
  domain: '#f59e0b',
  account: '#10b981',
  device: '#06b6d4',
  other: '#6b7280',
}
/** 项目统一色（非场景着色模式下用）。 */
const PROJECT_COLOR = '#6366f1'
const EDGE = {
  part_of: '#8b5cf6',
  related: '#94a3b8',
  uses: '#10b981',
}

export default function ProjectAssetGraph() {
  const nav = useNavigate()
  const [data, setData] = useState<ProjectGraphDto | null>(null)
  const [err, setErr] = useState('')
  const [colorBy, setColorBy] = useState<'scene' | 'type'>('scene')
  const [highlight, setHighlight] = useState<string[]>([])
  const highlightRef = useRef<string[]>([])

  useEffect(() => {
    api
      .get<ProjectGraphDto>('/projects/graph')
      .then(setData)
      .catch((e) => setErr(e instanceof Error ? e.message : '图谱加载失败'))
  }, [])

  const { nodes, edges } = useMemo(() => {
    if (!data) return { nodes: [] as ForceGraphNode[], edges: [] as ForceGraphEdge[] }
    const nodes: ForceGraphNode[] = [
      ...data.projects.map((p) => ({
        id: `p:${p.id}`,
        label: p.name,
        color: colorBy === 'scene' ? (SCENE_DOT[p.type] ?? '#6b7280') : PROJECT_COLOR,
      })),
      ...data.assets.map((a) => ({
        id: `a:${a.id}`,
        label: a.name,
        color: KIND_DOT[a.kind] ?? '#6b7280',
      })),
    ]
    const edges: ForceGraphEdge[] = [
      ...data.links.map((l) => ({
        source: `p:${l.from_project}`,
        target: `p:${l.to_project}`,
        color: EDGE[l.kind as keyof typeof EDGE] ?? EDGE.related,
        width: l.kind === 'part_of' ? 2 : 1,
      })),
      ...data.usages.map((u) => ({
        source: `p:${u.project_id}`,
        target: `a:${u.asset_id}`,
        color: EDGE.uses,
        width: 1,
      })),
    ]
    return { nodes, edges }
  }, [data, colorBy])

  const onPick = useCallback(
    (id: string) => {
      const [kind, uuid] = id.split(':')
      if (kind === 'p') nav(`/projects/${uuid}`)
      else if (kind === 'a') nav('/assets')
    },
    [nav],
  )

  /** 双击：收窄到该节点的邻域（Local graph 语义——引擎负责把它渲染成高亮）。 */
  const onExpand = useCallback(
    (id: string) => {
      const cur = highlightRef.current
      if (cur.length > 0 && cur[0] === id) {
        highlightRef.current = []
        setHighlight([])
        return
      }
      const near = new Set<string>([id])
      for (const e of edges) {
        if (e.source === id) near.add(e.target)
        if (e.target === id) near.add(e.source)
      }
      for (const e of edges) {
        if (e.source !== id && e.target !== id) continue
        const other = e.source === id ? e.target : e.source
        for (const e2 of edges) {
          if (e2.source === other) near.add(e2.target)
          if (e2.target === other) near.add(e2.source)
        }
      }
      const arr = [...near]
      highlightRef.current = arr
      setHighlight(arr)
    },
    [edges],
  )

  if (err) return <ErrorBox msg={err} />
  if (!data) return <Spinner />

  const nameOf = (id: string) => nodes.find((n) => n.id === id)?.label ?? id

  return (
    <div className="space-y-2">
      <p className="text-xs text-muted-foreground">
        工作线（圆点）与资产（台账条目）的关系网：紫边 = <b>隶属</b>（项目 → 母项目）、
        绿边 = <b>用到</b>（项目 → 资产）、灰边 = 相关。点节点跳页面，双击收窄到邻域。
      </p>
      <ForceGraph
        nodes={nodes}
        edges={edges}
        directed
        persistKey="project-assets"
        onPick={onPick}
        onExpand={onExpand}
        highlightIds={highlight}
        testId="project-asset-graph"
        className="h-[520px]"
        toolbar={
          <button
            type="button"
            className="rounded border border-border bg-card px-2 py-1 text-xs text-muted-foreground hover:text-foreground"
            onClick={() => setColorBy((c) => (c === 'scene' ? 'type' : 'scene'))}
            data-testid="scene-toggle"
          >
            {colorBy === 'scene' ? '场景着色' : '单色'}
          </button>
        }
        legend={
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
            <span className="flex items-center gap-1">
              <i className="size-2 rounded-full" style={{ background: EDGE.part_of }} /> 隶属
            </span>
            <span className="flex items-center gap-1">
              <i className="size-2 rounded-full" style={{ background: EDGE.uses }} /> 用到
            </span>
            <span className="flex items-center gap-1">
              <i className="size-2 rounded-full" style={{ background: EDGE.related }} /> 相关
            </span>
            <span className="text-muted-foreground/70">
              共 {data.projects.length} 工作线 · {data.assets.length} 资产 · {edges.length} 条关系
            </span>
          </div>
        }
      />
      {highlight.length > 0 && (
        <p className="text-[11px] text-muted-foreground">
          邻域：{highlight.slice(0, 6).map(nameOf).join('、')}
          {highlight.length > 6 && ` 等 ${highlight.length} 个节点`}（再双击一次取消）
        </p>
      )}
    </div>
  )
}

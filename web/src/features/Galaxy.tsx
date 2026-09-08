/**
 * 圈子 tab：左实体列表（搜索/类型过滤/密度排序/新建）+ 右侧默认关系图谱，
 * 点击节点或列表项进入实体详情（画像摘要/原子时间线/相关场景/挂摘/合并）。
 */
import { Suspense, lazy, useEffect, useMemo, useState } from 'react'
import { appConfirm } from '@/components/confirm'
import { Search } from 'lucide-react'
import {
  api,
  type Atom,
  type EntityDetail,
  type EntityGraph,
  type EntityNode,
  type EntityRevision,
  type SearchHit,
  type TimelineEvent,
} from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Card, Checkbox, Empty, ErrorBox, Spinner, StatusBadge } from '@/components/ui-bits'
import { ENTITY_KIND_COLOR as KIND_COLOR, REL_TYPE_COLOR, REL_TYPE_LABEL } from '@/lib/ui'
import { relTime, inputCls, selectCls } from '@/lib/ui'
import { cn } from '@/lib/utils'

const EntityGalaxy = lazy(() => import('@/components/EntityGalaxy'))

const KIND_LABEL: Record<string, string> = {
  person: '人物',
  project: '项目',
  topic: '主题',
  group: '群组',
  place: '地点',
}

export default function Galaxy({
  initialEntity,
  onGoPersona,
  onGoAtoms,
}: {
  initialEntity?: string | null
  onGoPersona: () => void
  onGoAtoms: () => void
}) {
  const [graph, setGraph] = useState<EntityGraph | null>(null)
  const [err, setErr] = useState('')
  const [selected, setSelected] = useState<string | null>(initialEntity ?? null)
  const [q, setQ] = useState('')
  const [kind, setKind] = useState('')
  const [creating, setCreating] = useState(false)
  const [searchHits, setSearchHits] = useState<SearchHit[] | null>(null)
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set())
  const [viewMode, setViewMode] = useState<'graph' | 'timeline'>('graph')

  const load = () =>
    api
      .get<EntityGraph>('/memory/entities/graph')
      .then(setGraph)
      .catch((e) => setErr(e instanceof Error ? e.message : String(e)))
  useEffect(() => {
    load()
  }, [])
  // 语义搜索：停顿 350ms 后打后端 token 检索（名字命中优先，summary 次之）
  useEffect(() => {
    if (!q.trim()) return
    const t = setTimeout(() => {
      api
        .get<SearchHit[]>(`/memory/entities/search?q=${encodeURIComponent(q)}`)
        .then(setSearchHits)
        .catch(() => setSearchHits(null))
    }, 350)
    return () => clearTimeout(t)
  }, [q])

  // 列表数据：语义搜索命中时按 score 映射回实体；否则 substring 过滤
  const visible = useMemo(() => {
    if (!graph) return [] as EntityNode[]
    if (searchHits) {
      const byId = new Map(graph.nodes.map((n) => [n.id, n]))
      const hits = searchHits
        .map((h) => byId.get(h.id))
        .filter((n): n is EntityNode => n !== undefined)
      if (hits.length > 0) return hits.filter((n) => !kind || n.kind === kind)
    }
    return graph.nodes.filter(
      (n) =>
        (!kind || n.kind === kind) &&
        (!q || n.name.toLowerCase().includes(q.toLowerCase()) || n.summary.includes(q)),
    )
  }, [graph, searchHits, kind, q])
  // 疑似重复：name 归一化（忽略大小写/空白/中英标点）后同名 → 提示候选 merge
  const dupGroups = useMemo(() => {
    if (!graph) return [] as EntityNode[][]
    const normalize = (s: string) => s.toLowerCase().replace(/[\s·.\-_()（）【】《》,，。、]/g, '')
    const m = new Map<string, EntityNode[]>()
    for (const n of graph.nodes) {
      const k = normalize(n.name)
      const arr = m.get(k) ?? []
      arr.push(n)
      m.set(k, arr)
    }
    return [...m.values()].filter((g) => g.length > 1)
  }, [graph])
  // 图数据 useMemo 缓存：EntityGalaxy 的 useEffect 依赖 graph 引用——直接传字面量
  // 会在每次 re-render 造新对象，触发图销毁重建 + 力导向重跑（卡顿/闪动的根因）。
  // 图显示全量实体（Obsidian 全貌），搜索/类型过滤只作用于左侧列表。
  const galaxyData = useMemo(
    () => ({
      nodes: graph?.nodes ?? [],
      edges: graph?.edges ?? [],
      relations: graph?.relations ?? [],
    }),
    [graph],
  )

  if (err) return <ErrorBox msg={err} />
  if (!graph) return <Spinner />
  // 低密度态：所有实体关联数 ≤1 → 图退化成均匀星形，诚实提示看列表
  const sparse = graph.nodes.length > 0 && graph.nodes.every((n) => n.atom_count <= 1)
  const toggleSelect = (id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }
  const doExport = async () => {
    const data = await api.get<Record<string, unknown>>('/memory/entities/export')
    const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = 'engram-entities-export.json'
    a.click()
    URL.revokeObjectURL(url)
  }
  const doBatchDelete = async () => {
    if (
      !(await appConfirm({
        title: '批量删除选中的实体？',
        description: '实体退场，原子本身保留。此操作不可逆。',
        destructive: true,
        confirmLabel: '批量删除',
        inputMatch: '批量删除',
      }))
    )
      return
    await api.post('/memory/entities/batch', { ids: [...selectedIds], confirm: '批量删除' })
    setSelectedIds(new Set())
    load()
  }

  return (
    <div className="flex h-[calc(100vh-8rem)] min-h-[32rem] flex-col gap-4 lg:flex-row">
      {/* 左：实体列表——清点与管理（独立滚动，与右侧互不牵连） */}
      <Card className="flex min-h-0 flex-col overflow-hidden lg:w-80 lg:shrink-0">
        <div className="flex items-center gap-2 border-b border-border px-3 py-2">
          <span className="font-mono text-xs text-muted-foreground">{graph.nodes.length} 实体</span>
          {selectedIds.size > 0 && (
            <button
              type="button"
              onClick={doBatchDelete}
              className="rounded border border-destructive/30 px-1.5 py-px text-xs text-destructive transition-colors hover:bg-destructive/10"
            >
              批量删除 {selectedIds.size}
            </button>
          )}
          <button
            type="button"
            onClick={doExport}
            className="rounded border border-border px-1.5 py-px text-xs text-muted-foreground transition-colors hover:border-foreground/30"
          >
            导出
          </button>
          <div className="relative ml-auto">
            <Search className="pointer-events-none absolute left-2 top-1/2 size-3 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
            <input
              className="w-32 rounded-md border border-border bg-card py-1 pl-7 pr-2 text-xs outline-none transition-colors placeholder:text-muted-foreground/60 focus-visible:border-foreground/40"
              placeholder="搜索实体…"
              value={q}
              onChange={(e) => {
              setQ(e.target.value)
              // 清空检索词时立即清结果（此前在 effect 里同步 setState，触发级联渲染告警）
              if (!e.target.value.trim()) setSearchHits(null)
            }}
            />
          </div>
        </div>
        {dupGroups.length > 0 && (
          <div className="border-b border-warning/30 bg-warning/10 px-3 py-1.5">
            {dupGroups.map((g) => (
              <p key={g[0].id} className="truncate text-xs text-warning">
                疑似重复：{g.map((n) => n.name).join(' ≈ ')}——可选中后「合并」
              </p>
            ))}
          </div>
        )}
        <div className="flex items-center gap-1.5 border-b border-border px-3 py-1.5">
          <button
            type="button"
            onClick={() => setKind('')}
            className={cn(
              'rounded border px-1.5 py-px font-mono text-xs transition-colors',
              kind === '' ? 'border-foreground bg-foreground text-background' : 'border-border text-muted-foreground hover:border-foreground/30',
            )}
          >
            全部
          </button>
          {Object.entries(KIND_LABEL).map(([k, label]) => (
            <button
              key={k}
              type="button"
              onClick={() => setKind(kind === k ? '' : k)}
              className={cn(
                'rounded border px-1.5 py-px font-mono text-xs transition-colors',
                kind === k ? 'border-foreground bg-foreground text-background' : 'border-border text-muted-foreground hover:border-foreground/30',
              )}
            >
              {label}
            </button>
          ))}
          <Button variant="ghost" size="sm" className="ml-auto h-6 px-2 text-xs" onClick={() => setCreating((v) => !v)}>
            {creating ? '收起' : '＋ 新建'}
          </Button>
        </div>
        {creating && (
          <CreateEntityForm
            onDone={() => {
              setCreating(false)
              load()
            }}
          />
        )}
        {visible.length === 0 ? (
          <div className="p-4">
            <Empty text={graph.nodes.length === 0 ? '暂无实体——蒸馏自动抽取，或手动新建' : '无匹配实体'} />
          </div>
        ) : (
          <ul className="min-h-0 flex-1 divide-y divide-border/60 overflow-y-auto">
            {visible.map((n) => (
              <li key={n.id}>
                <div className="flex items-center">
                  <Checkbox
                    checked={selectedIds.has(n.id)}
                    onChange={() => toggleSelect(n.id)}
                    label={`选择 ${n.name}`}
                    className="ml-2.5"
                  />
                  <button
                    type="button"
                    onClick={() => setSelected(n.id)}
                    aria-current={n.id === selected ? 'true' : undefined}
                    className={cn(
                      'min-w-0 flex-1 px-2 py-2 text-left transition-colors',
                      n.id === selected ? 'bg-foreground text-background' : 'hover:bg-muted/40',
                    )}
                  >
                    <p className="flex items-center gap-2">
                      <span
                        className="inline-block size-2 shrink-0 rounded-full"
                        style={{ backgroundColor: KIND_COLOR[n.kind] ?? 'var(--muted-foreground)' }}
                        aria-hidden="true"
                      />
                      <span className="truncate text-sm font-medium">{n.name}</span>
                      <span className="ml-auto shrink-0 font-mono text-xs tabular-nums">{n.atom_count}</span>
                    </p>
                    {n.summary && (
                      <p className={cn(
                        'mt-0.5 truncate text-xs',
                        n.id === selected ? 'text-background/70' : 'text-muted-foreground',
                      )}>
                        {n.summary}
                      </p>
                    )}
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </Card>

      {/* 右：默认关系图谱，选中即详情（独立滚动区） */}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {selected ? (
          <EntityDetailPane
            key={selected}
            entityId={selected}
            onBack={() => setSelected(null)}
            onMutated={load}
            onGoAtoms={onGoAtoms}
            onSelectEntity={setSelected}
            allNodes={graph.nodes}
          />
        ) : visible.length === 0 && graph.nodes.length === 0 ? (
          <Card className="p-4">
            <Empty text="圈子是你的记忆世界：人物 / 项目 / 主题 / 群组 / 地点。蒸馏会自动把对话里的人和事挂进来，也可以先手动新建" />
          </Card>
        ) : (
          <div className="flex min-h-0 flex-1 flex-col gap-3">
            {/* 视图切换：图谱 / 时间轴 */}
            <div className="flex items-center gap-1.5">
              <button
                type="button"
                onClick={() => setViewMode('graph')}
                className={cn('rounded border px-2 py-0.5 text-xs transition-colors', viewMode === 'graph' ? 'border-foreground bg-foreground text-background' : 'border-border text-muted-foreground hover:border-foreground/30')}
              >
                图谱
              </button>
              <button
                type="button"
                onClick={() => setViewMode('timeline')}
                className={cn('rounded border px-2 py-0.5 text-xs transition-colors', viewMode === 'timeline' ? 'border-foreground bg-foreground text-background' : 'border-border text-muted-foreground hover:border-foreground/30')}
              >
                时间轴
              </button>
            </div>
            {viewMode === 'timeline' ? (
              <TimelineView />
            ) : (
              <>
            {/* 图示：三行定义式，不再挤一段 */}
            <div className="grid gap-0.5 text-xs text-muted-foreground sm:grid-cols-2">
              <p><span className="text-foreground">圆点</span> = 记忆里的人 / 项目 / 主题（大小 = 记忆条数）</p>
              <p><span className="text-foreground">连线</span> = 一起出现（越粗越常见，自动抱团）</p>
              <p><span className="text-foreground">「我」</span> = 你 —— 点圆点看档案 · 按住拖动 · 滚轮缩放</p>
              <p className="flex flex-wrap items-center gap-2.5">
                {Object.entries(KIND_LABEL).map(([k, label]) => (
                  <span key={k} className="flex items-center gap-1 font-mono">
                    <span className="inline-block size-2 rounded-full" style={{ backgroundColor: KIND_COLOR[k] }} aria-hidden="true" />
                    {label}
                  </span>
                ))}
              </p>
            </div>
            {sparse && (
              <p className="rounded border border-border bg-muted/30 px-2.5 py-1.5 text-xs text-muted-foreground">
                记忆还在积累——每个实体目前的关联都不多，图谱关系会随蒸馏逐渐成形。现在先看左侧列表更清楚。
              </p>
            )}
            <Suspense fallback={<div className="min-h-0 flex-1 animate-pulse rounded-lg bg-muted/30" />}>
              <EntityGalaxy
                graph={galaxyData}
                onSelect={setSelected}
                onGoPersona={onGoPersona}
              />
            </Suspense>
              </>
            )}
          </div>
        )}
      </div>
    </div>
  )
}

function CreateEntityForm({ onDone }: { onDone: () => void }) {
  const [name, setName] = useState('')
  const [kind, setKind] = useState('person')
  const [summary, setSummary] = useState('')
  const [err, setErr] = useState('')
  return (
    <form
      className="space-y-2 border-b border-border bg-muted/30 px-3 py-3"
      onSubmit={async (e) => {
        e.preventDefault()
        try {
          await api.post('/memory/entities', { name, kind, summary })
          onDone()
        } catch (ex) {
          setErr(ex instanceof Error ? ex.message : '创建失败')
        }
      }}
    >
      <div className="flex gap-2">
        <input className={`${inputCls} flex-1`} placeholder="实体名（如：张三）" value={name} onChange={(e) => setName(e.target.value)} />
        <select className={`${selectCls} w-24`} value={kind} onChange={(e) => setKind(e.target.value)}>
          {Object.entries(KIND_LABEL).map(([k, label]) => (
            <option key={k} value={k}>
              {label}
            </option>
          ))}
        </select>
      </div>
      <input className={`${inputCls} w-full`} placeholder="画像摘要（可选，如：同事，负责后端）" value={summary} onChange={(e) => setSummary(e.target.value)} />
      {err && <p className="text-xs text-destructive">{err}</p>}
      <div className="flex justify-end">
        <Button size="sm" type="submit" disabled={!name.trim()}>
          创建
        </Button>
      </div>
    </form>
  )
}

function EntityDetailPane({
  entityId,
  onBack,
  onMutated,
  onGoAtoms,
  onSelectEntity,
  allNodes,
}: {
  entityId: string
  onBack: () => void
  onMutated: () => void
  onGoAtoms: () => void
  onSelectEntity: (id: string) => void
  allNodes: EntityNode[]
}) {
  const [detail, setDetail] = useState<EntityDetail | null>(null)
  const [err, setErr] = useState('')
  const [attachOpen, setAttachOpen] = useState(false)
  const [editingSummary, setEditingSummary] = useState(false)
  const [summaryDraft, setSummaryDraft] = useState('')
  const [mergeOpen, setMergeOpen] = useState(false)
  const [historyOpen, setHistoryOpen] = useState(false)
  const [entityRevisions, setEntityRevisions] = useState<EntityRevision[]>([])
  const [relOpen, setRelOpen] = useState(false)

  useEffect(() => {
    api
      .get<EntityDetail>(`/memory/entities/${entityId}`)
      .then(setDetail)
      .catch((e) => setErr(e instanceof Error ? e.message : String(e)))
  }, [entityId])

  const nameById = useMemo(() => new Map(allNodes.map((n) => [n.id, n.name])), [allNodes])

  if (err) return <ErrorBox msg={err} />
  if (!detail) return <Spinner />
  const { entity, atoms, scenarios, neighbors, relations } = detail

  return (
    <Card className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <div className="relative shrink-0 border-b border-border px-4 py-3">
        <div className="flex flex-wrap items-start justify-between gap-2">
          <div className="min-w-0 pr-40">
            <h2 className="flex items-center gap-2 text-base font-semibold tracking-tight">
              <span className="inline-block size-2.5 rounded-full" style={{ backgroundColor: KIND_COLOR[entity.kind] }} aria-hidden="true" />
              {entity.name}
              <span className="rounded border border-border px-1.5 py-px font-mono text-xs font-normal text-muted-foreground">
                {KIND_LABEL[entity.kind] ?? entity.kind}
              </span>
            </h2>
            {editingSummary ? (
              <div className="mt-2 space-y-2">
                <textarea
                  aria-label="实体摘要"
                  className={`${inputCls} min-h-24 w-full`}
                  value={summaryDraft}
                  onChange={(e) => setSummaryDraft(e.target.value)}
                />
                <div className="flex gap-1.5">
                  <Button
                    size="sm"
                    onClick={async () => {
                      await api.patch(`/memory/entities/${entity.id}`, { summary: summaryDraft })
                      setEditingSummary(false)
                      onMutated()
                    }}
                  >
                    保存（钉住）
                  </Button>
                  <Button variant="ghost" size="sm" onClick={() => setEditingSummary(false)}>
                    取消
                  </Button>
                </div>
              </div>
            ) : (
              <p
                className="mt-1 -mx-1 cursor-text rounded-sm px-1 text-sm text-muted-foreground transition-colors hover:bg-muted/40"
                title="点击编辑摘要（手编档案，蒸馏绕开）"
                onClick={() => {
                  setSummaryDraft(entity.summary)
                  setEditingSummary(true)
                }}
              >
                {entity.summary || '尚无画像摘要——蒸馏积累后自动丰富'}
                {entity.manually_edited && (
                  <span className="ml-1.5 rounded bg-success/15 px-1.5 py-0.5 font-mono text-xs text-success">已钉住</span>
                )}
              </p>
            )}
            <div className="mt-1 flex flex-wrap items-center gap-3 font-mono text-xs text-muted-foreground/70">
              <span>{entity.atom_count} 条原子 · 更新于 {relTime(entity.updated_at)}</span>
              <button
                type="button"
                onClick={async () => {
                  if (historyOpen) {
                    setHistoryOpen(false)
                    return
                  }
                  try {
                    const revs = await api.get<EntityRevision[]>(`/memory/entities/${entity.id}/revisions`)
                    setEntityRevisions(revs)
                    setHistoryOpen(true)
                  } catch {
                    setEntityRevisions([])
                    setHistoryOpen(true)
                  }
                }}
                className="underline underline-offset-4 hover:text-foreground"
              >
                历史{entityRevisions.length > 0 ? `（${entityRevisions.length}）` : ''}
              </button>
            </div>
            {historyOpen && (
              <div className="mt-2 space-y-1.5">
                {entityRevisions.length === 0 ? (
                  <p className="text-xs text-muted-foreground">暂无历史版本</p>
                ) : (
                  entityRevisions.map((r) => (
                    <div key={r.id} className="rounded border border-border bg-muted/30 px-2.5 py-1.5">
                      <p className="line-clamp-2 text-xs text-muted-foreground">{r.old_summary || '（空摘要）'}</p>
                      <p className="mt-0.5 font-mono text-[10px] text-muted-foreground/60">{r.edited_by} · {relTime(r.created_at)}</p>
                    </div>
                  ))
                )}
              </div>
            )}
          </div>
          {/* 固定右上：不随标题/摘要换行漂移（用户实测痛点 #4） */}
          <div className="absolute right-3 top-3 flex items-center gap-1.5">
            <Button variant="ghost" size="sm" onClick={onBack}>
              ← 返回图谱
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="hover:bg-destructive/10 hover:text-destructive"
              onClick={async () => {
                if (
                  !(await appConfirm({
                    title: `删除实体「${entity.name}」？`,
                    description: '原子本身保留，仅解除关联。',
                    destructive: true,
                    confirmLabel: '删除',
                  }))
                )
                  return
                await api.del(`/memory/entities/${entity.id}`)
                onBack()
                onMutated()
              }}
            >
              删除
            </Button>
          </div>
        </div>
      </div>

      <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-4 py-4">
        {/* 相关实体（共现邻居） */}
        {neighbors.length > 0 && (
          <section>
            <h3 className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">相关实体（{neighbors.length}）</h3>
            <div className="flex flex-wrap gap-1.5">
              {neighbors.map((n) => (
                <button
                  key={n.id}
                  type="button"
                  onClick={() => onSelectEntity(n.id)}
                  className="flex items-center gap-1.5 rounded border border-border px-1.5 py-px text-xs text-muted-foreground transition-colors hover:border-foreground/30 hover:text-foreground"
                >
                  <span className="inline-block size-2 rounded-full" style={{ backgroundColor: KIND_COLOR[n.kind] ?? 'var(--muted-foreground)' }} aria-hidden="true" />
                  {n.name}
                  <span className="font-mono text-muted-foreground/60">{n.atom_count}</span>
                </button>
              ))}
            </div>
          </section>
        )}

        {/* 关系（有向类型化） */}
        <section>
          <div className="mb-2 flex items-center justify-between">
            <h3 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">关系（{relations.length}）</h3>
            <Button variant="ghost" size="sm" onClick={() => setRelOpen((v) => !v)}>
              {relOpen ? '取消' : '＋ 建关系'}
            </Button>
          </div>
          {relOpen && (
            <RelationForm
              entityId={entity.id}
              allNodes={allNodes}
              onDone={() => {
                setRelOpen(false)
                api.get<EntityDetail>(`/memory/entities/${entityId}`).then(setDetail)
                onMutated()
              }}
            />
          )}
          {relations.length === 0 ? (
            <p className="text-xs text-muted-foreground">暂无类型化关系——蒸馏抽取或手动建立</p>
          ) : (
            <div className="flex flex-wrap gap-1.5">
              {relations.map((r) => {
                const isFrom = r.from_id === entity.id
                const otherId = isFrom ? r.to_id : r.from_id
                const otherName = nameById.get(otherId) ?? otherId.slice(0, 8)
                const label = (
                  <span className="font-mono" style={{ color: REL_TYPE_COLOR[r.rel_type] ?? 'var(--muted-foreground)' }}>
                    {REL_TYPE_LABEL[r.rel_type] ?? r.rel_type}
                  </span>
                )
                const other = (
                  <button type="button" onClick={() => onSelectEntity(otherId)} className="hover:underline">
                    {otherName}
                  </button>
                )
                return (
                  <span key={r.id} className="flex items-center gap-1 rounded border border-border px-1.5 py-px text-xs">
                    {isFrom ? (
                      <>我 {label}→ {other}</>
                    ) : (
                      <>{other} {label}→ 我</>
                    )}
                    {r.weight > 1 && <span className="font-mono text-muted-foreground/60">×{r.weight}</span>}
                    <button
                      type="button"
                      aria-label="删除关系"
                      title="删除关系"
                      className="text-muted-foreground/0 transition-colors hover:text-destructive"
                      onClick={async () => {
                        await api.del(`/memory/entities/${entity.id}/relations/${r.id}`)
                        api.get<EntityDetail>(`/memory/entities/${entityId}`).then(setDetail)
                        onMutated()
                      }}
                    >
                      ×
                    </button>
                  </span>
                )
              })}
            </div>
          )}
        </section>

        {/* 相关场景 */}
        {scenarios.length > 0 && (
          <section>
            <h3 className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">相关场景（{scenarios.length}）</h3>
            <div className="flex flex-wrap gap-1.5">
              {scenarios.map((s) => (
                <span key={s.id} className="rounded border border-border px-1.5 py-px text-xs text-muted-foreground" title={s.summary}>
                  {s.topic}
                </span>
              ))}
            </div>
          </section>
        )}

        {/* 原子时间线 */}
        <section>
          <div className="mb-2 flex items-center justify-between">
            <h3 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">原子时间线（{atoms.length}）</h3>
            <div className="flex gap-1.5">
              <Button variant="ghost" size="sm" onClick={() => setAttachOpen((v) => !v)}>
                {attachOpen ? '取消' : '挂原子'}
              </Button>
              <Button variant="ghost" size="sm" onClick={() => setMergeOpen((v) => !v)}>
                {mergeOpen ? '取消' : '合并…'}
              </Button>
            </div>
          </div>

          {attachOpen && (
            <AttachForm
              entityId={entity.id}
              exclude={atoms.map((a) => a.id)}
              onDone={() => {
                setAttachOpen(false)
                api.get<EntityDetail>(`/memory/entities/${entityId}`).then(setDetail)
                onMutated()
              }}
            />
          )}
          {mergeOpen && (
            <MergeForm
              entity={entity}
              onDone={() => {
                onBack()
                onMutated()
              }}
            />
          )}

          {atoms.length === 0 ? (
            <p className="text-sm text-muted-foreground">还没有关联原子——蒸馏抽取或手动挂链后在这里形成时间线</p>
          ) : (
            <ul className="divide-y divide-border/60">
              {atoms.map((a: Atom) => (
                <li key={a.id} className="group flex items-start gap-3 py-2.5">
                  {/* 状态列定宽——badge 长短不一会把内容列起点挤歪（用户红笔标注） */}
                  <span className="mt-1 w-20 shrink-0">
                    <StatusBadge status={a.status} />
                  </span>
                  <div className="min-w-0 flex-1">
                    <p className="text-sm">{a.content}</p>
                    <p className="mt-0.5 font-mono text-xs text-muted-foreground/70">
                      {KIND_LABEL[a.kind] ?? a.kind} · {relTime(a.created_at)} · 置信 {a.confidence.toFixed(2)}
                    </p>
                  </div>
                  <button
                    type="button"
                    aria-label={`摘除原子：${a.content.slice(0, 12)}…`}
                    title="摘除关联（原子保留）"
                    className="shrink-0 rounded px-1 text-muted-foreground/0 transition-colors hover:text-destructive group-hover:text-muted-foreground/60"
                    onClick={async () => {
                      await api.del(`/memory/entities/${entity.id}/atoms/${a.id}`)
                      setDetail({
                        ...detail,
                        atoms: atoms.filter((x) => x.id !== a.id),
                        entity: { ...entity, atom_count: entity.atom_count - 1 },
                      })
                      onMutated()
                    }}
                  >
                    ×
                  </button>
                </li>
              ))}
            </ul>
          )}
          <p className="mt-2 text-xs text-muted-foreground/60">
            想看全部原子？<button type="button" className="underline underline-offset-4 hover:text-foreground" onClick={onGoAtoms}>去原子 tab</button>
          </p>
        </section>
      </div>
    </Card>
  )
}

function AttachForm({ entityId, exclude, onDone }: { entityId: string; exclude: string[]; onDone: () => void }) {
  const [pool, setPool] = useState<Atom[] | null>(null)
  const [pick, setPick] = useState('')
  const [err, setErr] = useState('')
  useEffect(() => {
    api.get<Atom[]>('/memory/atoms?limit=100').then(setPool).catch(() => setPool([]))
  }, [])
  if (!pool) return <p className="text-xs text-muted-foreground">加载原子池…</p>
  const candidates = pool.filter((a) => !exclude.includes(a.id))
  return (
    <form
      className="mb-3 flex flex-wrap items-center gap-2 rounded-lg border border-border bg-muted/30 p-2.5"
      onSubmit={async (e) => {
        e.preventDefault()
        if (!pick) return
        try {
          await api.post(`/memory/entities/${entityId}/atoms/${pick}`)
          onDone()
        } catch (ex) {
          setErr(ex instanceof Error ? ex.message : '挂链失败')
        }
      }}
    >
      <select className={`${selectCls} min-w-64 flex-1`} value={pick} onChange={(e) => setPick(e.target.value)}>
        <option value="">选择要挂的原子（最近 100 条）…</option>
        {candidates.map((a) => (
          <option key={a.id} value={a.id}>
            [{KIND_LABEL[a.kind] ?? a.kind}] {a.content.slice(0, 30)}
          </option>
        ))}
      </select>
      <Button size="sm" type="submit" disabled={!pick}>
        挂链
      </Button>
      {err && <p className="w-full text-xs text-destructive">{err}</p>}
    </form>
  )
}

function MergeForm({ entity, onDone }: { entity: EntityNode; onDone: () => void }) {
  const [others, setOthers] = useState<EntityNode[] | null>(null)
  const [into, setInto] = useState('')
  const [err, setErr] = useState('')
  useEffect(() => {
    api.get<EntityNode[]>('/memory/entities').then(setOthers).catch(() => setOthers([]))
  }, [])
  if (!others) return <p className="text-xs text-muted-foreground">加载实体…</p>
  const candidates = others.filter((o) => o.id !== entity.id)
  return (
    <form
      className="mb-3 flex flex-wrap items-center gap-2 rounded-lg border border-warning/30 bg-warning/10 p-2.5"
      onSubmit={async (e) => {
        e.preventDefault()
        if (!into) return
        if (
          !(await appConfirm({
            title: '确认合并实体？',
            description: `把「${entity.name}」合并进「${others.find((o) => o.id === into)?.name}」——原子关联全部转移，本实体退场。`,
            destructive: true,
            confirmLabel: '合并',
          }))
        )
          return
        try {
          await api.post(`/memory/entities/${entity.id}/merge`, { into })
          onDone()
        } catch (ex) {
          setErr(ex instanceof Error ? ex.message : '合并失败')
        }
      }}
    >
      <span className="text-xs text-warning">合并进：</span>
      <select className={`${selectCls} min-w-48 flex-1`} value={into} onChange={(e) => setInto(e.target.value)}>
        <option value="">选择目标实体…</option>
        {candidates.map((o) => (
          <option key={o.id} value={o.id}>
            {o.name}（{KIND_LABEL[o.kind] ?? o.kind}）
          </option>
        ))}
      </select>
      <Button size="sm" type="submit" disabled={!into}>
        合并
      </Button>
      {err && <p className="w-full text-xs text-destructive">{err}</p>}
    </form>
  )
}

function RelationForm({ entityId, allNodes, onDone }: { entityId: string; allNodes: EntityNode[]; onDone: () => void }) {
  const [to, setTo] = useState('')
  const [relType, setRelType] = useState('member_of')
  const [err, setErr] = useState('')
  return (
    <form
      className="mb-3 flex flex-wrap items-center gap-2 rounded-lg border border-border bg-muted/30 p-2.5"
      onSubmit={async (e) => {
        e.preventDefault()
        if (!to) return
        try {
          await api.post(`/memory/entities/${entityId}/relations`, { to_id: to, rel_type: relType })
          onDone()
        } catch (ex) {
          setErr(ex instanceof Error ? ex.message : '建关系失败')
        }
      }}
    >
      <select className={`${selectCls} min-w-48 flex-1`} value={to} onChange={(e) => setTo(e.target.value)}>
        <option value="">选择目标实体…</option>
        {allNodes
          .filter((n) => n.id !== entityId)
          .map((n) => (
            <option key={n.id} value={n.id}>
              {n.name}（{KIND_LABEL[n.kind] ?? n.kind}）
            </option>
          ))}
      </select>
      <select className={`${selectCls} w-28`} value={relType} onChange={(e) => setRelType(e.target.value)}>
        {Object.entries(REL_TYPE_LABEL).map(([k, label]) => (
          <option key={k} value={k}>
            {label}
          </option>
        ))}
      </select>
      <Button size="sm" type="submit" disabled={!to}>
        建关系
      </Button>
      {err && <p className="w-full text-xs text-destructive">{err}</p>}
    </form>
  )
}

function TimelineView() {
  const [events, setEvents] = useState<TimelineEvent[] | null>(null)
  useEffect(() => {
    api.get<TimelineEvent[]>('/memory/timeline?limit=100').then(setEvents).catch(() => setEvents([]))
  }, [])
  if (!events) return <div className="min-h-0 flex-1 animate-pulse rounded-lg bg-muted/30" />
  if (events.length === 0) return <Empty text="暂无记忆事件——蒸馏后这里会形成时间脉络" />
  const kindLabel: Record<string, string> = { atom: '原子', scenario: '场景', entity: '实体' }
  return (
    <div className="min-h-0 flex-1 overflow-y-auto rounded-lg border border-border bg-card">
      <ul className="divide-y divide-border/60">
        {events.map((e) => (
          <li key={`${e.kind}-${e.id}`} className="flex items-start gap-3 px-4 py-2.5">
            <span className="mt-0.5 w-10 shrink-0 font-mono text-xs text-muted-foreground">{kindLabel[e.kind] ?? e.kind}</span>
            <div className="min-w-0 flex-1">
              <p className="text-sm">{e.content}</p>
              <p className="mt-0.5 font-mono text-xs text-muted-foreground/70">{relTime(e.at)}</p>
            </div>
          </li>
        ))}
      </ul>
    </div>
  )
}

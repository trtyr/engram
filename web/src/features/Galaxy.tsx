/**
 * 圈子 tab：左实体列表（搜索/类型过滤/密度排序/新建）+ 右侧默认关系图谱，
 * 点击节点或列表项进入实体详情（画像摘要/原子时间线/相关场景/挂摘/合并）。
 */
import { Suspense, lazy, useEffect, useState } from 'react'
import { Search } from 'lucide-react'
import {
  api,
  type Atom,
  type EntityDetail,
  type EntityGraph,
  type EntityNode,
} from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Card, Empty, ErrorBox, Spinner, StatusBadge } from '@/components/ui-bits'
import { ENTITY_KIND_COLOR as KIND_COLOR } from '@/lib/ui'
import { relTime, inputCls, selectCls } from '@/lib/ui'
import { cn } from '@/lib/utils'

const EntityGalaxy = lazy(() => import('@/components/EntityGalaxy'))

const KIND_LABEL: Record<string, string> = {
  person: '人物',
  project: '项目',
  topic: '主题',
  group: '群组',
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

  const load = () =>
    api
      .get<EntityGraph>('/memory/entities/graph')
      .then(setGraph)
      .catch((e) => setErr(e instanceof Error ? e.message : String(e)))
  useEffect(() => {
    load()
  }, [])

  if (err) return <ErrorBox msg={err} />
  if (!graph) return <Spinner />

  const visible = graph.nodes.filter(
    (n) =>
      (!kind || n.kind === kind) &&
      (!q || n.name.toLowerCase().includes(q.toLowerCase()) || n.summary.includes(q)),
  )

  return (
    <div className="flex flex-col gap-4 lg:flex-row">
      {/* 左：实体列表——清点与管理 */}
      <Card className="overflow-hidden lg:w-80 lg:shrink-0 lg:self-start">
        <div className="flex items-center gap-2 border-b border-border px-3 py-2">
          <span className="font-mono text-xs text-muted-foreground">{graph.nodes.length} 实体</span>
          <div className="relative ml-auto">
            <Search className="pointer-events-none absolute left-2 top-1/2 size-3 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
            <input
              className="w-32 rounded-md border border-border bg-card py-1 pl-7 pr-2 text-xs outline-none transition-colors placeholder:text-muted-foreground/60 focus-visible:border-foreground/40"
              placeholder="搜索实体…"
              value={q}
              onChange={(e) => setQ(e.target.value)}
            />
          </div>
        </div>
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
          <ul className="max-h-64 divide-y divide-border/60 overflow-auto lg:max-h-[calc(100vh-20rem)]">
            {visible.map((n) => (
              <li key={n.id}>
                <button
                  type="button"
                  onClick={() => setSelected(n.id)}
                  aria-current={n.id === selected ? 'true' : undefined}
                  className={cn(
                    'w-full px-3 py-2 text-left transition-colors',
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
              </li>
            ))}
          </ul>
        )}
      </Card>

      {/* 右：默认关系图谱，选中即详情 */}
      <div className="min-w-0 flex-1">
        {selected ? (
          <EntityDetailPane
            key={selected}
            entityId={selected}
            onBack={() => setSelected(null)}
            onMutated={load}
            onGoAtoms={onGoAtoms}
          />
        ) : visible.length === 0 && graph.nodes.length === 0 ? (
          <Card className="p-4">
            <Empty text="圈子是你的记忆世界：人物 / 项目 / 主题。蒸馏会自动把对话里的人和事挂进来，也可以先手动新建" />
          </Card>
        ) : (
          <div className="space-y-3">
            <p className="text-xs leading-relaxed text-muted-foreground">
              每个圆点是你记忆里的一个人 / 项目 / 主题（<span className="text-foreground">颜色 = 类型，大小 = 相关记忆条数</span>），
              连线 = 它们在你的记忆里<span className="text-foreground">一起出现</span>（越粗越常见，会自动抱团）；
              中间的「我」是你——点圆点看档案，<span className="text-foreground">按住可拖动</span>，滚轮缩放。
              <span className="ml-2 inline-flex flex-wrap items-center gap-2 align-middle">
                {Object.entries(KIND_LABEL).map(([k, label]) => (
                  <span key={k} className="flex items-center gap-1 font-mono">
                    <span className="inline-block size-2 rounded-full" style={{ backgroundColor: KIND_COLOR[k] }} aria-hidden="true" />
                    {label}
                  </span>
                ))}
              </span>
            </p>
            <Suspense fallback={<div className="h-[60vh] w-full animate-pulse rounded-lg bg-muted/30" />}>
              <EntityGalaxy
                graph={{ nodes: visible, edges: graph.edges }}
                onSelect={setSelected}
                onGoPersona={onGoPersona}
              />
            </Suspense>
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
}: {
  entityId: string
  onBack: () => void
  onMutated: () => void
  onGoAtoms: () => void
}) {
  const [detail, setDetail] = useState<EntityDetail | null>(null)
  const [err, setErr] = useState('')
  const [attachOpen, setAttachOpen] = useState(false)
  const [mergeOpen, setMergeOpen] = useState(false)

  useEffect(() => {
    api
      .get<EntityDetail>(`/memory/entities/${entityId}`)
      .then(setDetail)
      .catch((e) => setErr(e instanceof Error ? e.message : String(e)))
  }, [entityId])

  if (err) return <ErrorBox msg={err} />
  if (!detail) return <Spinner />
  const { entity, atoms, scenarios } = detail

  return (
    <Card className="overflow-hidden">
      <div className="border-b border-border px-4 py-3">
        <div className="flex flex-wrap items-start justify-between gap-2">
          <div className="min-w-0">
            <h2 className="flex items-center gap-2 text-base font-semibold tracking-tight">
              <span className="inline-block size-2.5 rounded-full" style={{ backgroundColor: KIND_COLOR[entity.kind] }} aria-hidden="true" />
              {entity.name}
              <span className="rounded border border-border px-1.5 py-px font-mono text-xs font-normal text-muted-foreground">
                {KIND_LABEL[entity.kind] ?? entity.kind}
              </span>
            </h2>
            <p className="mt-1 text-sm text-muted-foreground">{entity.summary || '尚无画像摘要——蒸馏积累后自动丰富'}</p>
            <p className="mt-1 font-mono text-xs text-muted-foreground/70">
              {entity.atom_count} 条原子 · 更新于 {relTime(entity.updated_at)}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-1.5">
            <Button variant="ghost" size="sm" onClick={onBack}>
              ← 返回图谱
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="hover:bg-destructive/10 hover:text-destructive"
              onClick={async () => {
                if (!confirm(`删除实体「${entity.name}」？原子本身保留，仅解除关联。`)) return
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

      <div className="space-y-5 px-4 py-4">
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
                  <span className="mt-1 shrink-0">
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
        if (!confirm(`把「${entity.name}」合并进「${others.find((o) => o.id === into)?.name}」？原子关联全部转移，本实体退场。`)) return
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

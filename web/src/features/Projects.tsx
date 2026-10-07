/**
 * 项目记忆域 · 外层项目选择页（2026-10-07 大改版）：
 * 这一层的职责只有一个——把「有哪些项目、该点进哪个」讲清楚。点击卡片才进项目详情，
 * 详情页才是文档阅读的地方（两层分工：外层选项目，内层读文档）。
 *
 * 卡片语言（GitHub repo / Vercel project 同族）：瓦片 + 名称 + 状态 + 描述两行 +
 * 分类 chips + 更新时间脚注；编辑/删除悬浮才显，复选框右上角常驻。
 * 新建是低频动作收进弹窗；场景分组 + 图谱 tab 保留。
 */
import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { api, type ProjectDto, type ProjectTypeDto } from '@/lib/api'
import ProjectAssetGraph from '@/components/ProjectAssetGraph'
import { Card, Checkbox, Empty, ErrorBox, PageHeader, Spinner, Tabs } from '@/components/ui-bits'
import { inputCls, selectCls, relTime } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { BookOpen, Code2, Coffee, FlaskConical, Hammer, Palette, PenTool } from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

const STATUS_META: Record<string, { label: string; cls: string }> = {
  active: { label: '进行中', cls: 'text-emerald-600' },
  paused: { label: '暂停', cls: 'text-amber-600' },
  done: { label: '完成', cls: 'text-muted-foreground' },
  abandoned: { label: '放弃', cls: 'text-destructive' },
}

/** 场景图标（标签文字由 `/projects/types` 提供——场景值域的单一事实源在后端 `PROJECT_TYPES` 常量）。供项目外层与详情页共用。 */
export const TYPE_ICON: Record<string, { icon: LucideIcon; color: string }> = {
  dev: { icon: Code2, color: '#3b82f6' },
  ops: { icon: Hammer, color: '#10b981' },
  research: { icon: FlaskConical, color: '#8b5cf6' },
  study: { icon: BookOpen, color: '#f59e0b' },
  life: { icon: Coffee, color: '#06b6d4' },
  create: { icon: PenTool, color: '#ec4899' },
}

/** 场景图标瓦片：分组标题与卡片的视觉锚点。 */
export function TypeTile({ type, size = 'md' }: { type: string; size?: 'sm' | 'md' | 'lg' }) {
  const spec = TYPE_ICON[type] ?? { icon: Palette, color: '#6b7280' }
  const Icon = spec.icon
  return (
    <span
      className={cn(
        'flex shrink-0 items-center justify-center rounded-md',
        size === 'sm' ? 'size-5' : size === 'lg' ? 'size-11' : 'size-9',
      )}
      style={{ background: `${spec.color}1f`, color: spec.color }}
    >
      <Icon className={size === 'sm' ? 'size-3' : size === 'lg' ? 'size-5' : 'size-4'} aria-hidden="true" />
    </span>
  )
}

export default function Projects() {
  const [rows, setRows] = useState<ProjectDto[] | null>(null)
  const [types, setTypes] = useState<ProjectTypeDto[]>([])
  const [filter, setFilter] = useState('')
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)
  /** 列表 / 图谱（共享引擎第四位住户：项目 ↔ 资产关系网） */
  const [tab, setTab] = useState<'list' | 'graph'>('list')
  // 新建弹窗（低频动作不常驻页面）
  const [showCreate, setShowCreate] = useState(false)

  // 编辑表单（卡片内联展开）
  const [editId, setEditId] = useState<string | null>(null)
  const [editName, setEditName] = useState('')
  const [editStatus, setEditStatus] = useState('active')
  const [editDesc, setEditDesc] = useState('')
  const [editCats, setEditCats] = useState('')

  const load = () =>
    api
      .get<ProjectDto[]>(`/projects${filter ? `?type=${filter}` : ''}`)
      .then(setRows)
      .catch((e) => setErr(e.message))

  useEffect(() => {
    api.get<ProjectTypeDto[]>('/projects/types').then(setTypes).catch(() => {})
    load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filter])

  const toggle = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const toggleAll = () => {
    setSelected((prev) => {
      if (prev.size === (rows?.length ?? 0) && rows && rows.length > 0) return new Set()
      return new Set((rows ?? []).map((r) => r.id))
    })
  }

  async function doDelete(id: string) {
    setBusy(true)
    try {
      await api.del(`/projects/${id}`)
      setSelected((prev) => {
        const next = new Set(prev)
        next.delete(id)
        return next
      })
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '删除失败')
    } finally {
      setBusy(false)
    }
  }

  async function doBatchDelete() {
    if (selected.size === 0) return
    setBusy(true)
    try {
      await api.post('/projects/batch-delete', { ids: [...selected] })
      setSelected(new Set())
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '批量删除失败')
    } finally {
      setBusy(false)
    }
  }

  function startEdit(p: ProjectDto) {
    setEditId(p.id)
    setEditName(p.name)
    setEditStatus(p.status)
    setEditDesc(p.description ?? '')
    setEditCats(p.categories.join(', '))
  }

  async function doEdit() {
    if (!editId || !editName.trim()) return
    setBusy(true)
    try {
      await api.put(`/projects/${editId}`, {
        name: editName.trim(),
        status: editStatus,
        description: editDesc.trim() || null,
        categories: editCats.split(',').map((s) => s.trim()).filter(Boolean),
      })
      setEditId(null)
      setErr('')
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '保存失败')
    } finally {
      setBusy(false)
    }
  }

  if (err && !rows) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  // 场景分组（未筛选时）：同一场景的项目聚在一起，边界看得见（README §2.5「混杂病在展示层治」）
  const grouped: [string, ProjectDto[]][] = (() => {
    const order = types.map((t) => t.type)
    const bucket = new Map<string, ProjectDto[]>()
    for (const p of rows) {
      const list = bucket.get(p.type) ?? []
      list.push(p)
      bucket.set(p.type, list)
    }
    const keys = [...bucket.keys()].sort((a, b) => {
      const ia = order.indexOf(a)
      const ib = order.indexOf(b)
      return (ia < 0 ? 999 : ia) - (ib < 0 ? 999 : ib)
    })
    return keys.map((k) => [k, bucket.get(k) ?? []] as [string, ProjectDto[]])
  })()

  return (
    <div className="space-y-5">
      <PageHeader title="项目" desc={`${rows.length} 个项目`}>
        <Button onClick={() => setShowCreate(true)}>＋ 新建项目</Button>
      </PageHeader>

      {/* 文档库 / 图谱 切换（全局 Tabs 同款；图谱 = 共享引擎第四位住户：项目 ↔ 资产关系网） */}
      <div className="flex flex-wrap items-center gap-4">
        <Tabs
          items={[
            { value: 'list', label: '文档库' },
            { value: 'graph', label: '图谱' },
          ]}
          value={tab}
          onChange={(v) => setTab(v as 'list' | 'graph')}
        />
        <div className="ml-auto">
          <select className={selectCls} value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="类型筛选">
            <option value="">全部类型</option>
            {types.map((t) => (
              <option key={t.type} value={t.type}>
                {t.label}
              </option>
            ))}
          </select>
        </div>
      </div>

      {tab === 'graph' ? (
        <ProjectAssetGraph />
      ) : (
        <>
          {err && <ErrorBox msg={err} />}

          {/* 批量操作条（选中时出现） */}
          {selected.size > 0 && (
            <div className="flex items-center gap-3 rounded-lg border border-border bg-muted/30 px-3 py-2">
              <span className="text-sm text-muted-foreground">已选 {selected.size} 项</span>
              <Button size="sm" variant="ghost" onClick={toggleAll}>
                {rows.length > 0 && selected.size === rows.length ? '取消全选' : '全选'}
              </Button>
              <Button size="sm" variant="destructive" disabled={busy} onClick={doBatchDelete}>
                批量删除
              </Button>
              <Button size="sm" variant="ghost" onClick={() => setSelected(new Set())}>
                取消选择
              </Button>
            </div>
          )}

          {rows.length === 0 ? (
            <Empty text="暂无项目——让 AI 通过 MCP projects create 建第一个项目" />
          ) : (
            <div className="space-y-7">
              {grouped.map(([t, items]) => (
                <section key={t} className="space-y-2.5">
                  <h2 className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
                    <TypeTile type={t} size="sm" />
                    {types.find((x) => x.type === t)?.label ?? t}
                    <span className="text-muted-foreground/50">（{items.length}）</span>
                  </h2>
                  <div className="grid gap-3 md:grid-cols-2 2xl:grid-cols-3">
                    {items.map((p) => (
                      <Card
                        key={p.id}
                        className="group relative flex flex-col p-4 transition-all hover:-translate-y-px hover:border-foreground/25 hover:shadow-md"
                      >
                        {/* 右上角轻操作区：编辑/删除悬浮才显，复选框常驻 */}
                        <div className="absolute right-3 top-3 flex items-center gap-1">
                          <div className="flex gap-1 opacity-0 transition-opacity group-hover:opacity-100">
                            <Button size="sm" variant="ghost" onClick={() => startEdit(p)}>
                              编辑
                            </Button>
                            <Button size="sm" variant="ghost" disabled={busy} onClick={() => doDelete(p.id)}>
                              删除
                            </Button>
                          </div>
                          <Checkbox
                            checked={selected.has(p.id)}
                            onChange={() => toggle(p.id)}
                            label={`选择 ${p.name}`}
                          />
                        </div>

                        {/* 身份行：瓦片 + 名称 + 状态 */}
                        <div className="flex items-start gap-3 pr-16">
                          <TypeTile type={p.type} />
                          <div className="min-w-0 flex-1">
                            <Link
                              to={`/projects/${p.id}`}
                              className="block truncate font-medium leading-6 hover:underline"
                              title={p.name}
                            >
                              {p.name}
                            </Link>
                            <StatusDot status={p.status} />
                          </div>
                        </div>

                        {/* 描述（两行截断） */}
                        <p className="mt-2 line-clamp-2 min-h-8 text-xs leading-5 text-muted-foreground">
                          {p.description || '（无描述）'}
                        </p>

                        {/* 脚注：分类 chips + 更新时间 */}
                        <div className="mt-auto flex items-center gap-2 pt-3">
                          <div className="flex min-w-0 flex-1 flex-wrap gap-1">
                            {p.categories.slice(0, 3).map((c) => (
                              <span
                                key={c}
                                className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground"
                              >
                                {c}
                              </span>
                            ))}
                            {p.categories.length > 3 && (
                              <span className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground/60">
                                +{p.categories.length - 3}
                              </span>
                            )}
                          </div>
                          <span className="shrink-0 text-[10px] text-muted-foreground/60" title={p.updated_at}>
                            {relTime(p.updated_at)}
                          </span>
                        </div>

                        {/* 编辑表单（内联展开） */}
                        {editId === p.id && (
                          <div className="mt-3 space-y-2 border-t border-border/60 pt-3">
                            <input
                              className={`${inputCls} w-full`}
                              value={editName}
                              onChange={(e) => setEditName(e.target.value)}
                            />
                            <div className="flex gap-2">
                              <select
                                className={selectCls}
                                value={editStatus}
                                onChange={(e) => setEditStatus(e.target.value)}
                              >
                                {Object.entries(STATUS_META).map(([k, v]) => (
                                  <option key={k} value={k}>
                                    {v.label}
                                  </option>
                                ))}
                              </select>
                              <input
                                className={`${inputCls} flex-1`}
                                placeholder="描述（可选）"
                                value={editDesc}
                                onChange={(e) => setEditDesc(e.target.value)}
                              />
                            </div>
                            <input
                              className={`${inputCls} w-full`}
                              placeholder="分类（逗号分隔，如：后端, 前端, 运维）"
                              value={editCats}
                              onChange={(e) => setEditCats(e.target.value)}
                            />
                            <div className="flex gap-2">
                              <Button size="sm" disabled={busy} onClick={doEdit}>
                                保存
                              </Button>
                              <Button size="sm" variant="ghost" onClick={() => setEditId(null)}>
                                取消
                              </Button>
                            </div>
                          </div>
                        )}
                      </Card>
                    ))}
                  </div>
                </section>
              ))}
            </div>
          )}
        </>
      )}

      {showCreate && (
        <CreateProjectDialog
          types={types}
          onClose={() => setShowCreate(false)}
          onCreated={() => {
            setShowCreate(false)
            load()
          }}
          onError={setErr}
        />
      )}
    </div>
  )
}

/** 新建项目弹窗：低频动作不常驻页面（与凭据/工单同款自建模态）。 */
function CreateProjectDialog({
  types,
  onClose,
  onCreated,
  onError,
}: {
  types: ProjectTypeDto[]
  onClose: () => void
  onCreated: () => void
  onError: (m: string) => void
}) {
  const [name, setName] = useState('')
  const [type, setType] = useState('dev')
  const [desc, setDesc] = useState('')
  const [busy, setBusy] = useState(false)

  async function submit() {
    if (!name.trim()) return
    setBusy(true)
    try {
      await api.post('/projects', { name: name.trim(), type, description: desc.trim() || null })
      onCreated()
    } catch (e) {
      onError(e instanceof Error ? e.message : '新建失败')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      onClick={onClose}
      role="presentation"
    >
      <Card
        className="w-full max-w-md space-y-3 p-4"
        onClick={(e: React.MouseEvent) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2">
          <TypeTile type={type} />
          <span className="text-sm font-medium">新建项目</span>
        </div>
        <input
          className={inputCls + ' w-full'}
          placeholder="项目名"
          value={name}
          onChange={(e) => setName(e.target.value)}
          autoFocus
        />
        <div className="flex gap-2">
          <select
            className={selectCls + ' min-w-0 flex-1'}
            value={type}
            onChange={(e) => setType(e.target.value)}
            aria-label="场景"
          >
            {types.map((t) => (
              <option key={t.type} value={t.type}>
                {t.label}
              </option>
            ))}
          </select>
        </div>
        <input
          className={inputCls + ' w-full'}
          placeholder="描述（可选）"
          value={desc}
          onChange={(e) => setDesc(e.target.value)}
        />
        <div className="flex items-center justify-end gap-2">
          <Button variant="outline" onClick={onClose}>
            取消
          </Button>
          <Button disabled={busy || !name.trim()} onClick={submit}>
            新建
          </Button>
        </div>
      </Card>
    </div>
  )
}

/** project 状态徽章（独立映射：active=进行中，与 atom 的 active=生效 语义不同）。 */
function StatusDot({ status }: { status: string }) {
  const meta = STATUS_META[status] ?? { label: status, cls: 'text-muted-foreground' }
  return (
    <span className={`shrink-0 whitespace-nowrap text-xs ${meta.cls}`} title={status}>
      ● {meta.label}
    </span>
  )
}

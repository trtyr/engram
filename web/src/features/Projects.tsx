/** 项目记忆域：项目列表（类型筛选 / 新建 / 编辑 / 多选 / 批量删除）。 */
import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { api, type ProjectDto, type ProjectTypeDto } from '@/lib/api'
import ProjectAssetGraph from '@/components/ProjectAssetGraph'
import { Card, Checkbox, Empty, ErrorBox, PageHeader, Spinner } from '@/components/ui-bits'
import { inputCls, selectCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'

/**
 * 场景色点（标签文字由 `/projects/types` 提供——场景值域的单一事实源在后端 `PROJECT_TYPES` 常量）。
 * 未知场景回落灰点，不猜语义。
 */
const TYPE_DOT: Record<string, string> = {
  dev: '#3b82f6', // 开发
  ops: '#10b981', // 运维
  research: '#8b5cf6', // 调研
  study: '#f59e0b', // 学习
  life: '#06b6d4', // 生活
  create: '#ec4899', // 创作
}
const STATUS_META: Record<string, { label: string; cls: string }> = {
  active: { label: '进行中', cls: 'text-emerald-600' },
  paused: { label: '暂停', cls: 'text-amber-600' },
  done: { label: '完成', cls: 'text-muted-foreground' },
  abandoned: { label: '放弃', cls: 'text-destructive' },
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

  // 新建表单
  const [name, setName] = useState('')
  const [type, setType] = useState('dev')
  const [desc, setDesc] = useState('')

  // 编辑表单（内联）
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

  async function doCreate() {
    if (!name.trim()) return
    setBusy(true)
    try {
      await api.post('/projects', { name: name.trim(), type, description: desc.trim() || null })
      setName('')
      setDesc('')
      setErr('')
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '新建失败')
    } finally {
      setBusy(false)
    }
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
      <PageHeader title="项目" desc="项目记忆域：跨会话的工作上下文——类型驱动分类，位置 + 文档 + 规划。">
        <select className={selectCls} value={filter} onChange={(e) => setFilter(e.target.value)}>
          <option value="">全部类型</option>
          {types.map((t) => (
            <option key={t.type} value={t.type}>
              {t.label}
            </option>
          ))}
        </select>
      </PageHeader>

      {/* 列表 / 图谱 切换（图谱 = 共享引擎第四位住户：项目 ↔ 资产关系网） */}
      <div className="flex items-center gap-1 border-b border-border">
        {(
          [
            ['list', '列表'],
            ['graph', '图谱'],
          ] as const
        ).map(([k, label]) => (
          <button
            key={k}
            type="button"
            onClick={() => setTab(k)}
            className={`-mb-px border-b-2 px-3 py-1.5 text-sm transition-colors ${
              tab === k
                ? 'border-foreground font-medium text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground'
            }`}
          >
            {label}
          </button>
        ))}
      </div>

      {tab === 'graph' ? (
        <ProjectAssetGraph />
      ) : (
        <>
          {/* 新建 + 全选（同一行） */}
      <Card className="p-3">
        <div className="flex flex-wrap items-center gap-3">
          {rows.length > 1 && (
            <Checkbox
              checked={selected.size === rows.length && rows.length > 0}
              onChange={() => toggleAll()}
              label="全选"
            >
              {selected.size === rows.length ? '取消全选' : '全选'}
            </Checkbox>
          )}
          <form
            className="flex flex-wrap items-center gap-2"
            onSubmit={(e) => {
              e.preventDefault()
              doCreate()
            }}
          >
            <input
              className={`${inputCls} w-52`}
              placeholder="项目名"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
            <select className={selectCls} value={type} onChange={(e) => setType(e.target.value)}>
              {types.map((t) => (
                <option key={t.type} value={t.type}>
                  {t.label}
                </option>
              ))}
            </select>
            <input
              className={`${inputCls} flex-1`}
              placeholder="描述（可选）"
              value={desc}
              onChange={(e) => setDesc(e.target.value)}
            />
            <Button size="sm" type="submit" disabled={busy || !name.trim()}>
              新建
            </Button>
          </form>
        </div>
      </Card>

      {err && <ErrorBox msg={err} />}

      {/* 批量操作条（选中时出现） */}
      {selected.size > 0 && (
        <div className="flex items-center gap-3 rounded-lg border border-border bg-muted/30 px-3 py-2">
          <span className="text-sm text-muted-foreground">已选 {selected.size} 项</span>
          <Button size="sm" variant="destructive" disabled={busy} onClick={doBatchDelete}>
            批量删除
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setSelected(new Set())}>
            取消选择
          </Button>
        </div>
      )}

      {rows.length === 0 ? (
        <Empty text="暂无项目" />
      ) : (
        <div className="space-y-6">
          {grouped.map(([t, items]) => (
            <section key={t} className="space-y-2">
              <h2 className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
                <i
                  className="size-2 rounded-full"
                  style={{ background: TYPE_DOT[t] ?? '#6b7280' }}
                  aria-hidden="true"
                />
                {types.find((x) => x.type === t)?.label ?? t}（{items.length}）
              </h2>
              <div className="grid gap-3 md:grid-cols-2">
                {items.map((p) => (
            <Card key={p.id} className="p-4">
              <div className="flex items-start gap-3">
                <Checkbox
                  className="mt-1"
                  checked={selected.has(p.id)}
                  onChange={() => toggle(p.id)}
                  label={`选择 ${p.name}`}
                />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <Link to={`/projects/${p.id}`} className="truncate font-medium hover:underline">
                      {p.name}
                    </Link>
                    <span className="flex shrink-0 items-center gap-1 rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">
                      <i
                        className="size-1.5 rounded-full"
                        style={{ background: TYPE_DOT[p.type] ?? '#6b7280' }}
                        aria-hidden="true"
                      />
                      {types.find((t) => t.type === p.type)?.label ?? p.type}
                    </span>
                    <StatusDot status={p.status} />
                  </div>
                  {p.description && (
                    <p className="mt-1 line-clamp-1 text-xs text-muted-foreground">{p.description}</p>
                  )}
                  {p.categories.length > 0 && (
                    <div className="mt-2 flex flex-wrap gap-1">
                      {p.categories.map((c) => (
                        <span
                          key={c}
                          className="rounded border border-border px-1.5 py-0.5 text-[11px] text-muted-foreground"
                        >
                          {c}
                        </span>
                      ))}
                    </div>
                  )}
                </div>
                <div className="flex shrink-0 gap-1">
                  <Button size="sm" variant="ghost" onClick={() => startEdit(p)}>
                    编辑
                  </Button>
                  <Button size="sm" variant="ghost" disabled={busy} onClick={() => doDelete(p.id)}>
                    删除
                  </Button>
                </div>
              </div>

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

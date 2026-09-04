/** 项目记忆域：项目列表（类型筛选 / 新建 / 编辑 / 多选 / 批量删除）。 */
import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { api, type ProjectDto, type ProjectTypeDto } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner } from '@/components/ui-bits'
import { inputCls, selectCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'

const TYPE_LABEL: Record<string, string> = { dev: '开发', research: '调研' }
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

  return (
    <div className="space-y-5">
      <PageHeader title="项目" desc="项目记忆第五域：跨会话的工作上下文——类型驱动分类，位置 + 文档 + 规划。">
        <select className={selectCls} value={filter} onChange={(e) => setFilter(e.target.value)}>
          <option value="">全部类型</option>
          {types.map((t) => (
            <option key={t.type} value={t.type}>
              {t.label}
            </option>
          ))}
        </select>
      </PageHeader>

      {/* 新建 */}
      <Card className="p-3">
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
      </Card>

      {err && <ErrorBox msg={err} />}

      {/* 批量操作条 */}
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
        <div className="grid gap-3 md:grid-cols-2">
          {rows.map((p) => (
            <Card key={p.id} className="p-4">
              <div className="flex items-start gap-3">
                <input
                  type="checkbox"
                  className="mt-1"
                  checked={selected.has(p.id)}
                  onChange={() => toggle(p.id)}
                />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <Link to={`/projects/${p.id}`} className="truncate font-medium hover:underline">
                      {p.name}
                    </Link>
                    <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">
                      {TYPE_LABEL[p.type] ?? p.type}
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

          {/* 全选（放最后一行底部） */}
          {rows.length > 1 && (
            <button
              type="button"
              className="flex items-center gap-2 self-start text-xs text-muted-foreground hover:text-foreground"
              onClick={toggleAll}
            >
              <input
                type="checkbox"
                readOnly
                checked={selected.size === rows.length && rows.length > 0}
              />
              {selected.size === rows.length ? '取消全选' : '全选'}
            </button>
          )}
        </div>
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

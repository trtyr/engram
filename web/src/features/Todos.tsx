/** 待办域（第七域）：不绑定项目的临时任务/灵感速记——速记→做完勾掉。 */
import { useEffect, useMemo, useState } from 'react'
import { Check, Trash2 } from 'lucide-react'
import { api, type Todo } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner } from '@/components/ui-bits'
import { inputCls, selectCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

const PRIO_LABEL: Record<string, string> = { low: '低', normal: '普通', high: '高' }
const PRIO_CLASS: Record<string, string> = {
  high: 'border-destructive/40 text-destructive',
  normal: 'border-border text-muted-foreground',
  low: 'border-border text-muted-foreground/70',
}

type StatusFilter = '' | 'open' | 'done' | 'archived'

export default function Todos() {
  const [rows, setRows] = useState<Todo[] | null>(null)
  const [status, setStatus] = useState<StatusFilter>('open')
  const [priority, setPriority] = useState('')
  const [tag, setTag] = useState('')
  const [q, setQ] = useState('')
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)

  // 快速输入条
  const [title, setTitle] = useState('')
  const [priority, setPriority] = useState('normal')
  const [projectHint, setProjectHint] = useState('')

  const query = useMemo(() => {
    const parts: string[] = []
    if (status) parts.push(`status=${status}`)
    if (priority) parts.push(`priority=${priority}`)
    if (tag.trim()) parts.push(`tag=${encodeURIComponent(tag.trim())}`)
    if (q.trim()) parts.push(`q=${encodeURIComponent(q.trim())}`)
    return parts.join('&')
  }, [status, priority, tag, q])

  const load = () =>
    api
      .get<Todo[]>(`/todos${query ? `?${query}` : ''}`)
      .then((r) => {
        setRows(r)
        setErr('')
      })
      .catch((e) => setErr(e.message))

  useEffect(() => {
    load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query])

  async function quickAdd() {
    if (!title.trim()) return
    setBusy(true)
    try {
      await api.post('/todos', {
        title: title.trim(),
        priority,
        project_hint: projectHint.trim() || undefined,
      })
      setTitle('')
      setProjectHint('')
      setErr('')
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '创建失败')
    } finally {
      setBusy(false)
    }
  }

  async function toggleDone(t: Todo) {
    setBusy(true)
    try {
      await api.put(`/todos/${t.id}`, { status: t.status === 'done' ? 'open' : 'done' })
      setErr('')
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '操作失败')
    } finally {
      setBusy(false)
    }
  }

  async function doArchive(t: Todo) {
    setBusy(true)
    try {
      await api.put(`/todos/${t.id}`, { status: 'archived' })
      setErr('')
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '归档失败')
    } finally {
      setBusy(false)
    }
  }

  async function doDelete(t: Todo) {
    if (!confirm(`删除待办「${t.title}」？不可恢复。`)) return
    setBusy(true)
    try {
      await api.del(`/todos/${t.id}`)
      setErr('')
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '删除失败')
    } finally {
      setBusy(false)
    }
  }

  if (err && !rows) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  const open = rows.filter((t) => t.status === 'open')
  const finished = rows.filter((t) => t.status !== 'open')

  return (
    <div className="space-y-5">
      <PageHeader title="待办" desc="第七域：不绑定项目的临时任务/灵感速记——速记、做完勾掉">
        <input
          className={`${inputCls} w-44`}
          placeholder="搜标题/详情…"
          aria-label="搜索待办"
          value={q}
          onChange={(e) => setQ(e.target.value)}
        />
        <select
          className={selectCls}
          aria-label="优先级筛选"
          value={priority}
          onChange={(e) => setPriority(e.target.value)}
        >
          <option value="">全部优先级</option>
          <option value="high">高</option>
          <option value="normal">普通</option>
          <option value="low">低</option>
        </select>
        <select
          className={selectCls}
          aria-label="标签筛选"
          value={tag}
          onChange={(e) => setTag(e.target.value)}
        >
          <option value="">全部标签</option>
          <option value="灵感">灵感</option>
          <option value="学习">学习</option>
          <option value="系统操作">系统操作</option>
          <option value="问题排查">问题排查</option>
        </select>
      </PageHeader>

      {/* 快速输入条 */}
      <Card className="p-4">
        <div className="flex flex-wrap gap-2">
          <input
            className={`${inputCls} min-w-0 flex-1`}
            placeholder="记一条待办…（回车快速创建）"
            aria-label="待办标题"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault()
                quickAdd()
              }
            }}
          />
          <select
            className={selectCls}
            aria-label="优先级"
            value={priority}
            onChange={(e) => setPriority(e.target.value)}
          >
            <option value="normal">普通</option>
            <option value="high">高</option>
            <option value="low">低</option>
          </select>
          <input
            className={`${inputCls} w-40`}
            placeholder="关联项目（可选）"
            aria-label="关联项目提示"
            value={projectHint}
            onChange={(e) => setProjectHint(e.target.value)}
          />
          <Button size="sm" disabled={busy || !title.trim()} onClick={quickAdd}>
            添加
          </Button>
        </div>
      </Card>

      {err && <ErrorBox msg={err} />}

      {/* 待办列表 */}
      {rows.length === 0 ? (
        <Empty text="暂无待办——上方输入框快速记一条，或让 AI 通过 todo_add 帮你记" />
      ) : (
        <div className="space-y-6">
          {/* 进行中 */}
          {open.length > 0 && (
            <div className="space-y-2">
              <h3 className="text-sm font-semibold">
                进行中 <span className="font-mono text-xs text-muted-foreground">{open.length}</span>
              </h3>
              <div className="space-y-2">
                {open.map((t) => (
                  <TodoRow key={t.id} t={t} busy={busy} onToggle={toggleDone} onArchive={doArchive} onDelete={doDelete} />
                ))}
              </div>
            </div>
          )}

          {/* 已完成/已归档 */}
          {finished.length > 0 && (
            <div className="space-y-2">
              <h3 className="text-sm font-semibold text-muted-foreground">
                已完成 / 归档{' '}
                <span className="font-mono text-xs text-muted-foreground">{finished.length}</span>
              </h3>
              <div className="space-y-2">
                {finished.map((t) => (
                  <TodoRow key={t.id} t={t} busy={busy} onToggle={toggleDone} onArchive={doArchive} onDelete={doDelete} />
                ))}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  )
}

/** 单条待办行：勾选完成 + 元信息 + 归档/删除。 */
function TodoRow({
  t,
  busy,
  onToggle,
  onArchive,
  onDelete,
}: {
  t: Todo
  busy: boolean
  onToggle: (t: Todo) => void
  onArchive: (t: Todo) => void
  onDelete: (t: Todo) => void
}) {
  const overdue = t.due_at && t.status === 'open' && new Date(t.due_at) < new Date()
  return (
    <Card
      className={cn(
        'flex items-start gap-3 p-3 transition-colors hover:bg-muted/40',
        t.status === 'done' && 'opacity-60',
      )}
    >
      <button
        type="button"
        role="checkbox"
        aria-checked={t.status === 'done'}
        aria-label={t.status === 'done' ? `重开 ${t.title}` : `完成 ${t.title}`}
        disabled={busy}
        onClick={() => onToggle(t.id)}
        className={cn(
          'mt-0.5 flex size-5 shrink-0 items-center justify-center rounded border transition-colors',
          t.status === 'done'
            ? 'border-success bg-success text-white'
            : 'border-input bg-card hover:border-foreground/40',
        )}
      >
        {t.status === 'done' && <Check className="size-3.5" aria-hidden="true" />}
      </button>
      <div className="min-w-0 flex-1">
        <p
          className={cn(
            'text-sm leading-6',
            t.status === 'done' && 'text-muted-foreground line-through',
          )}
        >
          {t.title}
        </p>
        {t.body && (
          <p className="mt-0.5 line-clamp-2 whitespace-pre-wrap text-xs text-muted-foreground">
            {t.body}
          </p>
        )}
        <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
          <span
            className={cn(
              'rounded border px-1.5 py-0.5 text-[10px]',
              PRIO_CLASS[t.priority] ?? 'border-border text-muted-foreground',
            )}
          >
            {PRIO_LABEL[t.priority] ?? t.priority}
          </span>
          {t.project_hint && (
            <span className="rounded bg-info/10 px-1.5 py-0.5 text-[10px] text-info">
              {t.project_hint}
            </span>
          )}
          {t.tags.map((tag) => (
            <span key={tag} className="rounded border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground">
              #{tag}
            </span>
          ))}
          {overdue && (
            <span className="rounded bg-destructive/10 px-1.5 py-0.5 text-[10px] text-destructive">
              已逾期
            </span>
          )}
          {t.due_at && (
            <span className="font-mono text-[10px] text-muted-foreground/70">
              截止 {new Date(t.due_at).toLocaleDateString()}
            </span>
          )}
        </div>
      </div>
      <div className="flex shrink-0 gap-1">
        {t.status !== 'archived' && (
          <Button
            size="sm"
            variant="ghost"
            disabled={busy}
            title="移入归档（从默认视图隐藏，可审计）"
            onClick={() => onArchive(t)}
          >
            归档
          </Button>
        )}
        <Button
          size="sm"
          variant="ghost"
          disabled={busy}
          aria-label={`删除 ${t.title}`}
          onClick={() => onDelete(t)}
        >
          <Trash2 className="size-3.5" aria-hidden="true" />
        </Button>
      </div>
    </Card>
  )
}

/** 待办页（待办形态）：不绑定项目的临时任务/灵感速记——微软 To Do 式清单流。
 *  与工单页（Tickets.tsx）同表不同心智：这里没有 severity、没有状态流转，只有「做完勾掉」。 */
import { useEffect, useMemo, useState } from 'react'
import { appConfirm } from '@/components/confirm'
import { Check, ChevronDown, Trash2 } from 'lucide-react'
import Pager from '@/components/Pager'
import { api, type Todo } from '@/lib/api'
import { Empty, ErrorBox, PageHeader, Spinner } from '@/components/ui-bits'
import { inputCls, selectCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { PRIO_DOT, PRIO_LABEL } from '@/lib/todos-ui'

/** 视图：进行中（默认）/ 已完成 / 已归档——微软 To Do 的清单切换心智 */
type View = 'active' | 'done' | 'archived'

export default function Todos() {
  const [rows, setRows] = useState<Todo[] | null>(null)
  const [view, setView] = useState<View>('active')
  const [priority, setPriority] = useState('')
  const [due, setDue] = useState('') // overdue=已过期 today=今天到期
  const [tag, setTag] = useState('')
  const [q, setQ] = useState('')
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)
  const [doneRows, setDoneRows] = useState<Todo[] | null>(null)
  const [showDone, setShowDone] = useState(false) // 「已完成」折叠组，默认收起（微软 To Do 心智）

  // 翻页（前端切页：单用户数据量小，一次拉全 + slice，支持页码直跳）
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(20)

  // 快速输入条：一行输入回车即建，优先级轻量可调
  const [title, setTitle] = useState('')
  const [quickPriority, setQuickPriority] = useState('normal')

  const query = useMemo(() => {
    // 视图 → 服务端 status 过滤（todo 形态只有 open/done/archived 三态）
    const status = view === 'active' ? 'open' : view
    // limit 拉满（服务端默认 200 会静默截断）——翻页在前端切
    const parts = [`kind=todo`, `status=${status}`, `limit=1000`]
    if (priority) parts.push(`priority=${priority}`)
    if (due) parts.push(`due=${due}`)
    if (tag.trim()) parts.push(`tag=${encodeURIComponent(tag.trim())}`)
    if (q.trim()) parts.push(`q=${encodeURIComponent(q.trim())}`)
    return parts.join('&')
  }, [view, priority, due, tag, q])

  const load = () =>
    api
      .get<Todo[]>(`/todos?${query}`)
      .then((r) => {
        setRows(r)
        setErr('')
      })
      .catch((e) => setErr(e instanceof Error ? e.message : '加载失败'))

  useEffect(() => {
    load()
    setPage(1) // 筛选/视图变化回到第一页
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query])

  // 「已完成」折叠组：active 视图内展开时才拉取（默认收起不白拉）；列表变化后同步刷新
  useEffect(() => {
    if (view !== 'active' || !showDone) return
    api
      .get<Todo[]>('/todos?kind=todo&status=done')
      .then(setDoneRows)
      .catch(() => undefined)
  }, [view, showDone, rows])

  async function quickAdd() {
    if (!title.trim()) return
    setBusy(true)
    try {
      await api.post('/todos', { title: title.trim(), kind: 'todo', priority: quickPriority })
      setTitle('')
      setQuickPriority('normal')
      setErr('')
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '创建失败')
    } finally {
      setBusy(false)
    }
  }

  /** 勾选完成 / 重开——完成后行从「进行中」消失（视图切换），已完成组在 active 视图内可展开看 */
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
    if (
      !(await appConfirm({
        title: `删除待办「${t.title}」？`,
        destructive: true,
        confirmLabel: '删除',
      }))
    )
      return
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

  // 翻页钳制：自动刷新后总数变少时当前页可能越界
  const maxPage = Math.max(1, Math.ceil(rows.length / pageSize))
  const cur = Math.min(page, maxPage)

  const views: { key: View; label: string }[] = [
    { key: 'active', label: '进行中' },
    { key: 'done', label: '已完成' },
    { key: 'archived', label: '已归档' },
  ]

  return (
    <div className="space-y-5">
      <PageHeader title="待办" desc="不绑定项目的临时任务/灵感速记——速记、做完勾掉">
        <input
          className={`${inputCls} w-40`}
          placeholder="搜索待办…"
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
          aria-label="到期筛选"
          value={due}
          onChange={(e) => setDue(e.target.value)}
        >
          <option value="">全部到期</option>
          <option value="overdue">已逾期</option>
          <option value="today">今天到期</option>
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

      {/* 视图切换（微软 To Do 的清单心智：进行中 / 已完成 / 已归档） */}
      <div className="flex gap-1" role="tablist" aria-label="待办视图">
        {views.map((v) => (
          <button
            key={v.key}
            type="button"
            role="tab"
            aria-selected={view === v.key}
            onClick={() => setView(v.key)}
            className={cn(
              'rounded-md px-3 py-1.5 text-sm font-medium transition-colors',
              view === v.key
                ? 'bg-foreground text-background'
                : 'text-muted-foreground hover:bg-muted hover:text-foreground',
            )}
          >
            {v.label}
          </button>
        ))}
      </div>

      {/* 快速输入：一行输入 + 回车即建（归档视图不建新） */}
      {view !== 'archived' && (
        <div className="flex flex-wrap items-center gap-2 rounded-lg border border-border bg-card px-3 py-2.5 shadow-sm transition-colors focus-within:border-foreground/30">
          <input
            className={`${inputCls} min-w-0 flex-1 border-0 bg-transparent px-0 shadow-none focus-visible:ring-0`}
            placeholder="记一条待办…（回车快速创建）"
            aria-label="新建待办"
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
            className={cn(selectCls, 'w-auto')}
            aria-label="优先级"
            value={quickPriority}
            onChange={(e) => setQuickPriority(e.target.value)}
          >
            <option value="normal">普通</option>
            <option value="high">高</option>
            <option value="low">低</option>
          </select>
          <Button size="sm" disabled={busy || !title.trim()} onClick={quickAdd}>
            添加
          </Button>
        </div>
      )}

      {err && <ErrorBox msg={err} />}

      {/* 清单 */}
      {rows.length === 0 ? (
        <Empty
          text={
            view === 'active'
              ? '没有进行中的待办——上方输入框回车记一条，或让 AI 通过 todo_add 帮你记'
              : view === 'done'
                ? '还没有完成的待办'
                : '没有归档的待办'
          }
        />
      ) : (
        <>
          <div className="space-y-1">
            {rows.slice((cur - 1) * pageSize, cur * pageSize).map((t) => (
              <TodoRow
                key={t.id}
                t={t}
                busy={busy}
                onToggle={() => toggleDone(t)}
                onArchive={doArchive}
                onDelete={doDelete}
              />
            ))}
          </div>
          <Pager
            total={rows.length}
            page={cur}
            pageSize={pageSize}
            onPage={setPage}
            onPageSize={(n) => {
              setPageSize(n)
              setPage(1)
            }}
          />
        </>
      )}

      {/* 「已完成」折叠组：仅进行中视图，默认收起（微软 To Do 心智） */}
      {view === 'active' && (
        <DoneGroup
          count={doneRows?.length ?? 0}
          open={showDone}
          rows={doneRows}
          busy={busy}
          onToggleOpen={() => setShowDone((v) => !v)}
          onToggle={toggleDone}
          onArchive={doArchive}
          onDelete={doDelete}
        />
      )}
    </div>
  )
}

/** 翻页条（公共组件 @/components/Pager）。 */

/** 「已完成 N」折叠组（微软 To Do：完成后收进这里，默认收起，展开可看/可重开）。 */
function DoneGroup({
  count,
  open,
  rows,
  busy,
  onToggleOpen,
  onToggle,
  onArchive,
  onDelete,
}: {
  count: number
  open: boolean
  rows: Todo[] | null
  busy: boolean
  onToggleOpen: () => void
  onToggle: (t: Todo) => void
  onArchive: (t: Todo) => void
  onDelete: (t: Todo) => void
}) {
  return (
    <div className="pt-3">
      <button
        type="button"
        aria-expanded={open}
        onClick={onToggleOpen}
        className="flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
      >
        <ChevronDown className={cn('size-4 transition-transform', !open && '-rotate-90')} aria-hidden="true" />
        已完成
        <span className="font-mono text-xs">{count}</span>
      </button>
      {open && (
        <div className="mt-1 space-y-1 border-l border-border pl-3">
          {rows === null ? (
            <Spinner />
          ) : rows.length === 0 ? (
            <p className="px-2 py-1 text-xs text-muted-foreground/70">还没有完成的待办</p>
          ) : (
            rows.map((t) => (
              <TodoRow
                key={t.id}
                t={t}
                busy={busy}
                onToggle={() => onToggle(t)}
                onArchive={onArchive}
                onDelete={onDelete}
              />
            ))
          )}
        </div>
      )}
    </div>
  )
}

/** 单条待办行：微软 To Do 式清单行（无卡片边框，hover 起底色）——勾选完成 + 元信息 + 归档/删除。 */
function TodoRow({
  t,
  busy,
  onToggle,
  onArchive,
  onDelete,
}: {
  t: Todo
  busy: boolean
  onToggle: (id: string) => void
  onArchive: (t: Todo) => void
  onDelete: (t: Todo) => void
}) {
  const overdue = t.due_at && t.status === 'open' && new Date(t.due_at) < new Date()
  const done = t.status === 'done'
  return (
    <div
      className={cn(
        'group flex items-start gap-3 rounded-lg px-3 py-2.5 transition-colors hover:bg-muted/40',
        done && 'opacity-55',
      )}
    >
      {/* 勾选 */}
      <button
        type="button"
        role="checkbox"
        aria-checked={done}
        aria-label={done ? `重开 ${t.title}` : `完成 ${t.title}`}
        disabled={busy}
        onClick={() => onToggle(t.id)}
        className={cn(
          'mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-full border transition-colors',
          done ? 'border-success bg-success text-white' : 'border-input bg-card hover:border-success/60',
        )}
      >
        {done && <Check className="size-3.5" aria-hidden="true" />}
      </button>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          {/* 优先级色点（微软 To Do 的星标位——轻量、不打断清单流） */}
          <span
            aria-label={`优先级 ${PRIO_LABEL[t.priority] ?? t.priority}`}
            className={cn('size-2 shrink-0 rounded-full', PRIO_DOT[t.priority] ?? 'bg-muted-foreground/30')}
          />
          <p className={cn('truncate text-sm leading-6', done && 'text-muted-foreground line-through')}>
            {t.title}
          </p>
        </div>
        {t.body && (
          <p className="mt-0.5 line-clamp-2 pl-4 whitespace-pre-wrap text-xs text-muted-foreground">{t.body}</p>
        )}
        <div className="mt-1 flex flex-wrap items-center gap-1.5 pl-4">
          <span className="font-mono text-[10px] text-muted-foreground/70">EN-{t.short_no}</span>
          {t.project_hint && (
            <span className="rounded bg-info/10 px-1.5 py-0.5 text-[10px] text-info">{t.project_hint}</span>
          )}
          {t.tags.map((tag: string) => (
            <span key={tag} className="rounded border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground">
              #{tag}
            </span>
          ))}
          {overdue && (
            <span className="rounded bg-destructive/10 px-1.5 py-0.5 text-[10px] text-destructive">已逾期</span>
          )}
          {t.due_at && (
            <span className="font-mono text-[10px] text-muted-foreground/70">
              截止 {new Date(t.due_at).toLocaleDateString()}
            </span>
          )}
        </div>
      </div>
      {/* 动作：hover 才显形（清单不被按钮堆打断） */}
      <div className="flex shrink-0 gap-1 opacity-0 transition-opacity group-hover:opacity-100">
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
        <Button size="sm" variant="ghost" disabled={busy} aria-label={`删除 ${t.title}`} onClick={() => onDelete(t)}>
          <Trash2 className="size-3.5" aria-hidden="true" />
        </Button>
      </div>
    </div>
  )
}

/** 待办域（第七域）：不绑定项目的临时任务/灵感速记——速记→做完勾掉。 */
import { useEffect, useMemo, useState } from 'react'
import { appConfirm } from '@/components/confirm'
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
const SEVERITY_CLASS: Record<string, string> = {
  P0: 'border-destructive bg-destructive/10 text-destructive',
  P1: 'border-warning/60 bg-warning/10 text-warning',
  P2: 'border-border text-muted-foreground',
  P3: 'border-border text-muted-foreground/70',
}
/** 工单状态中文标签 */
const TICKET_STATUS_LABEL: Record<string, string> = {
  open: '待确认',
  confirmed: '已确认',
  in_progress: '处理中',
  resolved: '已解决',
  verified: '已验证',
  archived: '已归档',
}
/** 工单状态下一步流转（点按钮推进） */
const TICKET_NEXT: Record<string, { next: string; label: string }> = {
  open: { next: 'confirmed', label: '确认' },
  confirmed: { next: 'in_progress', label: '开始处理' },
  in_progress: { next: 'resolved', label: '标记解决' },
  resolved: { next: 'verified', label: '验证通过' },
}

type StatusFilter = '' | 'open' | 'done' | 'archived' | 'confirmed' | 'in_progress' | 'resolved' | 'verified'

export default function Todos() {
  const [rows, setRows] = useState<Todo[] | null>(null)
  const [status, setStatus] = useState<StatusFilter>('open')
  const [priority, setPriority] = useState('')
  const [kind, setKind] = useState<'' | 'todo' | 'ticket'>('')
  const [tag, setTag] = useState('')
  const [q, setQ] = useState('')
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)

  // 快速输入条
  const [title, setTitle] = useState('')
  const [quickPriority, setQuickPriority] = useState('normal')
  const [quickKind, setQuickKind] = useState<'todo' | 'ticket'>('todo')
  const [quickSeverity, setQuickSeverity] = useState('P2')
  const [quickSymptom, setQuickSymptom] = useState('')
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
        kind: quickKind,
        priority: quickPriority,
        severity: quickKind === 'ticket' ? quickSeverity : undefined,
        symptom: quickKind === 'ticket' ? quickSymptom.trim() || undefined : undefined,
        project_hint: projectHint.trim() || undefined,
      })
      setTitle('')
      setQuickSymptom('')
      setProjectHint('')
      setErr('')
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '创建失败')
    } finally {
      setBusy(false)
    }
  }

  /** 工单状态流转（推进到下一态；resolved 由后端校验 resolution 必填） */
  async function advance(t: Todo) {
    const step = TICKET_NEXT[t.status]
    if (!step) return
    if (step.next === 'resolved') {
      const resolution = window.prompt('解决记录（做了什么/怎么修的）——resolved 状态必填：')
      if (!resolution?.trim()) return
      setBusy(true)
      try {
        await api.put(`/todos/${t.id}`, { status: 'resolved', resolution: resolution.trim() })
        setErr('')
        load()
      } catch (e) {
        setErr(e instanceof Error ? e.message : '操作失败')
      } finally {
        setBusy(false)
      }
      return
    }
    setBusy(true)
    try {
      await api.put(`/todos/${t.id}`, { status: step.next })
      setErr('')
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '操作失败')
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

  const open = rows.filter((t) => t.status === 'open')
  const active = rows.filter((t) => ['confirmed', 'in_progress'].includes(t.status))
  const finished = rows.filter((t) => !['open', 'confirmed', 'in_progress'].includes(t.status))
  const byKind = (list: Todo[]) => (kind ? list.filter((t) => t.kind === kind) : list)

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
          aria-label="状态筛选"
          value={status}
          onChange={(e) => setStatus(e.target.value as StatusFilter)}
        >
          <option value="">全部状态</option>
          <option value="open">open</option>
          <option value="confirmed">已确认（工单）</option>
          <option value="in_progress">处理中（工单）</option>
          <option value="resolved">已解决（工单）</option>
          <option value="verified">已验证（工单）</option>
          <option value="done">已完成</option>
          <option value="archived">已归档</option>
        </select>
        <select
          className={selectCls}
          aria-label="形态筛选"
          value={kind}
          onChange={(e) => setKind(e.target.value as '' | 'todo' | 'ticket')}
        >
          <option value="">全部形态</option>
          <option value="todo">待办</option>
          <option value="ticket">工单</option>
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
          <select
            className={selectCls}
            aria-label="形态"
            value={quickKind}
            onChange={(e) => setQuickKind(e.target.value as 'todo' | 'ticket')}
          >
            <option value="todo">待办</option>
            <option value="ticket">工单</option>
          </select>
          <input
            className={`${inputCls} min-w-0 flex-1`}
            placeholder={quickKind === 'ticket' ? '工单标题…（描述问题）' : '记一条待办…（回车快速创建）'}
            aria-label="标题"
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
            value={quickPriority}
            onChange={(e) => setQuickPriority(e.target.value)}
          >
            <option value="normal">普通</option>
            <option value="high">高</option>
            <option value="low">低</option>
          </select>
          {quickKind === 'ticket' && (
            <select
              className={selectCls}
              aria-label="严重度"
              value={quickSeverity}
              onChange={(e) => setQuickSeverity(e.target.value)}
            >
              <option value="P0">P0 致命</option>
              <option value="P1">P1 严重</option>
              <option value="P2">P2 一般</option>
              <option value="P3">P3 轻微</option>
            </select>
          )}
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
        {quickKind === 'ticket' && (
          <div className="mt-2">
            <input
              className={`${inputCls} w-full`}
              placeholder="症状/现象描述（建议填——描述越清楚工单越有用）"
              aria-label="工单症状"
              value={quickSymptom}
              onChange={(e) => setQuickSymptom(e.target.value)}
            />
          </div>
        )}
      </Card>

      {err && <ErrorBox msg={err} />}

      {/* 待办/工单列表 */}
      {rows.length === 0 ? (
        <Empty text="暂无条目——上方选形态（待办/工单）快速记一条，或让 AI 通过 todo_add 帮你记" />
      ) : (
        <div className="space-y-6">
          {/* 待办进行中（todo） */}
          {byKind(open).filter((t) => t.kind === 'todo').length > 0 && (
            <div className="space-y-2">
              <h3 className="text-sm font-semibold">
                进行中{' '}
                <span className="font-mono text-xs text-muted-foreground">
                  {byKind(open).filter((t) => t.kind === 'todo').length}
                </span>
              </h3>
              <div className="space-y-2">
                {byKind(open)
                  .filter((t) => t.kind === 'todo')
                  .map((t) => (
                    <TodoRow key={t.id} t={t} busy={busy} onToggle={() => toggleDone(t)} onArchive={doArchive} onDelete={doDelete} />
                  ))}
              </div>
            </div>
          )}

          {/* 工单 open（待确认） */}
          {byKind(open).filter((t) => t.kind === 'ticket').length > 0 && (
            <div className="space-y-2">
              <h3 className="text-sm font-semibold">
                工单 · 待确认{' '}
                <span className="font-mono text-xs text-muted-foreground">
                  {byKind(open).filter((t) => t.kind === 'ticket').length}
                </span>
              </h3>
              <div className="space-y-2">
                {byKind(open)
                  .filter((t) => t.kind === 'ticket')
                  .map((t) => (
                    <TicketRow key={t.id} t={t} busy={busy} onAdvance={advance} onArchive={doArchive} onDelete={doDelete} />
                  ))}
              </div>
            </div>
          )}

          {/* 工单 confirmed/in_progress */}
          {byKind(active).length > 0 && (
            <div className="space-y-2">
              <h3 className="text-sm font-semibold">
                工单 · 处理中{' '}
                <span className="font-mono text-xs text-muted-foreground">{byKind(active).length}</span>
              </h3>
              <div className="space-y-2">
                {byKind(active).map((t) => (
                  <TicketRow key={t.id} t={t} busy={busy} onAdvance={advance} onArchive={doArchive} onDelete={doDelete} />
                ))}
              </div>
            </div>
          )}

          {/* 已完成/已解决/已归档 */}
          {byKind(finished).length > 0 && (
            <div className="space-y-2">
              <h3 className="text-sm font-semibold text-muted-foreground">
                已完成 / 已解决 / 归档{' '}
                <span className="font-mono text-xs text-muted-foreground">{byKind(finished).length}</span>
              </h3>
              <div className="space-y-2">
                {byKind(finished).map((t) =>
                  t.kind === 'ticket' ? (
                    <TicketRow key={t.id} t={t} busy={busy} onAdvance={advance} onArchive={doArchive} onDelete={doDelete} />
                  ) : (
                    <TodoRow key={t.id} t={t} busy={busy} onToggle={() => toggleDone(t)} onArchive={doArchive} onDelete={doDelete} />
                  ),
                )}
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
  onToggle: (id: string) => void
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
          {t.tags.map((tag: string) => (
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


/** 单条工单行：severity/状态徽章 + 症状/验收/解决展示 + 状态流转按钮。 */
function TicketRow({
  t,
  busy,
  onAdvance,
  onArchive,
  onDelete,
}: {
  t: Todo
  busy: boolean
  onAdvance: (t: Todo) => void
  onArchive: (t: Todo) => void
  onDelete: (t: Todo) => void
}) {
  const doneish = ['resolved', 'verified', 'archived'].includes(t.status)
  const step = TICKET_NEXT[t.status]
  return (
    <Card
      className={cn(
        'flex items-start gap-3 p-3 transition-colors hover:bg-muted/40',
        doneish && 'opacity-60',
      )}
    >
      <span
        aria-label={`严重度 ${t.severity ?? '未定级'}`}
        className={cn(
          'mt-0.5 flex h-6 shrink-0 items-center rounded border px-1.5 font-mono text-xs',
          SEVERITY_CLASS[t.severity ?? 'P3'],
        )}
      >
        {t.severity ?? 'P?'}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <p className={cn('text-sm font-medium leading-6', doneish && 'text-muted-foreground')}>
            {t.title}
          </p>
          <span className="rounded border border-border px-1.5 py-0.5 text-xs text-muted-foreground">
            {TICKET_STATUS_LABEL[t.status] ?? t.status}
          </span>
        </div>
        {t.symptom && (
          <p className="mt-1 text-xs leading-5 text-muted-foreground">
            <span className="text-foreground/70">症状：</span>
            {t.symptom}
          </p>
        )}
        {t.reproduce && (
          <p className="mt-0.5 text-xs leading-5 text-muted-foreground">
            <span className="text-foreground/70">复现：</span>
            {t.reproduce}
          </p>
        )}
        {t.acceptance && (
          <p className="mt-0.5 text-xs leading-5 text-muted-foreground">
            <span className="text-foreground/70">验收：</span>
            {t.acceptance}
          </p>
        )}
        {t.resolution && (
          <p className="mt-1 rounded bg-success/10 px-2 py-1 text-xs leading-5 text-success">
            <span className="font-medium">解决记录：</span>
            {t.resolution}
          </p>
        )}
        <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <span className={cn('rounded border px-1.5', PRIO_CLASS[t.priority])}>
            {PRIO_LABEL[t.priority]}
          </span>
          {t.due_at && <span>截止 {new Date(t.due_at).toLocaleDateString()}</span>}
          {t.project_hint && <span>· {t.project_hint}</span>}
        </div>
      </div>
      <div className="flex shrink-0 flex-col items-end gap-1.5">
        {step && (
          <Button size="sm" disabled={busy} onClick={() => onAdvance(t)}>
            {step.label}
          </Button>
        )}
        {t.status === 'resolved' && (
          <Button size="sm" variant="outline" disabled={busy} onClick={() => onAdvance(t)}>
            验证通过
          </Button>
        )}
        {!doneish && (
          <button
            type="button"
            className="text-xs text-muted-foreground hover:text-foreground"
            disabled={busy}
            onClick={() => onArchive(t)}
          >
            归档
          </button>
        )}
        <button
          type="button"
          className="text-xs text-destructive/80 hover:text-destructive"
          disabled={busy}
          onClick={() => onDelete(t)}
        >
          删除
        </button>
      </div>
    </Card>
  )
}

/** 工单页（第七域·工单形态）：传统工单——列表 + 详情面板，结构化问题跟踪。
 *  与待办页（Todos.tsx）同表不同心智：severity、状态机流转、症状/复现/验收/解决记录。 */
import { useEffect, useMemo, useState } from 'react'
import { appConfirm } from '@/components/confirm'
import { Trash2, X } from 'lucide-react'
import { api, type Todo } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner } from '@/components/ui-bits'
import { inputCls, selectCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import {
  PRIO_LABEL,
  SEVERITY_CLASS,
  TICKET_DONEISH,
  TICKET_NEXT,
  TICKET_STATUS_CLASS,
  TICKET_STATUS_LABEL,
} from '@/lib/todos-ui'

type StatusFilter = '' | 'open' | 'confirmed' | 'in_progress' | 'resolved' | 'verified' | 'archived'

export default function Tickets() {
  const [rows, setRows] = useState<Todo[] | null>(null)
  const [status, setStatus] = useState<StatusFilter>('')
  const [severity, setSeverity] = useState('')
  const [tag, setTag] = useState('')
  const [q, setQ] = useState('')
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)
  /** 选中的工单（右侧详情面板）；null = 未选中 */
  const [selected, setSelected] = useState<Todo | null>(null)

  const query = useMemo(() => {
    const parts = [`kind=ticket`]
    if (status) parts.push(`status=${status}`)
    if (severity) parts.push(`severity=${severity}`)
    if (tag.trim()) parts.push(`tag=${encodeURIComponent(tag.trim())}`)
    if (q.trim()) parts.push(`q=${encodeURIComponent(q.trim())}`)
    return parts.join('&')
  }, [status, severity, tag, q])

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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query])

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

  /** 编辑工单四件套之一（详情面板内联编辑；PUT 部分更新，其余字段不动） */
  async function saveField(
    t: Todo,
    field: 'symptom' | 'reproduce' | 'acceptance' | 'resolution',
    value: string,
  ) {
    setBusy(true)
    try {
      await api.put(`/todos/${t.id}`, { [field]: value })
      setErr('')
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '保存失败')
      throw e
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
        title: `删除工单「${t.title}」？`,
        destructive: true,
        confirmLabel: '删除',
      }))
    )
      return
    setBusy(true)
    try {
      await api.del(`/todos/${t.id}`)
      setErr('')
      setSelected(null)
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '删除失败')
    } finally {
      setBusy(false)
    }
  }

  if (err && !rows) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  return (
    <div className="space-y-5">
      <PageHeader title="工单" desc="结构化问题跟踪——症状/复现/验收/解决记录，状态机流转">
        <input
          className={`${inputCls} w-40`}
          placeholder="搜索工单…"
          aria-label="搜索工单"
          value={q}
          onChange={(e) => setQ(e.target.value)}
        />
        <select
          className={selectCls}
          aria-label="状态筛选"
          value={status}
          onChange={(e) => setStatus(e.target.value as StatusFilter)}
        >
          <option value="">全部状态</option>
          <option value="open">待确认</option>
          <option value="confirmed">已确认</option>
          <option value="in_progress">处理中</option>
          <option value="resolved">已解决</option>
          <option value="verified">已验证</option>
          <option value="archived">已归档</option>
        </select>
        <select
          className={selectCls}
          aria-label="严重度筛选"
          value={severity}
          onChange={(e) => setSeverity(e.target.value)}
        >
          <option value="">全部严重度</option>
          <option value="P0">P0 致命</option>
          <option value="P1">P1 严重</option>
          <option value="P2">P2 一般</option>
          <option value="P3">P3 轻微</option>
        </select>
        <select
          className={selectCls}
          aria-label="标签筛选"
          value={tag}
          onChange={(e) => setTag(e.target.value)}
        >
          <option value="">全部标签</option>
          <option value="engram">engram</option>
          <option value="待修">待修</option>
          <option value="待优化">待优化</option>
          <option value="待设计">待设计</option>
        </select>
      </PageHeader>

      {err && <ErrorBox msg={err} />}

      {/* 列表 + 详情面板（lg 起双栏；选中行高亮，右侧面板展开详情） */}
      {rows.length === 0 ? (
        <Empty text="暂无工单——让 AI 通过 todo_add（kind=ticket）帮你记，填上症状更好用" />
      ) : (
        <div className={cn('grid grid-cols-1 gap-4', selected && 'lg:grid-cols-[minmax(0,1fr)_minmax(0,26rem)]')}>
          <div className="min-w-0 space-y-1.5">
            {rows.map((t) => (
              <TicketRow
                key={t.id}
                t={t}
                busy={busy}
                active={selected?.id === t.id}
                onSelect={() => setSelected(t)}
              />
            ))}
          </div>
          {selected && (
            <TicketDetail
              t={rows.find((r) => r.id === selected.id) ?? selected}
              busy={busy}
              onClose={() => setSelected(null)}
              onAdvance={advance}
              onArchive={doArchive}
              onDelete={doDelete}
              onEditSave={saveField}
            />
          )}
        </div>
      )}
    </div>
  )
}

/** 列表行：severity 徽章 + 状态徽章 + 标题 + 短号 + 更新时间；点击选中。 */
function TicketRow({
  t,
  busy,
  active,
  onSelect,
}: {
  t: Todo
  busy: boolean
  active: boolean
  onSelect: () => void
}) {
  const doneish = TICKET_DONEISH.includes(t.status)
  return (
    <button
      type="button"
      disabled={busy}
      aria-label={`查看工单 ${t.title}`}
      aria-current={active ? 'true' : undefined}
      onClick={onSelect}
      className={cn(
        'flex w-full items-center gap-3 rounded-lg border px-3 py-2.5 text-left transition-colors',
        active
          ? 'border-foreground/40 bg-muted'
          : 'border-border bg-card hover:border-foreground/25 hover:bg-muted/40',
        doneish && 'opacity-70',
      )}
    >
      <span
        aria-label={`严重度 ${t.severity ?? '未定级'}`}
        className={cn(
          'flex h-6 shrink-0 items-center rounded border px-1.5 font-mono text-xs',
          SEVERITY_CLASS[t.severity ?? 'P3'],
        )}
      >
        {t.severity ?? 'P?'}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <p className={cn('truncate text-sm font-medium', doneish && 'text-muted-foreground')}>{t.title}</p>
          <span
            className={cn('shrink-0 rounded border px-1.5 py-0.5 text-[11px]', TICKET_STATUS_CLASS[t.status])}
          >
            {TICKET_STATUS_LABEL[t.status] ?? t.status}
          </span>
        </div>
        <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1">
          {t.symptom && (
            <p className="min-w-0 max-w-full flex-1 basis-40 truncate text-xs text-muted-foreground">
              {t.symptom}
            </p>
          )}
          {t.tags.map((tag: string) => (
            <span
              key={tag}
              className="rounded border border-border px-1 py-0 text-[10px] text-muted-foreground"
            >
              #{tag}
            </span>
          ))}
        </div>
      </div>
      <div className="shrink-0 text-right font-mono text-[10px] text-muted-foreground/70">
        <p>EN-{t.short_no}</p>
        <p>{new Date(t.updated_at).toLocaleDateString()}</p>
      </div>
    </button>
  )
}

/** 详情面板：symptom/reproduce/acceptance/resolution 完整展示 + **内联编辑** + 状态流转 + 归档/删除。 */
function TicketDetail({
  t,
  busy,
  onClose,
  onAdvance,
  onArchive,
  onDelete,
  onEditSave,
}: {
  t: Todo
  busy: boolean
  onClose: () => void
  onAdvance: (t: Todo) => void
  onArchive: (t: Todo) => void
  onDelete: (t: Todo) => void
  onEditSave: (
    t: Todo,
    field: 'symptom' | 'reproduce' | 'acceptance' | 'resolution',
    value: string,
  ) => Promise<void>
}) {
  const doneish = TICKET_DONEISH.includes(t.status)
  const step = TICKET_NEXT[t.status]
  return (
    <Card className="h-fit min-w-0 p-4 lg:sticky lg:top-0">
      {/* 头：短号 + 关闭 */}
      <div className="flex items-center justify-between gap-2">
        <span className="font-mono text-xs text-muted-foreground">EN-{t.short_no}</span>
        <Button
          size="sm"
          variant="ghost"
          aria-label="关闭详情"
          className="size-7 p-0"
          disabled={busy}
          onClick={onClose}
        >
          <X className="size-4" aria-hidden="true" />
        </Button>
      </div>

      {/* 标题 + 徽章 */}
      <h3 className={cn('mt-1 text-base leading-6 font-semibold break-words', doneish && 'text-muted-foreground')}>
        {t.title}
      </h3>
      <div className="mt-2 flex flex-wrap items-center gap-1.5">
        <span
          aria-label={`严重度 ${t.severity ?? '未定级'}`}
          className={cn(
            'flex h-6 items-center rounded border px-1.5 font-mono text-xs',
            SEVERITY_CLASS[t.severity ?? 'P3'],
          )}
        >
          {t.severity ?? 'P?'}
        </span>
        <span className={cn('rounded border px-1.5 py-0.5 text-xs', TICKET_STATUS_CLASS[t.status])}>
          {TICKET_STATUS_LABEL[t.status] ?? t.status}
        </span>
        <span className={cn('rounded border px-1.5 py-0.5 text-[11px]', 'border-border text-muted-foreground')}>
          {PRIO_LABEL[t.priority] ?? t.priority}
        </span>
        {t.tags.map((tag: string) => (
          <span key={tag} className="rounded border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground">
            #{tag}
          </span>
        ))}
      </div>

      {/* 工单四件套（内联编辑：点「编辑」改文本，保存走 PUT 部分更新，其余字段不动） */}
      <div className="mt-3 space-y-2.5 text-sm">
        <EditableSection label="症状（symptom）" field="symptom" value={t.symptom} busy={busy} t={t} onSave={onEditSave} />
        <EditableSection
          label="复现（reproduce）"
          field="reproduce"
          value={t.reproduce}
          busy={busy}
          t={t}
          onSave={onEditSave}
        />
        <EditableSection
          label="验收（acceptance）"
          field="acceptance"
          value={t.acceptance}
          busy={busy}
          t={t}
          onSave={onEditSave}
        />
        <EditableSection
          label="解决记录（resolution）"
          field="resolution"
          value={t.resolution}
          busy={busy}
          t={t}
          onSave={onEditSave}
          tone="success"
        />
      </div>

      {/* 元信息 */}
      <div className="mt-3 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[10px] text-muted-foreground/70">
        {t.project_hint && <span>项目 {t.project_hint}</span>}
        {t.due_at && <span>截止 {new Date(t.due_at).toLocaleDateString()}</span>}
        <span>更新 {new Date(t.updated_at).toLocaleString()}</span>
        {t.resolved_at && <span>解决 {new Date(t.resolved_at).toLocaleString()}</span>}
      </div>

      {/* 动作：状态流转 + 归档/删除 */}
      <div className="mt-4 flex flex-wrap items-center gap-2 border-t border-border pt-3">
        {step && (
          <Button size="sm" disabled={busy} onClick={() => onAdvance(t)}>
            {step.label}
          </Button>
        )}
        {!doneish && (
          <Button
            size="sm"
            variant="outline"
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
          aria-label={`删除工单 ${t.title}`}
          className="ml-auto text-destructive/80 hover:text-destructive"
          onClick={() => onDelete(t)}
        >
          <Trash2 className="size-3.5" aria-hidden="true" />
        </Button>
      </div>
    </Card>
  )
}

/** 四件套小节（内联编辑）：点「编辑」→ textarea → 保存走 PUT 部分更新；
 *  空字段诚实标「未填」（工单字段空着比藏着有用），resolution 用成功色调。 */
function EditableSection({
  label,
  field,
  value,
  busy,
  t,
  onSave,
  tone,
}: {
  label: string
  field: 'symptom' | 'reproduce' | 'acceptance' | 'resolution'
  value: string
  busy: boolean
  t: Todo
  onSave: (
    t: Todo,
    field: 'symptom' | 'reproduce' | 'acceptance' | 'resolution',
    value: string,
  ) => Promise<void>
  tone?: 'success'
}) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')
  const [err, setErr] = useState('')

  async function save() {
    try {
      await onSave(t, field, draft)
      setEditing(false)
      setErr('')
    } catch {
      // 保存失败：留在编辑态，错误已由父级 ErrorBox 展示
    }
  }

  return (
    <div className={cn(tone === 'success' && 'rounded bg-success/10 px-2.5 py-2')}>
      <div className="flex items-center justify-between gap-2">
        <p
          className={cn(
            'text-[11px] font-medium tracking-wide',
            tone === 'success' ? 'text-success' : 'text-muted-foreground/70',
          )}
        >
          {label}
        </p>
        {!editing && (
          <button
            type="button"
            className="text-[11px] text-muted-foreground hover:text-foreground"
            disabled={busy}
            aria-label={`编辑${label}`}
            onClick={() => {
              setDraft(value)
              setEditing(true)
            }}
          >
            编辑
          </button>
        )}
      </div>
      {editing ? (
        <div className="mt-1">
          <textarea
            className={cn(inputCls, 'min-h-20 w-full text-sm')}
            aria-label={`编辑${label}`}
            value={draft}
            disabled={busy}
            onChange={(e) => setDraft(e.target.value)}
          />
          <div className="mt-1.5 flex gap-1.5">
            <Button size="sm" disabled={busy} onClick={save}>
              保存
            </Button>
            <Button size="sm" variant="ghost" disabled={busy} onClick={() => setEditing(false)}>
              取消
            </Button>
          </div>
        </div>
      ) : value ? (
        <p
          className={cn(
            'mt-0.5 break-words whitespace-pre-wrap leading-5',
            tone === 'success' && 'text-xs text-success',
          )}
        >
          {value}
        </p>
      ) : (
        <p className="mt-0.5 text-xs text-muted-foreground/50">（未填）</p>
      )}
      {err && <p className="mt-1 text-xs text-destructive">{err}</p>}
    </div>
  )
}

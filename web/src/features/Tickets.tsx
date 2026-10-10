/** 工单页（0074 项目绑定制）：独立 tickets 表——列表按项目分组 + 详情面板。
 *  工单必须绑定已有项目（AI 创建时后端强制）；severity、六态状态机、症状/复现/验收/解决记录。 */
import { useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { appConfirm } from '@/components/confirm'
import { FolderOpen, Trash2, X } from 'lucide-react'
import { api } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner } from '@/components/ui-bits'
import { inputCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import WikiMarkdown from '@/components/WikiMarkdown'
import { cn } from '@/lib/utils'
import {
  SEVERITY_CLASS,
  TICKET_DONEISH,
  TICKET_NEXT,
  TICKET_STATUS_CLASS,
  TICKET_STATUS_LABEL,
} from '@/lib/todos-ui'

export interface Ticket {
  id: string
  project_id: string
  title: string
  body: string
  status: string
  severity: string | null
  symptom: string
  reproduce: string
  acceptance: string
  resolution: string
  resolved_at: string | null
  created_at: string
  updated_at: string
  short_no: number
}

interface ProjectBrief {
  id: string
  name: string
}

export default function Tickets() {
  const nav = useNavigate()
  const [rows, setRows] = useState<Ticket[] | null>(null)
  // P019-M3：服务端 total（单页鲉 200，超出时提示未全显示）
  const [total, setTotal] = useState<number | null>(null)
  const [projects, setProjects] = useState<ProjectBrief[]>([])
  const [severity, setSeverity] = useState('')
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)
  /** 选中的工单（右侧详情面板）；null = 未选中 */
  const [selected, setSelected] = useState<Ticket | null>(null)

  const query = useMemo(() => {
    const parts: string[] = []
    if (severity) parts.push(`severity=${severity}`)
    return parts.join('&')
  }, [severity])

  const load = () =>
    Promise.all([
      api.get<{ items: Ticket[]; total: number }>(`/tickets?${query}`),
      api.get<{ topics?: unknown[] } | unknown[]>('/projects').catch(() => null),
    ])
      .then(([r, pj]) => {
        setRows(r.items)
        // P019-M3：记录服务端 total——单页钳 200，超出时给「未全显示」提示（旧实现静默藏）
        setTotal(typeof r.total === 'number' ? r.total : null)
        // 项目名录（分组标题 + 跳转链接显示名）
        const list = Array.isArray(pj) ? pj : ((pj as { projects?: ProjectBrief[] })?.projects ?? [])
        setProjects(
          (list as { id: string; name: string }[]).map((p) => ({ id: p.id, name: p.name })),
        )
        setErr('')
      })
      .catch((e) => setErr(e instanceof Error ? e.message : '加载失败'))

  useEffect(() => {
    load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query])

  const projectName = (id: string) => projects.find((p) => p.id === id)?.name ?? '未知项目'

  // 按项目分组（0074 项目绑定制：每个工单都有归属）——组内 open 优先、其余时间倒序（后端序）
  const byProject = useMemo(() => {
    const m = new Map<string, Ticket[]>()
    for (const t of rows ?? []) {
      const arr = m.get(t.project_id) ?? []
      arr.push(t)
      m.set(t.project_id, arr)
    }
    return [...m.entries()]
  }, [rows])

  /** 工单状态流转（推进到下一态；resolved 由后端校验 resolution 必填） */
  async function advance(t: Ticket) {
    const step = TICKET_NEXT[t.status]
    if (!step) return
    if (step.next === 'resolved') {
      const resolution = window.prompt('解决记录（做了什么/怎么修的）——resolved 状态必填：')
      if (!resolution?.trim()) return
      setBusy(true)
      try {
        await api.put(`/tickets/${t.id}`, { status: 'resolved', resolution: resolution.trim() })
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
      await api.put(`/tickets/${t.id}`, { status: step.next })
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
    t: Ticket,
    field: 'symptom' | 'reproduce' | 'acceptance' | 'resolution',
    value: string,
  ) {
    setBusy(true)
    try {
      await api.put(`/tickets/${t.id}`, { [field]: value })
      setErr('')
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '保存失败')
      throw e
    } finally {
      setBusy(false)
    }
  }

  async function doArchive(t: Ticket) {
    setBusy(true)
    try {
      await api.put(`/tickets/${t.id}`, { status: 'archived' })
      setErr('')
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '归档失败')
    } finally {
      setBusy(false)
    }
  }

  async function doDelete(t: Ticket) {
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
      await api.del(`/tickets/${t.id}`)
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
      <PageHeader title="工单">
        <select
          className={selectClsFor()}
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
      </PageHeader>

      {err && <ErrorBox msg={err} />}

      {/* P019-M3：服务端单页鲉 200 条——超出时提示未全显示（旧实现静默截断） */}
      {total != null && rows.length < total && (
        <p className="text-xs text-warning" role="status">
          当前显示 {rows.length} 条，共 {total} 条——超出单页上限，请用筛选（severity/搜索）缩小范围，或走 AI/导出通道查看全量
        </p>
      )}

      {rows.length === 0 ? (
        <Empty text="暂无工单——让 AI 通过 tickets 域（必须绑定项目）帮你记，填上症状更好用" />
      ) : (
        <div
          className={cn(
            'grid grid-cols-1 gap-4',
            selected
              ? 'lg:grid-cols-[minmax(0,1fr)_minmax(0,26rem)] lg:h-[calc(100dvh-7.5rem)]'
              : 'grid-cols-1',
          )}
        >
          {/* 左：按项目分段的工单列表 */}
          <div
            className={cn(
              'min-w-0 space-y-4',
              selected && 'lg:h-full lg:overflow-y-auto lg:pr-1',
            )}
          >
            {byProject.map(([pid, list]) => {
              const openRows = list.filter((t) => !TICKET_DONEISH.includes(t.status))
              const restRows = list.filter((t) => TICKET_DONEISH.includes(t.status))
              const seg = (l: Ticket[]) =>
                l.length === 0 ? null : (
                  <div className="space-y-1.5">
                    {l.map((t) => (
                      <TicketRow
                        key={t.id}
                        t={t}
                        busy={busy}
                        active={selected?.id === t.id}
                        onSelect={() => setSelected(t)}
                      />
                    ))}
                  </div>
                )
              return (
                <section key={pid} aria-label={`项目 ${projectName(pid)} 的工单`}>
                  <h3 className="mb-1.5 flex items-center gap-2 text-xs font-semibold tracking-wide text-muted-foreground">
                    <FolderOpen className="size-3.5" aria-hidden="true" />
                    <button
                      type="button"
                      className="hover:text-foreground hover:underline"
                      onClick={() => nav(`/projects/${pid}`)}
                      title="打开项目详情"
                    >
                      {projectName(pid)}
                    </button>
                    <span className="font-mono font-normal">{list.length}</span>
                  </h3>
                  {seg(openRows)}
                  {seg(restRows)}
                </section>
              )
            })}
          </div>

          {selected && (
            <div className="min-w-0 lg:self-start lg:max-h-full lg:overflow-y-auto lg:pl-1">
              <TicketDetail
                t={rows.find((r) => r.id === selected.id) ?? selected}
                projectName={projectName(selected.project_id)}
                busy={busy}
                onClose={() => setSelected(null)}
                onAdvance={advance}
                onArchive={doArchive}
                onDelete={doDelete}
                onEditSave={saveField}
                onGoProject={(pid) => nav(`/projects/${pid}`)}
              />
            </div>
          )}
        </div>
      )}
    </div>
  )
}

/** selectCls 的本地包装（lib/ui 的 selectCls 依赖未随文件迁移的上下文时兜底）。 */
function selectClsFor() {
  return 'rounded border border-border bg-transparent px-2 py-1.5 text-sm'
}

/** 列表行：severity 徽章 + 状态徽章 + 标题 + 短号 + 更新时间；点击选中。 */
function TicketRow({
  t,
  busy,
  active,
  onSelect,
}: {
  t: Ticket
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
          <p className={cn('min-w-0 flex-1 truncate text-sm font-medium', doneish && 'text-muted-foreground')}>{t.title}</p>
          <span
            className={cn('shrink-0 rounded border px-1.5 py-0.5 text-[11px]', TICKET_STATUS_CLASS[t.status])}
          >
            {TICKET_STATUS_LABEL[t.status] ?? t.status}
          </span>
        </div>
        {t.symptom && (
          <p className="mt-0.5 truncate text-xs text-muted-foreground">{t.symptom}</p>
        )}
      </div>
      <div className="shrink-0 text-right font-mono text-[10px] text-muted-foreground/70">
        <p>EN-{t.short_no}</p>
        <p>{new Date(t.updated_at).toLocaleDateString()}</p>
      </div>
    </button>
  )
}

/** 详情面板：项目链接 + symptom/reproduce/acceptance/resolution 内联编辑 + 状态流转 + 时间线。 */
function TicketDetail({
  t,
  projectName,
  busy,
  onClose,
  onAdvance,
  onArchive,
  onDelete,
  onEditSave,
  onGoProject,
}: {
  t: Ticket
  projectName: string
  busy: boolean
  onClose: () => void
  onAdvance: (t: Ticket) => void
  onArchive: (t: Ticket) => void
  onDelete: (t: Ticket) => void
  onEditSave: (
    t: Ticket,
    field: 'symptom' | 'reproduce' | 'acceptance' | 'resolution',
    value: string,
  ) => Promise<void>
  onGoProject: (pid: string) => void
}) {
  const doneish = TICKET_DONEISH.includes(t.status)
  const step = TICKET_NEXT[t.status]

  // 活动时间线（ticket_events）：状态流转自动留痕 + 评论
  const [events, setEvents] = useState<TicketEvent[] | null>(null)
  const [cText, setCText] = useState('')
  useEffect(() => {
    setEvents(null)
    api
      .get<{ events: TicketEvent[] }>(`/tickets/${t.id}/events`)
      .then((r) => setEvents(r.events))
      .catch(() => setEvents([]))
  }, [t.id])
  async function postComment() {
    const text = cText.trim()
    if (!text) return
    await api.post(`/tickets/${t.id}/events`, { text })
    setCText('')
    api
      .get<{ events: TicketEvent[] }>(`/tickets/${t.id}/events`)
      .then((r) => setEvents(r.events))
      .catch(() => {})
  }

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
        {/* 项目归属：链接直达项目详情 */}
        <button
          type="button"
          className="flex items-center gap-1 rounded border border-border px-1.5 py-0.5 text-[11px] text-muted-foreground hover:text-foreground"
          onClick={() => onGoProject(t.project_id)}
          title="打开项目详情"
        >
          <FolderOpen className="size-3" aria-hidden="true" />
          {projectName}
        </button>
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

      {/* 活动时间线：event 自动留痕 + comment 讨论 */}
      <div className="mt-4 border-t border-border pt-3">
        <div className="text-xs font-medium text-muted-foreground">活动时间线</div>
        {events && events.length > 0 && (
          <ul className="mt-1.5 space-y-1 text-xs">
            {events.map((e) => (
              <li key={e.id} className="text-muted-foreground">
                {e.kind === 'event'
                  ? `状态 ${String(e.payload.from)} → ${String(e.payload.to)}`
                  : String(e.payload.text ?? '')}
                {' · '}
                {e.actor} · {new Date(e.created_at).toLocaleString()}
              </li>
            ))}
          </ul>
        )}
        {events && events.length === 0 && (
          <p className="mt-1 text-xs text-muted-foreground">暂无动态——状态流转与评论都会留在这里。</p>
        )}
        <div className="mt-2 flex gap-2">
          <input
            className="min-w-0 flex-1 rounded border border-border bg-transparent px-2 py-1 text-xs"
            placeholder="写评论…"
            aria-label="评论输入"
            value={cText}
            onChange={(ev) => setCText(ev.target.value)}
          />
          <Button size="sm" variant="outline" disabled={busy || !cText.trim()} onClick={postComment}>
            评论
          </Button>
        </div>
      </div>
    </Card>
  )
}

interface TicketEvent {
  id: string
  kind: string
  payload: Record<string, unknown>
  actor: string
  created_at: string
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
  t: Ticket
  onSave: (
    t: Ticket,
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
        <div className={cn('mt-0.5 break-words', tone === 'success' && 'text-xs text-success')}>
          {/* 四件套是 AI 常写 markdown 的字段——按全站渲染器出（**、列表、空行段落），
              纯文本 pre-wrap 会裸显 markdown 源码且空行撑出大块空白 */}
          <WikiMarkdown content={value} />
        </div>
      ) : (
        <p className="mt-0.5 text-xs text-muted-foreground/50">（未填）</p>
      )}
      {err && <p className="mt-1 text-xs text-destructive">{err}</p>}
    </div>
  )
}

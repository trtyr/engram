/**
 * 日志页：**系统唯一的时间线**（P010）。
 *
 * 系统里发生的一切都在这条线上：HTTP 请求、错误、以及系统后台的执行过程
 * （入队 → 开始 → 进度 → 终态）。后台执行**不是**另一个概念——它就是
 * 日志里的一类条目（fields.job_id 非空），可展开看过程、失败可重跑。
 *
 * 过滤器：「范围」切全部/后台/系统（后台=带 job_id 的行）；时间窗默认近 7 天；
 * 长内容默认收起，点击展开。
 */
import { useEffect, useState } from 'react'
import { api, type Job, type JobEvent } from '@/lib/api'
import { Card, Empty, PageHeader, Spinner, StatusBadge } from '@/components/ui-bits'
import { fmtTime, inputCls, selectCls, tableCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import Pager from '@/components/Pager'

interface LogRow {
  id: number
  ts: string
  level: string
  target: string
  message: string
  fields: Record<string, unknown>
  request_id: string | null
}

const LEVEL_CLS: Record<string, string> = {
  ERROR: 'text-destructive font-semibold',
  WARN: 'text-amber-600 dark:text-amber-400',
  INFO: '',
  DEBUG: 'text-muted-foreground',
}

/** 时间窗预设（值=天数，0=不限）。 */
const WINDOW_OPTIONS = [
  { value: '1', label: '近 1 天' },
  { value: '7', label: '近 7 天' },
  { value: '30', label: '近 30 天' },
  { value: '0', label: '全部' },
]

function sinceIso(days: string): string | undefined {
  const d = Number(days)
  if (!d) return undefined
  return new Date(Date.now() - d * 86_400_000).toISOString()
}

/** 一条日志是否属于某次后台执行（带 fields.job_id 的行）。 */
function jobIdOf(r: LogRow): string | null {
  const v = r.fields?.job_id
  return typeof v === 'string' && v ? v : null
}

/** 长文本：超阈值默认截断，点击展开/收起。 */
const MSG_MAX = 140
function LongText({ text }: { text: string }) {
  const [open, setOpen] = useState(false)
  if (text.length <= MSG_MAX) return <span>{text}</span>
  return (
    <span>
      <span className={open ? '' : 'break-all'}>{open ? text : `${text.slice(0, MSG_MAX)}…`}</span>
      <button
        type="button"
        className="ml-1 shrink-0 text-xs text-info hover:underline"
        onClick={() => setOpen((v) => !v)}
      >
        {open ? '收起' : `展开（${text.length} 字）`}
      </button>
    </span>
  )
}

export default function Logs() {
  const [rows, setRows] = useState<LogRow[] | null>(null)
  /** 同过滤条件下的真实总数（服务端返回，非「已拉取条数」）。 */
  const [total, setTotal] = useState(0)
  const [level, setLevel] = useState('')
  const [q, setQ] = useState('')
  const [audit, setAudit] = useState(false)
  const [days, setDays] = useState('7')
  /** 范围：all=全部 / job=仅后台 / system=仅系统 */
  const [scope, setScope] = useState<'all' | 'job' | 'system'>(
    () => (new URLSearchParams(window.location.search).get('scope') as 'job') || 'all',
  )
  // T019 域化：memory / wiki / codegraph / system（'' = 全部）
  const [domain, setDomain] = useState(
    () => new URLSearchParams(window.location.search).get('domain') || '',
  )
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(50)
  const [openJob, setOpenJob] = useState<string | null>(null)

  const loadLogs = () => {
    const p = new URLSearchParams({
      limit: String(pageSize),
      offset: String((page - 1) * pageSize),
    })
    if (level) p.set('level', level)
    if (q) p.set('q', q)
    if (audit) p.set('audit', 'true')
    const since = sinceIso(days)
    if (since) p.set('since', since)
    // 范围与任务筛选都交给服务端——总数才会是真的
    if (scope !== 'all') p.set('scope', scope)
    if (domain) p.set('domain', domain)
    if (scope === 'job' && openJob) p.set('job_id', openJob)
    api
      .get<{ logs: LogRow[]; total: number }>(`/logs?${p}`)
      .then((v) => {
        setRows(v.logs)
        setTotal(v.total)
      })
      .catch(() => setRows([]))
  }

  useEffect(() => {
    setRows(null)
    loadLogs()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [level, q, audit, days, scope, domain, openJob, page, pageSize])

  // 10s 自动刷新（保持当前页）
  useEffect(() => {
    const t = setInterval(loadLogs, 10000)
    return () => clearInterval(t)
  })

  const maxPage = Math.max(1, Math.ceil(total / pageSize))
  const cur = Math.min(page, maxPage)
  const winLabel = WINDOW_OPTIONS.find((o) => o.value === days)?.label ?? ''

  return (
    <div className="space-y-6">
      <PageHeader
        title="日志"
        desc="系统里发生的一切：请求、错误、后台执行（info 保留 30 天 / debug 7 天）"
      >
        <div className="flex flex-wrap items-center gap-2">
          <select className={selectCls} value={days} onChange={(e) => setDays(e.target.value)}>
            {WINDOW_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
          <select
            className={selectCls}
            value={scope}
            onChange={(e) => setScope(e.target.value as 'all' | 'job' | 'system')}
          >
            <option value="all">全部</option>
            <option value="job">仅后台</option>
            <option value="system">仅系统</option>
          </select>
          <select className={selectCls} value={domain} onChange={(e) => setDomain(e.target.value)}>
            <option value="">全部域</option>
            <option value="memory">记忆</option>
            <option value="wiki">Wiki</option>
            <option value="codegraph">代码图谱</option>
            <option value="system">系统</option>
          </select>
          <select className={selectCls} value={level} onChange={(e) => setLevel(e.target.value)}>
            <option value="">全部级别</option>
            {['ERROR', 'WARN', 'INFO', 'DEBUG'].map((l) => (
              <option key={l}>{l}</option>
            ))}
          </select>
          <input
            className={inputCls}
            placeholder="搜消息/target…"
            value={q}
            onChange={(e) => setQ(e.target.value)}
          />
          <label className="flex items-center gap-1 text-sm">
            <input type="checkbox" checked={audit} onChange={(e) => setAudit(e.target.checked)} />
            仅审计
          </label>
        </div>
      </PageHeader>
      {rows === null ? (
        <Spinner />
      ) : rows.length === 0 ? (
        <Empty text={`${winLabel}无日志行`} />
      ) : (
        <Card className="overflow-x-auto">
          <table className={tableCls.root}>
            <thead className={tableCls.thead}>
              <tr>
                <th className={tableCls.th}>时间</th>
                <th className={tableCls.th}>级别</th>
                <th className={tableCls.th}>target</th>
                <th className={tableCls.th}>消息</th>
                <th className={tableCls.th}>request_id</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => {
                const jid = jobIdOf(r)
                return (
                  <tr key={r.id} className={tableCls.row}>
                    <td className={`${tableCls.td} text-xs tabular-nums text-muted-foreground`}>
                      {fmtTime(r.ts)}
                    </td>
                    <td className={`${tableCls.td} text-xs font-medium ${LEVEL_CLS[r.level] ?? ''}`}>
                      {r.level}
                    </td>
                    <td className={`${tableCls.td} max-w-40 truncate font-mono text-xs`}>
                      {jid ? (
                        <button
                          type="button"
                          className="rounded bg-muted px-1 py-px text-info hover:underline"
                          title="查看该执行过程"
                          onClick={() => setOpenJob(jid)}
                        >
                          后台
                        </button>
                      ) : (
                        r.target
                      )}
                    </td>
                    <td className={`${tableCls.td} max-w-96`}>
                      <LongText text={r.message} />
                      {Object.keys(r.fields ?? {}).length > 0 && (
                        <details className="mt-0.5">
                          <summary className="cursor-pointer text-xs text-muted-foreground">
                            字段（{Object.keys(r.fields).length}）
                          </summary>
                          <pre className="mt-1 max-h-32 overflow-auto whitespace-pre-wrap rounded-lg bg-muted/50 p-2 text-xs">
                            {JSON.stringify(r.fields, null, 2)}
                          </pre>
                        </details>
                      )}
                    </td>
                    <td className={`${tableCls.td} font-mono text-xs text-muted-foreground`}>
                      {r.request_id ?? ''}
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </Card>
      )}
      {total > 0 && (
        <Pager
          total={total}
          page={cur}
          pageSize={pageSize}
          onPage={setPage}
          onPageSize={(n) => {
            setPageSize(n)
            setPage(1)
          }}
          hint={total > pageSize ? `${winLabel}共 ${total} 条，本页第 ${(cur - 1) * pageSize + 1}–${Math.min(cur * pageSize, total)} 条` : undefined}
        />
      )}
      {openJob && <JobTrajectory jobId={openJob} onClose={() => setOpenJob(null)} />}
    </div>
  )
}

/** 执行过程：该条链在日志中的全部行 + 状态 + 失败可重跑（日志流内展开，非独立区块）。 */
function JobTrajectory({ jobId, onClose }: { jobId: string; onClose: () => void }) {
  const [job, setJob] = useState<Job | null>(null)
  const [events, setEvents] = useState<JobEvent[] | null>(null)

  useEffect(() => {
    api.get<Job>(`/jobs/${jobId}`).then(setJob).catch(() => setJob(null))
    api
      .get<JobEvent[]>(`/jobs/${jobId}/events?limit=200`)
      .then(setEvents)
      .catch(() => setEvents([]))
  }, [jobId])

  return (
    <Card className="p-4">
      <div className="mb-3 flex items-center justify-between">
        <p className="text-sm font-medium">
          执行过程{' '}
          {job && <StatusBadge status={job.status} />}
          <span className="ml-2 font-mono text-xs text-muted-foreground">{jobId}</span>
        </p>
        <div className="flex gap-2">
          {job && (job.status === 'dead' || job.status === 'failed') && (
            <Button
              size="sm"
              variant="outline"
              onClick={async () => {
                await api.post(`/jobs/${jobId}/revive`)
                onClose()
              }}
            >
              重跑
            </Button>
          )}
          <Button size="sm" variant="ghost" onClick={onClose}>
            关闭
          </Button>
        </div>
      </div>
      {events === null ? (
        <Spinner label="过程加载…" />
      ) : events.length === 0 ? (
        <p className="text-sm text-muted-foreground">无过程行</p>
      ) : (
        <div className="max-h-96 space-y-1 overflow-auto">
          {events.map((e) => (
            <div key={e.id} className="border-b border-border/50 py-1.5 text-sm last:border-0">
              <span
                className={`mr-2 text-xs tabular-nums ${e.level === 'error' ? 'text-destructive' : 'text-muted-foreground'}`}
              >
                {fmtTime(e.ts)}
              </span>
              <LongText text={e.message} />
              {e.data != null && (
                <details className="mt-1">
                  <summary className="cursor-pointer text-xs text-muted-foreground">数据</summary>
                  <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded-lg bg-muted/50 p-2 text-xs">
                    {JSON.stringify(e.data, null, 2)}
                  </pre>
                </details>
              )}
            </div>
          ))}
        </div>
      )}
    </Card>
  )
}

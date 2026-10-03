/**
 * 日志域：结构化运行日志 + 后台任务区块（原「任务」页并入，不再单独入口）。
 * 时间窗默认近 7 天；长内容（消息/字段）默认收起，点击展开——避免整页被撑爆。
 */
import { useEffect, useState } from 'react'
import { useLocation } from 'react-router-dom'
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
  // ---- 日志面 ----
  const [rows, setRows] = useState<LogRow[] | null>(null)
  const [level, setLevel] = useState('')
  const [q, setQ] = useState('')
  const [audit, setAudit] = useState(false)
  const [days, setDays] = useState('7')
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(50)

  // ---- 任务面（原 Jobs 页并入）----
  const [jobs, setJobs] = useState<Job[] | null>(null)
  const [jobStatus, setJobStatus] = useState('')
  const [openJob, setOpenJob] = useState<Job | null>(null)

  const loadLogs = () => {
    const p = new URLSearchParams({ limit: '200' })
    if (level) p.set('level', level)
    if (q) p.set('q', q)
    if (audit) p.set('audit', 'true')
    const since = sinceIso(days)
    if (since) p.set('since', since)
    api.get<{ logs: LogRow[] }>(`/logs?${p}`).then((v) => setRows(v.logs)).catch(() => {})
  }

  const loadJobs = () => {
    const p = new URLSearchParams({ limit: '50' })
    if (jobStatus) p.set('status', jobStatus)
    api.get<Job[]>(`/jobs?${p}`).then(setJobs).catch(() => {})
  }

  useEffect(() => {
    setRows(null)
    loadLogs()
    setPage(1)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [level, q, audit, days])

  useEffect(() => {
    setJobs(null)
    loadJobs()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jobStatus])

  // 支持 /logs#jobs 锚点直达（Dashboard 的失败徽章/活动行跳此处）
  const { hash } = useLocation()
  useEffect(() => {
    if (hash === '#jobs') {
      const el = document.getElementById('jobs')
      if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' })
    }
  }, [hash])

  // 10s 自动刷新
  useEffect(() => {
    const t = setInterval(() => {
      loadLogs()
      loadJobs()
    }, 10000)
    return () => clearInterval(t)
  })

  const maxPage = Math.max(1, Math.ceil((rows?.length ?? 0) / pageSize))
  const cur = Math.min(page, maxPage)
  const winLabel = WINDOW_OPTIONS.find((o) => o.value === days)?.label ?? ''

  return (
    <div className="space-y-6">
      {/* ── 运行日志 ── */}
      <PageHeader title="日志" desc="结构化运行日志（info 保留 30 天 / debug 7 天）">
        <div className="flex flex-wrap items-center gap-2">
          <select className={selectCls} value={days} onChange={(e) => setDays(e.target.value)}>
            {WINDOW_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
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
              {rows.slice((cur - 1) * pageSize, cur * pageSize).map((r) => (
                <tr key={r.id} className={tableCls.row}>
                  <td className={`${tableCls.td} text-xs tabular-nums text-muted-foreground`}>
                    {fmtTime(r.ts)}
                  </td>
                  <td className={`${tableCls.td} text-xs font-medium ${LEVEL_CLS[r.level] ?? ''}`}>
                    {r.level}
                  </td>
                  <td className={`${tableCls.td} max-w-40 truncate font-mono text-xs`}>{r.target}</td>
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
              ))}
            </tbody>
          </table>
        </Card>
      )}
      {(rows?.length ?? 0) > 0 && (
        <Pager
          total={rows?.length ?? 0}
          page={cur}
          pageSize={pageSize}
          onPage={setPage}
          onPageSize={(n) => {
            setPageSize(n)
            setPage(1)
          }}
          hint={(rows?.length ?? 0) >= 200 ? '仅载入 200 条（收窄时间窗看更早）' : undefined}
        />
      )}

      {/* ── 后台任务（原「任务」页并入） ── */}
      <div
        id="jobs"
        className="flex flex-wrap items-center justify-between gap-2 border-t border-border pt-5"
      >
        <h2 className="text-sm font-semibold">后台任务</h2>
        <select className={selectCls} value={jobStatus} onChange={(e) => setJobStatus(e.target.value)}>
          <option value="">全部状态</option>
          {['pending', 'running', 'succeeded', 'failed', 'dead'].map((s) => (
            <option key={s}>{s}</option>
          ))}
        </select>
      </div>
      {jobs === null ? (
        <Spinner />
      ) : jobs.length === 0 ? (
        <Empty text="无任务" />
      ) : (
        <Card className="overflow-x-auto">
          <table className={tableCls.root}>
            <thead className={tableCls.thead}>
              <tr>
                <th className={tableCls.th}>类型</th>
                <th className={tableCls.th}>状态</th>
                <th className={tableCls.th}>尝试</th>
                <th className={tableCls.th}>时间</th>
                <th className={tableCls.th}>错误</th>
              </tr>
            </thead>
            <tbody>
              {jobs.map((j) => (
                <tr
                  key={j.id}
                  className={`${tableCls.row} cursor-pointer`}
                  onClick={() => setOpenJob(j)}
                >
                  <td className={`${tableCls.td} font-medium`}>{j.kind}</td>
                  <td className={tableCls.td}>
                    <StatusBadge status={j.status} />
                  </td>
                  <td className={`${tableCls.td} tabular-nums`}>{j.attempts}</td>
                  <td className={`${tableCls.td} text-muted-foreground`}>{fmtTime(j.created_at)}</td>
                  <td className={`${tableCls.td} max-w-64 text-destructive`}>
                    {j.error ? <LongText text={j.error} /> : ''}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}
      {openJob && <EventTimeline job={openJob} onClose={() => setOpenJob(null)} />}
    </div>
  )
}

function EventTimeline({ job, onClose }: { job: Job; onClose: () => void }) {
  const [events, setEvents] = useState<JobEvent[] | null>(null)
  useEffect(() => {
    api
      .get<JobEvent[]>(`/jobs/${job.id}/events?limit=200`)
      .then(setEvents)
      .catch(() => setEvents([]))
  }, [job.id])
  return (
    <Card className="p-4">
      <div className="mb-3 flex items-center justify-between">
        <p className="text-sm font-medium">
          {job.kind} <span className="ml-2 font-mono text-xs text-muted-foreground">{job.id}</span>
        </p>
        <div className="flex gap-2">
          {(job.status === 'dead' || job.status === 'failed') && (
            <Button
              size="sm"
              variant="outline"
              onClick={async () => {
                await api.post(`/jobs/${job.id}/revive`)
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
        <Spinner label="事件加载…" />
      ) : events.length === 0 ? (
        <p className="text-sm text-muted-foreground">无事件</p>
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

/** Jobs 域：任务表 + 事件时间线。 */
import { useEffect, useState } from 'react'
import { api, type Job, type JobEvent } from '@/lib/api'
import { Card, Empty, PageHeader, Spinner, StatusBadge } from '@/components/ui-bits'
import { fmtTime, selectCls, tableCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import Pager from '@/components/Pager'

export default function Jobs() {
  const [rows, setRows] = useState<Job[] | null>(null)
  const [status, setStatus] = useState('')
  const [open, setOpen] = useState<Job | null>(null)
  // 翻页（前端切页：拉满服务端上限 200 条后本地分页，支持页码直跳）
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(50)
  const load = () => {
    // 拉满服务端上限 200 条（5s 自动刷新重拉，不打断翻页视图）
    const p = new URLSearchParams({ limit: '200' })
    if (status) p.set('status', status)
    api.get<Job[]>(`/jobs?${p}`).then(setRows).catch(() => {})
  }
  useEffect(() => {
    setRows(null)
    load()
    setPage(1) // 筛选变化回到第一页
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [status])
  // 5s 自动刷新
  useEffect(() => {
    const t = setInterval(load, 5000)
    return () => clearInterval(t)
  })
  if (!rows) return <Spinner />

  // 翻页钳制：自动刷新后总数变少时当前页可能越界
  const maxPage = Math.max(1, Math.ceil(rows.length / pageSize))
  const cur = Math.min(page, maxPage)

  return (
    <div className="space-y-6">
      <PageHeader title="任务" desc="后台任务队列与事件时间线">
        <select className={selectCls} value={status} onChange={(e) => setStatus(e.target.value)}>
          <option value="">全部状态</option>
          {['pending', 'running', 'succeeded', 'failed', 'dead'].map((s) => (
            <option key={s}>{s}</option>
          ))}
        </select>
      </PageHeader>
      {rows.length === 0 ? (
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
              {rows.slice((cur - 1) * pageSize, cur * pageSize).map((j) => (
                <tr key={j.id} className={`${tableCls.row} cursor-pointer`} onClick={() => setOpen(j)}>
                  <td className={`${tableCls.td} font-medium`}>{j.kind}</td>
                  <td className={tableCls.td}>
                    <StatusBadge status={j.status} />
                  </td>
                  <td className={`${tableCls.td} tabular-nums`}>{j.attempts}</td>
                  <td className={`${tableCls.td} text-muted-foreground`}>{fmtTime(j.created_at)}</td>
                  <td className={`${tableCls.td} max-w-64 truncate text-destructive`}>{j.error ?? ''}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}
      {rows.length > 0 && (
        <Pager
          total={rows.length}
          page={cur}
          pageSize={pageSize}
          onPage={setPage}
          onPageSize={(n) => {
            setPageSize(n)
            setPage(1)
          }}
          hint={rows.length >= 200 ? '仅载入最近 200 条' : undefined}
        />
      )}
      {open && <EventTimeline job={open} onClose={() => setOpen(null)} />}
    </div>
  )
}

function EventTimeline({ job, onClose }: { job: Job; onClose: () => void }) {
  const [events, setEvents] = useState<JobEvent[] | null>(null)
  useEffect(() => {
    api.get<JobEvent[]>(`/jobs/${job.id}/events?limit=200`).then(setEvents).catch(() => setEvents([]))
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
              {e.message}
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

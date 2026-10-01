/** Logs 域（P005-T005）：结构化日志查询——级别/关键词/审计过滤 + 自动刷新。 */
import { useEffect, useState } from 'react'
import { api } from '@/lib/api'
import { Card, Empty, PageHeader, Spinner } from '@/components/ui-bits'
import { fmtTime, inputCls, selectCls, tableCls } from '@/lib/ui'
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

export default function Logs() {
  const [rows, setRows] = useState<LogRow[] | null>(null)
  const [level, setLevel] = useState('')
  const [q, setQ] = useState('')
  const [audit, setAudit] = useState(false)
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(50)

  const load = () => {
    const p = new URLSearchParams({ limit: '200' })
    if (level) p.set('level', level)
    if (q) p.set('q', q)
    if (audit) p.set('audit', 'true')
    api.get<{ logs: LogRow[] }>(`/logs?${p}`).then((v) => setRows(v.logs)).catch(() => {})
  }
  useEffect(() => {
    setRows(null)
    load()
    setPage(1)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [level, q, audit])
  // 10s 自动刷新
  useEffect(() => {
    const t = setInterval(load, 10000)
    return () => clearInterval(t)
  })
  if (!rows) return <Spinner />

  const maxPage = Math.max(1, Math.ceil(rows.length / pageSize))
  const cur = Math.min(page, maxPage)

  return (
    <div className="space-y-6">
      <PageHeader title="日志" desc="结构化运行日志（info 保留 30 天 / debug 7 天）">
        <div className="flex flex-wrap items-center gap-2">
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
      {rows.length === 0 ? (
        <Empty text="无日志行" />
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
                    <span>{r.message}</span>
                    {Object.keys(r.fields ?? {}).length > 0 && (
                      <details className="mt-0.5">
                        <summary className="cursor-pointer text-xs text-muted-foreground">字段</summary>
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
    </div>
  )
}

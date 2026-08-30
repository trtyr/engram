/** Dashboard：统计条 + 近期任务 + LLM 用量。 */
import { useEffect, useState } from 'react'
import { api, type Atom, type Document, type Job, type SearchResponse, type UsageRow, type WikiPage, type Persona } from '@/lib/api'
import {
  Card,
  Empty,
  ErrorBox,
  PageHeader,
  Spinner,
  StatusBadge,
} from '@/components/ui-bits'
import { Button } from '@/components/ui/button'
import { fmtTime, inputCls, tableCls } from '@/lib/ui'

const DOMAIN_LABEL: Record<string, string> = {
  memory: '记忆',
  knowledge: '知识',
  wiki: 'Wiki',
}

/** 跨域统一检索：一次查询融合记忆 / 知识 / Wiki 三域（POST /search）。 */
function GlobalSearch() {
  const [q, setQ] = useState('')
  const [hits, setHits] = useState<SearchResponse | null>(null)
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)

  const run = async () => {
    const query = q.trim()
    if (!query || busy) return
    setBusy(true)
    setErr('')
    try {
      setHits(await api.post<SearchResponse>('/search', { query, limit: 10 }))
    } catch (e) {
      setErr(e instanceof Error ? e.message : '检索失败')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card className="p-4">
      <div className="flex gap-2">
        <input
          className={`${inputCls} flex-1`}
          placeholder="跨域检索：记忆 / 知识 / Wiki 一次搜"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && run()}
        />
        <Button size="sm" onClick={run} disabled={busy}>
          {busy ? '检索中…' : '搜索'}
        </Button>
      </div>
      {err && (
        <div className="mt-3">
          <ErrorBox msg={err} />
        </div>
      )}
      {hits && hits.hits.length === 0 && (
        <div className="mt-3">
          <Empty text="无匹配结果" />
        </div>
      )}
      {hits && hits.hits.length > 0 && (
        <ul className="mt-3 space-y-2">
          {hits.hits.map((h) => {
            const label = DOMAIN_LABEL[h.domain] ?? h.domain
            return (
              <li key={`${h.domain}-${h.id}`} className="rounded-md border border-border p-3 transition-colors hover:border-foreground/25">
                <div className="flex items-center gap-2">
                  <span className="rounded border border-border px-1.5 py-px font-mono text-xs text-muted-foreground">{label}</span>
                  {h.title && <span className="text-sm font-medium">{h.title}</span>}
                  <span className="ml-auto font-mono text-xs text-muted-foreground">{h.score.toFixed(2)}</span>
                </div>
                <p className="mt-1.5 text-sm text-muted-foreground">{h.snippet}</p>
              </li>
            )
          })}
        </ul>
      )}
    </Card>
  )
}

export default function Dashboard() {
  const [err, setErr] = useState('')
  const [atoms, setAtoms] = useState<Atom[] | null>(null)
  const [docs, setDocs] = useState<Document[] | null>(null)
  const [pages, setPages] = useState<WikiPage[] | null>(null)
  const [persona, setPersona] = useState<Persona[] | null>(null)
  const [jobs, setJobs] = useState<Job[] | null>(null)
  const [usage, setUsage] = useState<UsageRow[] | null>(null)

  useEffect(() => {
    api.get<Atom[]>('/memory/atoms?limit=500').then(setAtoms).catch((e) => setErr(String(e.message)))
    api.get<Document[]>('/knowledge/documents?limit=200').then(setDocs).catch(() => {})
    api.get<WikiPage[]>('/wiki/pages?limit=300').then(setPages).catch(() => {})
    api.get<Persona[]>('/memory/persona').then(setPersona).catch(() => {})
    api.get<Job[]>('/jobs?limit=8').then(setJobs).catch(() => {})
    api.get<UsageRow[]>('/llm/usage').then(setUsage).catch(() => {})
  }, [])

  if (err) return <ErrorBox msg={err} />
  if (!atoms || !docs || !pages) return <Spinner />

  const totalTokens = (usage ?? []).reduce((s, u) => s + u.input_tokens + u.output_tokens, 0)
  const stats = [
    { label: '活跃原子 L1', n: atoms.filter((a) => a.status === 'active').length },
    { label: '文档', n: docs.filter((d) => d.status === 'ready').length },
    { label: 'Wiki 页面', n: pages.filter((p) => p.page_type !== 'index' && p.page_type !== 'log').length },
    { label: '画像分面 L3', n: persona?.length ?? 0 },
    { label: 'LLM tokens（30 天）', n: totalTokens },
  ]

  return (
    <div className="space-y-8">
      <PageHeader title="Dashboard" desc="四类长期记忆资产的概览与近期动态" />

      <GlobalSearch />

      <div className="grid grid-cols-2 gap-px overflow-hidden rounded-xl border border-border bg-border/50 sm:grid-cols-3 lg:grid-cols-5">
        {stats.map((s) => (
          <div key={s.label} className="bg-card p-4">
            <p className="font-mono text-2xl font-medium tracking-tight">{s.n.toLocaleString()}</p>
            <p className="mt-1 text-xs text-muted-foreground">{s.label}</p>
          </div>
        ))}
      </div>

      <Card className="overflow-x-auto">
        <div className="border-b border-border px-4 py-3">
          <h2 className="text-sm font-medium">近期任务</h2>
        </div>
        {jobs === null || jobs.length === 0 ? (
          <div className="p-4">
            <Empty text="暂无任务" />
          </div>
        ) : (
          <table className={tableCls.root}>
            <thead className={tableCls.thead}>
              <tr>
                <th className={tableCls.th}>类型</th>
                <th className={tableCls.th}>状态</th>
                <th className={tableCls.th}>时间</th>
                <th className={tableCls.th}>错误</th>
              </tr>
            </thead>
            <tbody>
              {jobs.map((j) => (
                <tr key={j.id} className={tableCls.row}>
                  <td className={tableCls.td}>{j.kind}</td>
                  <td className={tableCls.td}>
                    <StatusBadge status={j.status} />
                  </td>
                  <td className={`${tableCls.td} text-muted-foreground`}>{fmtTime(j.created_at)}</td>
                  <td className={`${tableCls.td} max-w-64 truncate text-destructive`}>{j.error ?? ''}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>

      <Card className="overflow-x-auto">
        <div className="border-b border-border px-4 py-3">
          <h2 className="text-sm font-medium">LLM 用量（近 30 天）</h2>
        </div>
        {(usage ?? []).length === 0 ? (
          <div className="p-4">
            <Empty text="暂无用量" />
          </div>
        ) : (
          <table className={tableCls.root}>
            <thead className={tableCls.thead}>
              <tr>
                <th className={tableCls.th}>时间</th>
                <th className={tableCls.th}>用途</th>
                <th className={tableCls.th}>模型</th>
                <th className={tableCls.th}>输入</th>
                <th className={tableCls.th}>输出</th>
                <th className={tableCls.th}>延迟</th>
              </tr>
            </thead>
            <tbody>
              {(usage ?? []).slice(0, 12).map((u) => (
                <tr key={u.id} className={tableCls.row}>
                  <td className={`${tableCls.td} text-muted-foreground`}>{fmtTime(u.ts)}</td>
                  <td className={tableCls.td}>{u.purpose}</td>
                  <td className={tableCls.td}>{u.model}</td>
                  <td className={`${tableCls.td} tabular-nums`}>{u.input_tokens}</td>
                  <td className={`${tableCls.td} tabular-nums`}>{u.output_tokens}</td>
                  <td className={`${tableCls.td} tabular-nums`}>{u.latency_ms}ms</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
    </div>
  )
}

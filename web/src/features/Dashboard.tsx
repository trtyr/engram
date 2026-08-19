/** Dashboard：统计卡 + 近期任务 + LLM 用量。 */
import { useEffect, useState } from 'react'
import { api, type Atom, type Document, type Job, type UsageRow, type WikiPage, type Persona } from '@/lib/api'
import { Empty, ErrorBox, Spinner, StatusBadge, fmtTime } from '@/components/ui-bits'

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
  const cards = [
    { label: '活跃原子 L1', n: atoms.filter((a) => a.status === 'active').length },
    { label: '文档', n: docs.filter((d) => d.status === 'ready').length },
    { label: 'Wiki 页面', n: pages.filter((p) => p.page_type !== 'index' && p.page_type !== 'log').length },
    { label: '画像分面 L3', n: persona?.length ?? 0 },
    { label: 'LLM tokens（30 天）', n: totalTokens },
  ]

  return (
    <div className="space-y-8">
      <h1 className="text-xl font-semibold">Dashboard</h1>
      <div className="grid grid-cols-2 gap-4 md:grid-cols-5">
        {cards.map((c) => (
          <div key={c.label} className="rounded-lg border p-4">
            <p className="text-2xl font-semibold">{c.n.toLocaleString()}</p>
            <p className="mt-1 text-xs text-muted-foreground">{c.label}</p>
          </div>
        ))}
      </div>

      <section>
        <h2 className="mb-2 font-medium">近期任务</h2>
        {jobs === null || jobs.length === 0 ? (
          <Empty text="暂无任务" />
        ) : (
          <table className="w-full text-sm">
            <tbody>
              {jobs.map((j) => (
                <tr key={j.id} className="border-b">
                  <td className="py-1.5 pr-4">{j.kind}</td>
                  <td className="pr-4">
                    <StatusBadge status={j.status} />
                  </td>
                  <td className="pr-4 text-muted-foreground">{fmtTime(j.created_at)}</td>
                  <td className="max-w-64 truncate text-red-400">{j.error ?? ''}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <section>
        <h2 className="mb-2 font-medium">LLM 用量（近 30 天）</h2>
        {(usage ?? []).length === 0 ? (
          <Empty text="暂无用量" />
        ) : (
          <table className="w-full text-sm">
            <thead className="text-left text-muted-foreground">
              <tr className="border-b">
                <th className="py-1.5 pr-4">时间</th>
                <th className="pr-4">用途</th>
                <th className="pr-4">模型</th>
                <th className="pr-4">输入</th>
                <th className="pr-4">输出</th>
                <th className="pr-4">延迟</th>
              </tr>
            </thead>
            <tbody>
              {(usage ?? []).slice(0, 12).map((u) => (
                <tr key={u.id} className="border-b">
                  <td className="py-1.5 pr-4">{fmtTime(u.ts)}</td>
                  <td className="pr-4">{u.purpose}</td>
                  <td className="pr-4">{u.model}</td>
                  <td className="pr-4">{u.input_tokens}</td>
                  <td className="pr-4">{u.output_tokens}</td>
                  <td className="pr-4">{u.latency_ms}ms</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  )
}

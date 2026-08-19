/** Wiki 域：页面浏览 / Markdown 渲染 / 人工编辑 / 图谱 / Lint / 提案。 */
import { useEffect, useState } from 'react'
import ReactMarkdown from 'react-markdown'
import { api, type GraphDto, type LintReport, type WikiPage } from '@/lib/api'
import { Empty, ErrorBox, Spinner, fmtTime } from '@/components/ui-bits'
import { Button } from '@/components/ui/button'

type Tab = 'pages' | 'graph' | 'lint' | 'proposals'

export default function Wiki() {
  const [tab, setTab] = useState<Tab>('pages')
  return (
    <div className="space-y-6">
      <h1 className="text-xl font-semibold">Wiki</h1>
      <div className="flex gap-2">
        {(['pages', 'graph', 'lint', 'proposals'] as Tab[]).map((t) => (
          <Button key={t} variant={tab === t ? 'default' : 'outline'} size="sm" onClick={() => setTab(t)}>
            {t}
          </Button>
        ))}
      </div>
      {tab === 'pages' && <PagesPane />}
      {tab === 'graph' && <GraphPane />}
      {tab === 'lint' && <LintPane />}
      {tab === 'proposals' && <ProposalsPane />}
    </div>
  )
}

function PagesPane() {
  const [pages, setPages] = useState<WikiPage[] | null>(null)
  const [open, setOpen] = useState<WikiPage | null>(null)
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')
  const [title, setTitle] = useState('')
  const [ingestText, setIngestText] = useState('')
  const [ingestTitle, setIngestTitle] = useState('')
  const [msg, setMsg] = useState('')
  const load = () => api.get<WikiPage[]>('/wiki/pages?limit=100').then(setPages).catch(() => {})
  useEffect(() => {
    load()
  }, [])
  if (!pages) return <Spinner />

  return (
    <div className="grid gap-6 lg:grid-cols-[1fr_2fr]">
      <div className="space-y-2">
        <details className="rounded-lg border p-3">
          <summary className="cursor-pointer text-sm font-medium">新文档 ingest</summary>
          <div className="mt-2 space-y-2">
            <input className="w-full rounded border bg-transparent px-2 py-1 text-sm" placeholder="标题" value={ingestTitle} onChange={(e) => setIngestTitle(e.target.value)} />
            <textarea className="h-24 w-full rounded border bg-transparent px-2 py-1 text-sm" placeholder="源文本" value={ingestText} onChange={(e) => setIngestText(e.target.value)} />
            <Button
              size="sm"
              onClick={async () => {
                const r = await api.post<{ skipped: boolean }>('/wiki/ingest', { title: ingestTitle, text: ingestText })
                setMsg(r.skipped ? '相同内容已摄取（sha 跳过）' : '已入队摄取')
                setIngestText('')
                setIngestTitle('')
                setTimeout(load, 3000)
              }}
            >
              摄取
            </Button>
            {msg && <p className="text-xs text-muted-foreground">{msg}</p>}
          </div>
        </details>
        {pages.map((p) => (
          <div
            key={p.id}
            className={`cursor-pointer rounded-lg border p-3 text-sm hover:bg-accent/30 ${open?.id === p.id ? 'border-accent' : ''}`}
            onClick={async () => {
              setOpen(await api.get<WikiPage>(`/wiki/pages/${encodeURIComponent(p.slug)}`))
              setEditing(false)
            }}
          >
            <div className="flex items-center justify-between">
              <span className="font-medium">{p.title}</span>
              <span className="text-xs text-muted-foreground">
                {p.page_type} v{p.version}
              </span>
            </div>
            {p.origin === 'human' && <span className="text-xs text-orange-400">人工</span>}
          </div>
        ))}
      </div>
      <div>
        {!open ? (
          <Empty text="选择左侧页面" />
        ) : editing ? (
          <div className="space-y-2">
            <input className="w-full rounded-md border bg-transparent px-3 py-2 text-sm" value={title} onChange={(e) => setTitle(e.target.value)} />
            <textarea className="h-96 w-full rounded-md border bg-transparent px-3 py-2 font-mono text-sm" value={draft} onChange={(e) => setDraft(e.target.value)} />
            <div className="flex gap-2">
              <Button
                size="sm"
                onClick={async () => {
                  await api.put(`/wiki/pages/${encodeURIComponent(open.slug)}`, { title, content: draft })
                  setOpen(await api.get<WikiPage>(`/wiki/pages/${encodeURIComponent(open.slug)}`))
                  setEditing(false)
                  load()
                }}
              >
                保存（人工版）
              </Button>
              <Button size="sm" variant="ghost" onClick={() => setEditing(false)}>
                取消
              </Button>
            </div>
          </div>
        ) : (
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <p className="text-xs text-muted-foreground">
                {open.slug} · v{open.version} · {fmtTime(open.updated_at)} · {open.origin}
              </p>
              <Button
                size="sm"
                variant="outline"
                onClick={() => {
                  setDraft(open.content)
                  setTitle(open.title)
                  setEditing(true)
                }}
              >
                编辑
              </Button>
            </div>
            <article className="prose prose-sm prose-invert max-w-none">
              <ReactMarkdown>{open.content}</ReactMarkdown>
            </article>
          </div>
        )}
      </div>
    </div>
  )
}

function GraphPane() {
  const [g, setG] = useState<GraphDto | null>(null)
  useEffect(() => {
    api.get<GraphDto>('/wiki/graph').then(setG).catch(() => {})
  }, [])
  if (!g) return <Spinner />
  if (g.nodes.length === 0) return <Empty text="图谱为空（ingest 后生成）" />
  // 轻量文本渲染：邻接表（sigma.js 在有真实数据量后引入）
  const bySlug = new Map(g.nodes.map((n) => [n.slug, n]))
  const outMap = new Map<string, string[]>()
  for (const e of g.edges) {
    outMap.set(e.from_slug, [...(outMap.get(e.from_slug) ?? []), e.to_slug])
  }
  return (
    <div className="space-y-2">
      {g.nodes
        .filter((n) => n.page_type !== 'index' && n.page_type !== 'log')
        .map((n) => (
          <div key={n.slug} className="rounded-lg border p-3 text-sm">
            <p className="font-medium">
              {n.title} <span className="text-xs text-muted-foreground">({n.page_type})</span>
            </p>
            {(outMap.get(n.slug) ?? []).length > 0 && (
              <p className="mt-1 text-xs text-muted-foreground">
                → {(outMap.get(n.slug) ?? []).map((s) => bySlug.get(s)?.title ?? s).join(' · ')}
              </p>
            )}
          </div>
        ))}
    </div>
  )
}

function LintPane() {
  const [r, setR] = useState<LintReport | null>(null)
  const [err, setErr] = useState('')
  return (
    <div className="space-y-4">
      <Button size="sm" onClick={async () => {
        try {
          setR(await api.post<LintReport>('/wiki/lint'))
        } catch (e) {
          setErr(e instanceof Error ? e.message : 'lint 失败')
        }
      }}>
        运行 Lint
      </Button>
      {err && <ErrorBox msg={err} />}
      {r && (
        <div>
          <p className="mb-2 text-sm text-muted-foreground">检查 {r.checked_pages} 页，{r.issues.length} 个问题</p>
          {r.issues.map((i, idx) => (
            <div key={idx} className="border-b py-2 text-sm">
              <span className="mr-2 rounded bg-yellow-500/15 px-1.5 text-xs text-yellow-400">{i.rule}</span>
              <span className="font-medium">{i.slug}</span>
              <span className="ml-2 text-muted-foreground">{i.detail}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

function ProposalsPane() {
  const [events, setEvents] = useState<{ job_id: string; data: { page_slug: string; proposal_content: string }; ts: string }[] | null>(null)
  const load = async () => {
    const jobs = await api.get<{ id: string }[]>('/jobs?kind=wiki_generate&limit=20')
    const all = []
    for (const j of jobs) {
      try {
        const evs = await api.get<{ message: string; data: unknown; ts: string }[]>(`/jobs/${j.id}/events`)
        for (const ev of evs) {
          if (ev.message.includes('提案')) {
            all.push({ job_id: j.id, data: ev.data as { page_slug: string; proposal_content: string }, ts: ev.ts })
          }
        }
      } catch { /* skip */ }
    }
    setEvents(all)
  }
  useEffect(() => {
    load()
  }, [])
  if (!events) return <Spinner />
  if (events.length === 0) return <Empty text="无待审提案" />
  return (
    <div className="space-y-3">
      {events.map((e, i) => (
        <div key={i} className="rounded-lg border p-4">
          <p className="text-sm font-medium">
            {e.data.page_slug} <span className="ml-2 text-xs text-muted-foreground">{fmtTime(e.ts)}</span>
          </p>
          <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap rounded bg-muted/50 p-2 text-xs">{e.data.proposal_content}</pre>
          <Button
            size="sm"
            className="mt-2"
            onClick={async () => {
              const page = await api.get<WikiPage>(`/wiki/pages/${encodeURIComponent(e.data.page_slug)}`)
              await api.post('/wiki/proposals/apply', {
                slug: e.data.page_slug,
                title: page.title,
                content: e.data.proposal_content,
              })
              load()
            }}
          >
            合入
          </Button>
        </div>
      ))}
    </div>
  )
}

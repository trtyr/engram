/** Wiki 域：页面浏览 / Markdown 渲染 / 人工编辑 / 图谱 / Lint / 提案。 */
import { useEffect, useState } from 'react'
import WikiGraph from '@/components/WikiGraph'
import InsightsPanel from '@/components/InsightsPanel'
import ReviewQueue from '@/components/ReviewQueue'
import WikiMarkdown from '@/components/WikiMarkdown'
import { useSearchParams } from 'react-router-dom'
import { api, type GraphDto, type LintReport, type WikiPage } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner, Tabs } from '@/components/ui-bits'
import { fmtTime, inputCls, tableCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'

type Tab = 'pages' | 'graph' | 'insights' | 'lint' | 'proposals' | 'sources'

const TABS: { value: Tab; label: string }[] = [
  { value: 'pages', label: '页面' },
  { value: 'graph', label: '图谱' },
  { value: 'insights', label: '洞察' },
  { value: 'lint', label: 'Lint' },
  { value: 'proposals', label: '提案' },
  { value: 'sources', label: '原料' },
]

export default function Wiki() {
  const [tab, setTab] = useState<Tab>('pages')
  return (
    <div className="space-y-6">
      <PageHeader title="Wiki" desc="LLM 增量维护的互链知识库" />
      <Tabs items={TABS} value={tab} onChange={setTab} />
      {tab === 'pages' && <PagesPane />}
      {tab === 'graph' && <GraphPane />}
      {tab === 'insights' && <GraphWithInsights />}
      {tab === 'lint' && <LintPane />}
      {tab === 'proposals' && <ReviewAndProposals />}
      {tab === 'sources' && <SourcesPane />}
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
  const [params] = useSearchParams()
  const load = () => api.get<WikiPage[]>('/wiki/pages?limit=100').then(setPages).catch(() => {})
  useEffect(() => {
    load()
  }, [])
  // ?page= 深链（wikilink 跳转）
  useEffect(() => {
    const slug = params.get('page')
    if (slug) {
      api.get<WikiPage>(`/wiki/pages/${encodeURIComponent(slug)}`).then(setOpen).catch(() => {})
    }
  }, [params])
  if (!pages) return <Spinner />

  return (
    <div className="grid gap-6 lg:grid-cols-[1fr_2fr]">
      <div className="space-y-2">
        <Card className="p-3">
          <details>
            <summary className="cursor-pointer text-sm font-medium">新文档 ingest</summary>
            <div className="mt-3 space-y-2">
              <input
                className={`${inputCls} w-full`}
                placeholder="标题"
                value={ingestTitle}
                onChange={(e) => setIngestTitle(e.target.value)}
              />
              <textarea
                className={`${inputCls} h-24 w-full`}
                placeholder="源文本"
                value={ingestText}
                onChange={(e) => setIngestText(e.target.value)}
              />
              <Button
                size="sm"
                onClick={async () => {
                  const r = await api.post<{ skipped: boolean }>('/wiki/ingest', {
                    title: ingestTitle,
                    text: ingestText,
                  })
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
        </Card>
        {pages.map((p) => (
          <Card
            key={p.id}
            className={`cursor-pointer p-3 transition-colors ${open?.id === p.id ? 'border-brand/50 bg-brand/5' : 'hover:bg-muted/30'}`}
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
            {p.origin === 'human' && <span className="mt-1 inline-block text-xs text-orange-400">人工</span>}
          </Card>
        ))}
      </div>
      <div>
        {!open ? (
          <Empty text="选择左侧页面" />
        ) : editing ? (
          <Card className="space-y-3 p-4">
            <input
              className={`${inputCls} w-full`}
              value={title}
              onChange={(e) => setTitle(e.target.value)}
            />
            <textarea
              className={`${inputCls} h-96 w-full font-mono`}
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
            />
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
          </Card>
        ) : (
          <Card className="space-y-3 p-4">
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
            <WikiMarkdown content={open.content} onNavigateSlug={() => {}} />
          </Card>
        )}
      </div>
    </div>
  )
}

function GraphPane({ highlightSlugs }: { highlightSlugs?: string[] }) {
  const [g, setG] = useState<GraphDto | null>(null)
  useEffect(() => {
    api.get<GraphDto>('/wiki/graph').then(setG).catch(() => {})
  }, [])
  if (!g) return <Spinner />
  return <WikiGraph graph={g} highlightSlugs={highlightSlugs} />
}

/** 图谱 + 洞察面板联动（点击洞察卡高亮图谱节点） */
function GraphWithInsights() {
  const [g, setG] = useState<GraphDto | null>(null)
  const [highlight, setHighlight] = useState<string[] | null>(null)
  useEffect(() => {
    api.get<GraphDto>('/wiki/graph').then(setG).catch(() => {})
  }, [])
  if (!g) return <Spinner />
  return (
    <div className="grid gap-4 lg:grid-cols-[3fr_2fr]">
      <div>
        <h2 className="mb-2 text-sm font-medium">图谱（点击洞察卡联动高亮）</h2>
        <WikiGraph graph={g} highlightSlugs={highlight ?? undefined} />
      </div>
      <div>
        <h2 className="mb-2 text-sm font-medium">洞察</h2>
        <InsightsPanel onHighlight={setHighlight} />
      </div>
    </div>
  )
}

/** Review 队列 + 人工页提案合流 */
function ReviewAndProposals() {
  return (
    <div className="space-y-6">
      <section>
        <h2 className="mb-2 text-sm font-medium">人审队列</h2>
        <ReviewQueue />
      </section>
      <section>
        <h2 className="mb-2 text-sm font-medium">人工页更新提案</h2>
        <ProposalsPane />
      </section>
    </div>
  )
}

/** sources 管理（级联删除） */
function SourcesPane() {
  const [rows, setRows] = useState<{ id: string; title: string | null; status: string }[] | null>(null)
  const [confirming, setConfirming] = useState<string | null>(null)
  const [report, setReport] = useState<{ deleted_pages: string[]; updated_shared: string[]; cleaned_links: number } | null>(null)
  const load = () => api.get<{ id: string; title: string | null; status: string }[]>('/wiki/sources').then(setRows).catch(() => {})
  useEffect(() => {
    load()
  }, [])
  if (!rows) return <Spinner />
  return (
    <div className="space-y-3" data-testid="sources-pane">
      {rows.length === 0 ? (
        <Empty text="暂无原料" />
      ) : (
        <Card className="overflow-hidden">
          <table className={tableCls.root}>
            <thead className={tableCls.thead}>
              <tr>
                <th className={tableCls.th}>标题</th>
                <th className={tableCls.th}>状态</th>
                <th className={tableCls.th} />
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.id} className={tableCls.row}>
                  <td className={`${tableCls.td} font-medium`}>{r.title ?? '(未命名)'}</td>
                  <td className={tableCls.td}>{r.status}</td>
                  <td className={`${tableCls.td} text-right`}>
                    {confirming === r.id ? (
                      <span className="inline-flex gap-1.5">
                        <Button
                          size="sm"
                          variant="destructive"
                          data-testid={`confirm-delete-${r.id}`}
                          onClick={async () => {
                            const rep = await api.del<typeof report>(`/wiki/sources/${r.id}`)
                            setReport(rep)
                            setConfirming(null)
                            load()
                          }}
                        >
                          确认级联删除
                        </Button>
                        <Button size="sm" variant="ghost" onClick={() => setConfirming(null)}>
                          取消
                        </Button>
                      </span>
                    ) : (
                      <Button size="sm" variant="outline" onClick={() => setConfirming(r.id)}>
                        删除
                      </Button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}
      {report && (
        <Card className="p-3 text-xs" data-testid="cascade-report">
          <p className="mb-1 font-medium">级联删除报告：</p>
          <p>整页删除：{report.deleted_pages.length}（{report.deleted_pages.join(', ')}）</p>
          <p>共享页摘源：{report.updated_shared.length}（{report.updated_shared.join(', ')}）</p>
          <p>清理死链：{report.cleaned_links} 条</p>
        </Card>
      )}
    </div>
  )
}

function LintPane() {
  const [r, setR] = useState<LintReport | null>(null)
  const [err, setErr] = useState('')
  return (
    <div className="space-y-4">
      <Button
        size="sm"
        onClick={async () => {
          try {
            setR(await api.post<LintReport>('/wiki/lint'))
          } catch (e) {
            setErr(e instanceof Error ? e.message : 'lint 失败')
          }
        }}
      >
        运行 Lint
      </Button>
      {err && <ErrorBox msg={err} />}
      {r && (
        <div>
          <p className="mb-2 text-sm text-muted-foreground">
            检查 {r.checked_pages} 页，{r.issues.length} 个问题
          </p>
          {r.issues.map((i, idx) => (
            <div key={idx} className="border-b border-border/50 py-2 text-sm last:border-0">
              <span className="mr-2 rounded bg-yellow-500/15 px-1.5 py-0.5 text-xs text-yellow-400">
                {i.rule}
              </span>
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
      } catch {
        /* skip */
      }
    }
    setEvents(all)
  }
  useEffect(() => {
    // oxlint-disable-next-line react/set-state-in-effect -- load 异步拉取，setEvents 在 await 之后，非同步级联（误报）
    load()
  }, [])
  if (!events) return <Spinner />
  if (events.length === 0) return <Empty text="无待审提案" />
  return (
    <div className="space-y-3">
      {events.map((e, i) => (
        <Card key={i} className="p-4">
          <p className="text-sm font-medium">
            {e.data.page_slug} <span className="ml-2 text-xs font-normal text-muted-foreground">{fmtTime(e.ts)}</span>
          </p>
          <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap rounded-lg bg-muted/50 p-3 text-xs">
            {e.data.proposal_content}
          </pre>
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
        </Card>
      ))}
    </div>
  )
}

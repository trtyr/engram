/** Wiki 域：Obsidian 式浏览 —— 目录树（folder 层级）+ Markdown 阅读 + 图谱独立视图。
 *  文档 = 收件箱入口；洞察/Lint/提案/原料/目标 = 运维二级入口。 */
import { useEffect, useMemo, useState } from 'react'
import { ChevronRight, FileText, Folder, FolderOpen } from 'lucide-react'
import WikiGraph from '@/components/WikiGraph'
import InsightsPanel from '@/components/InsightsPanel'
import ReviewQueue from '@/components/ReviewQueue'
import WikiMarkdown from '@/components/WikiMarkdown'
import { DocumentsPane } from './Knowledge'
import { useSearchParams } from 'react-router-dom'
import { api, type GraphDto, type LintReport, type Purpose, type WikiPage } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner, Tabs } from '@/components/ui-bits'
import { fmtTime, inputCls, tableCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

type View = 'tree' | 'graph'
type Panel = 'none' | 'inbox' | 'ops'

export default function Wiki() {
  const [view, setView] = useState<View>('tree')
  const [panel, setPanel] = useState<Panel>('none')
  return (
    <div className="space-y-4">
      <PageHeader title="Wiki" desc="AI 织入的互链知识库——目录树浏览页面，图谱看关系">
        {panel !== 'none' ? (
          <Button size="sm" variant="outline" onClick={() => setPanel('none')}>
            ← 返回 Wiki
          </Button>
        ) : (
          <>
            <Tabs
              items={[
                { value: 'tree', label: '目录' },
                { value: 'graph', label: '图谱' },
              ]}
              value={view}
              onChange={setView}
            />
            <Button size="sm" variant="outline" onClick={() => setPanel('inbox')}>
              收件箱
            </Button>
            <Button size="sm" variant="outline" onClick={() => setPanel('ops')}>
              运维
            </Button>
          </>
        )}
      </PageHeader>
      {panel === 'inbox' && <InboxPane />}
      {panel === 'ops' && <OpsPanel />}
      {panel === 'none' && view === 'tree' && <TreeReader />}
      {panel === 'none' && view === 'graph' && <GraphPane />}
    </div>
  )
}

/** 文档收件箱：上传 / URL / 列表 / 阅读，织入后页面进目录树。 */
function InboxPane() {
  return (
    <div className="space-y-3">
      <p className="text-sm text-muted-foreground">
        上传或粘贴文档 → 自动解析、分块、织入 Wiki 页面树（原料可检索原文，页面由 LLM 增量维护）。
      </p>
      <DocumentsPane />
    </div>
  )
}

// ---------- 目录树 + 阅读 ----------

interface FolderNode {
  name: string
  folders: FolderNode[]
  pages: WikiPage[]
}

/** 按 folder（/ 分隔多级）把页面聚成嵌套树；folder='' 的页面落在根。 */
function buildFolders(pages: WikiPage[]): FolderNode {
  const root: FolderNode = { name: '', folders: [], pages: [] }
  const ensure = (node: FolderNode, parts: string[]): FolderNode => {
    let cur = node
    for (const part of parts) {
      let next = cur.folders.find((f) => f.name === part)
      if (!next) {
        next = { name: part, folders: [], pages: [] }
        cur.folders.push(next)
      }
      cur = next
    }
    return cur
  }
  const sort = (n: FolderNode) => {
    n.folders.sort((a, b) => a.name.localeCompare(b.name, 'zh'))
    n.pages.sort((a, b) => a.title.localeCompare(b.title, 'zh'))
    n.folders.forEach(sort)
  }
  for (const p of pages) {
    const parts = (p.folder || '')
      .split('/')
      .map((s) => s.trim())
      .filter(Boolean)
    if (parts.length === 0) root.pages.push(p)
    else ensure(root, parts).pages.push(p)
  }
  sort(root)
  return root
}

function countPages(n: FolderNode): number {
  return n.pages.length + n.folders.reduce((acc, f) => acc + countPages(f), 0)
}

function FolderTree({
  node,
  path,
  openId,
  onSelect,
  collapsed,
  onToggleFolder,
}: {
  node: FolderNode
  path: string
  openId: string | null
  onSelect: (slug: string) => void
  collapsed: Set<string>
  onToggleFolder: (p: string) => void
}) {
  return (
    <div>
      {node.folders.map((f) => {
        const fp = path ? `${path}/${f.name}` : f.name
        const isCollapsed = collapsed.has(fp)
        return (
          <div key={fp}>
            <button
              type="button"
              onClick={() => onToggleFolder(fp)}
              className="flex w-full items-center gap-1.5 rounded px-2 py-1 text-sm text-muted-foreground hover:bg-muted hover:text-foreground"
            >
              <ChevronRight className={cn('size-3.5 shrink-0 transition-transform', !isCollapsed && 'rotate-90')} />
              {isCollapsed ? <Folder className="size-3.5 shrink-0" /> : <FolderOpen className="size-3.5 shrink-0" />}
              <span className="truncate">{f.name}</span>
              <span aria-hidden="true" className="ml-auto font-mono text-xs tabular-nums text-muted-foreground/60">{countPages(f)}</span>
            </button>
            {!isCollapsed && (
              <div className="ml-3 border-l border-border/60 pl-2">
                <FolderTree
                  node={f}
                  path={fp}
                  openId={openId}
                  onSelect={onSelect}
                  collapsed={collapsed}
                  onToggleFolder={onToggleFolder}
                />
              </div>
            )}
          </div>
        )
      })}
      {node.pages.map((p) => (
        <button
          key={p.id}
          type="button"
          onClick={() => onSelect(p.slug)}
          className={cn(
            'flex w-full items-center gap-1.5 rounded px-2 py-1 text-left text-sm',
            openId === p.id ? 'bg-foreground text-background' : 'text-foreground hover:bg-muted',
          )}
        >
          <FileText className="size-3.5 shrink-0" />
          <span className="truncate">{p.title}</span>
          {p.origin === 'human' && <span aria-hidden="true" className="ml-auto text-[10px] text-warning">人</span>}
        </button>
      ))}
    </div>
  )
}

function PageReader({ page, onSaved }: { page: WikiPage | null; onSaved: () => void }) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')
  const [title, setTitle] = useState('')
  const [folder, setFolder] = useState('')
  if (!page) return <Empty text="选择左侧页面" />
  if (!editing) {
    return (
      <Card className="flex min-h-0 flex-1 flex-col">
        <div className="flex items-start justify-between gap-3 border-b border-border px-4 py-2.5">
          <div className="min-w-0">
            <h2 className="text-base font-semibold">{page.title}</h2>
            <p className="mt-0.5 text-xs text-muted-foreground">
              {page.slug} · {page.page_type} · v{page.version} · {fmtTime(page.updated_at)}
              {page.folder && ` · ${page.folder}`}
            </p>
          </div>
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              setDraft(page.content)
              setTitle(page.title)
              setFolder(page.folder)
              setEditing(true)
            }}
          >
            编辑
          </Button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto p-4">
          <WikiMarkdown content={page.content} onNavigateSlug={() => {}} />
        </div>
      </Card>
    )
  }
  return (
    <Card className="space-y-3 p-4">
      <input
        className={`${inputCls} w-full`}
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        aria-label="页面标题"
      />
      <input
        className={`${inputCls} w-full`}
        value={folder}
        onChange={(e) => setFolder(e.target.value)}
        placeholder="文件夹（如 技术/Rust，留空=根目录）"
        aria-label="文件夹"
      />
      <textarea
        className={`${inputCls} h-96 w-full font-mono`}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        aria-label="页面内容"
      />
      <div className="flex gap-2">
        <Button
          size="sm"
          onClick={async () => {
            await api.put(`/wiki/pages/${encodeURIComponent(page.slug)}`, {
              title,
              content: draft,
              folder: folder.trim() || undefined,
            })
            setEditing(false)
            onSaved()
          }}
        >
          保存（人工版）
        </Button>
        <Button size="sm" variant="ghost" onClick={() => setEditing(false)}>
          取消
        </Button>
      </div>
    </Card>
  )
}

function TreeReader() {
  const [pages, setPages] = useState<WikiPage[] | null>(null)
  const [open, setOpen] = useState<WikiPage | null>(null)
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
  const [params] = useSearchParams()
  const load = () =>
    api
      .get<WikiPage[]>('/wiki/pages?limit=300')
      .then(setPages)
      .catch(() => {})
  useEffect(() => {
    load()
  }, [])
  // ?page= 深链（wikilink 跳转）
  useEffect(() => {
    const slug = params.get('page')
    if (slug) {
      api
        .get<WikiPage>(`/wiki/pages/${encodeURIComponent(slug)}`)
        .then(setOpen)
        .catch(() => {})
    }
  }, [params])
  const tree = useMemo(() => (pages ? buildFolders(pages) : null), [pages])
  const onToggleFolder = (p: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev)
      if (next.has(p)) next.delete(p)
      else next.add(p)
      return next
    })
  }
  const onSelect = async (slug: string) => {
    const page = await api.get<WikiPage>(`/wiki/pages/${encodeURIComponent(slug)}`)
    setOpen(page)
  }
  if (!pages || !tree) return <Spinner />
  return (
    <div className="grid gap-4 lg:grid-cols-[280px_1fr]">
      <Card className="p-2 lg:h-[calc(100vh-15rem)] lg:overflow-y-auto">
        {pages.length === 0 ? (
          <Empty text="还没有页面——去收件箱上传文档，织入后这里会长出目录树" />
        ) : (
          <FolderTree
            node={tree}
            path=""
            openId={open?.id ?? null}
            onSelect={onSelect}
            collapsed={collapsed}
            onToggleFolder={onToggleFolder}
          />
        )}
      </Card>
      <div className="flex min-h-0 lg:h-[calc(100vh-15rem)]">
        <PageReader key={open?.id} page={open} onSaved={load} />
      </div>
    </div>
  )
}

// ---------- 图谱独立视图 ----------

function GraphPane({ highlightSlugs }: { highlightSlugs?: string[] }) {
  const [g, setG] = useState<GraphDto | null>(null)
  useEffect(() => {
    api
      .get<GraphDto>('/wiki/graph')
      .then(setG)
      .catch(() => {})
  }, [])
  if (!g) return <Spinner />
  return <WikiGraph graph={g} highlightSlugs={highlightSlugs} />
}

// ---------- 运维二级入口 ----------

type OpsSection = 'insights' | 'lint' | 'proposals' | 'sources' | 'purpose'
const OPS_SECTIONS: { value: OpsSection; label: string }[] = [
  { value: 'insights', label: '洞察' },
  { value: 'lint', label: 'Lint' },
  { value: 'proposals', label: '提案' },
  { value: 'sources', label: '原料' },
  { value: 'purpose', label: '目标' },
]

function OpsPanel() {
  const [section, setSection] = useState<OpsSection>('insights')
  return (
    <div className="space-y-4">
      <Tabs items={OPS_SECTIONS} value={section} onChange={setSection} />
      {section === 'insights' && <InsightsPanel onHighlight={() => {}} />}
      {section === 'lint' && <LintPane />}
      {section === 'proposals' && <ReviewAndProposals />}
      {section === 'sources' && <SourcesPane />}
      {section === 'purpose' && <PurposePane />}
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
  const load = () =>
    api
      .get<{ id: string; title: string | null; status: string }[]>('/wiki/sources')
      .then(setRows)
      .catch(() => {})
  useEffect(() => {
    load()
  }, [])
  if (!rows) return <Spinner />
  return (
    <div className="space-y-3" data-testid="sources-pane">
      {rows.length === 0 ? (
        <Empty text="暂无原料" />
      ) : (
        <Card className="overflow-x-auto">
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
          <p>
            整页删除：{report.deleted_pages.length}（{report.deleted_pages.join(', ')}）
          </p>
          <p>
            共享页摘源：{report.updated_shared.length}（{report.updated_shared.join(', ')}）
          </p>
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
              <span className="mr-2 rounded bg-warning/15 px-1.5 py-0.5 text-xs text-warning">{i.rule}</span>
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
  const [events, setEvents] = useState<
    { job_id: string; data: { page_slug: string; proposal_content: string }; ts: string }[] | null
  >(null)
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

/** Wiki 目标（purpose）：goals / key_questions / scope 三栏，每行一条。 */
function PurposePane() {
  const [p, setP] = useState<Purpose | null>(null)
  const [goals, setGoals] = useState('')
  const [questions, setQuestions] = useState('')
  const [scope, setScope] = useState('')
  const [msg, setMsg] = useState('')
  useEffect(() => {
    api
      .get<Purpose>('/wiki/purpose')
      .then((p) => {
        setP(p)
        setGoals(p.goals.join('\n'))
        setQuestions(p.key_questions.join('\n'))
        setScope(p.scope.join('\n'))
      })
      .catch(() => {})
  }, [])
  if (!p) return <Spinner />
  const split = (s: string) =>
    s
      .split('\n')
      .map((x) => x.trim())
      .filter(Boolean)
  return (
    <Card className="space-y-4 p-4">
      <div>
        <label className="mb-1.5 block text-sm font-medium">目标（为什么建这个知识库）</label>
        <textarea className={`${inputCls} h-24 w-full`} value={goals} onChange={(e) => setGoals(e.target.value)} />
      </div>
      <div>
        <label className="mb-1.5 block text-sm font-medium">关键问题（应能回答什么）</label>
        <textarea className={`${inputCls} h-24 w-full`} value={questions} onChange={(e) => setQuestions(e.target.value)} />
      </div>
      <div>
        <label className="mb-1.5 block text-sm font-medium">范围边界</label>
        <textarea className={`${inputCls} h-24 w-full`} value={scope} onChange={(e) => setScope(e.target.value)} />
      </div>
      <div className="flex items-center gap-2">
        <Button
          size="sm"
          onClick={async () => {
            try {
              await api.put('/wiki/purpose', {
                goals: split(goals),
                key_questions: split(questions),
                scope: split(scope),
              })
              setMsg('已保存')
            } catch (ex) {
              setMsg(ex instanceof Error ? ex.message : '保存失败')
            }
          }}
        >
          保存
        </Button>
        {msg && <p className="text-xs text-muted-foreground">{msg}</p>}
      </div>
    </Card>
  )
}

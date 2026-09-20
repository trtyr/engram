/** Wiki 域：Obsidian 式浏览 —— 目录树（folder 层级）+ Markdown 阅读 + 图谱独立视图。
 *  文档 = 收件箱入口；洞察/Lint/提案/原料/目标 = 运维二级入口。
 *  单库终局（2026-09-20）：库选择/建库/删库 UI 已移除，lib 固定 main，/wiki/* 请求仍带 ?lib=；
 *  POST /wiki/search 例外走 body.library（当前前端无该调用点）。
 *  2026-09-03 审计 28 项全修：布局骨架 / 状态提升与 URL / 视觉层次 / 排版 / 可访问性 / 健壮性。 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { CSSProperties, MouseEvent as ReactMouseEvent } from 'react'
import { ChevronRight, FileText, Folder, FolderOpen, Inbox } from 'lucide-react'
import WikiGraph from '@/components/WikiGraph'
import InsightsPanel from '@/components/InsightsPanel'
import ReviewQueue from '@/components/ReviewQueue'
import WikiMarkdown from '@/components/WikiMarkdown'
import { DocumentsPane } from './DocumentsPane'
import { useSearchParams } from 'react-router-dom'
import { api, type GraphDto, type LintReport, type Purpose, type WikiPage } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner, Tabs } from '@/components/ui-bits'
import { fmtTime, inputCls, relTime, selectCls, tableCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

type View = 'tree' | 'graph'
type Panel = 'none' | 'inbox' | 'ops'
const TREE_W_MIN = 220
const TREE_W_MAX = 480

/** /wiki/* 请求拼接当前库 query（?lib=）；已有 query 用 & 合并。 */
const withLib = (path: string, lib: string) => `${path}${path.includes('?') ? '&' : '?'}lib=${encodeURIComponent(lib)}`

/** localStorage 守卫读写——Node 26 实验性 localStorage / 隐私模式下静默降级。 */
function lsGet<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key)
    return raw ? (JSON.parse(raw) as T) : fallback
  } catch {
    return fallback
  }
}
function lsSet(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value))
  } catch {
    /* 忽略 */
  }
}

export default function Wiki() {
  const [view, setView] = useState<View>('tree')
  const [panel, setPanel] = useState<Panel>('none')
  const [params, setParams] = useSearchParams()
  // —— 单库终局（2026-09-20）：库选择 UI 已移除，lib 固定 main（API ?lib= 参数保留兼容）——
  const [lib] = useState('main')
  // —— 状态提升（审计 #4）：切图谱 / 收件箱 / 运维再回来，选中与折叠不丢 ——
  const [pages, setPages] = useState<WikiPage[] | null>(null)
  const [loadErr, setLoadErr] = useState('')
  const [open, setOpen] = useState<WikiPage | null>(null)
  const [opening, setOpening] = useState(false)
  const [openErr, setOpenErr] = useState('')
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set(lsGet<string[]>('engram-wiki-collapsed', [])))
  const [treeW, setTreeW] = useState<number>(() => {
    const v = lsGet<number>('engram-wiki-split', 0)
    return v >= TREE_W_MIN && v <= TREE_W_MAX ? v : 280
  })
  // onSelect 已拉取的页面，深链 effect 跳过重复请求
  const requestedRef = useRef<string | null>(null)

  const load = useCallback(() => {
    return api
      .get<WikiPage[]>(withLib('/wiki/pages', lib))
      .then((r) => {
        setPages(r)
        setLoadErr('')
      })
      .catch((e: unknown) => setLoadErr(e instanceof Error ? e.message : '目录树加载失败'))
  }, [lib])
  useEffect(() => {
    void load()
  }, [load])

  // ?page= 深链（wikilink / 分享 / 刷新）：打开页面 + 自动展开所在 folder（#21）
  useEffect(() => {
    const slug = params.get('page')
    if (!slug) return
    if (requestedRef.current === slug) {
      requestedRef.current = null
      return
    }
    // oxlint-disable-next-line react/set-state-in-effect -- 置 loading 先于异步拉取，非同步级联（同 ProposalsPane 先例）
    setOpening(true)
    let stale = false
    api
      .get<WikiPage>(withLib(`/wiki/pages/${encodeURIComponent(slug)}`, lib))
      .then((p) => {
        if (stale) return
        setOpen(p)
        setOpenErr('')
        const parts = (p.folder || '')
          .split('/')
          .map((s) => s.trim())
          .filter(Boolean)
        if (parts.length) {
          setCollapsed((prev) => {
            const next = new Set(prev)
            let path = ''
            for (const part of parts) {
              path = path ? `${path}/${part}` : part
              next.delete(path)
            }
            lsSet('engram-wiki-collapsed', [...next])
            return next
          })
        }
      })
      .catch(() => setOpenErr(`页面「${slug}」打开失败：可能不存在或服务异常`))
      .finally(() => {
        if (!stale) setOpening(false)
      })
    return () => {
      stale = true
    }
  }, [params, lib])

  const onSelect = async (slug: string) => {
    // 存量 [[lib/slug]] 跨库引用容错：单库终局后统一按 main 打开尾段 slug
    if (slug.includes('/')) {
      const targetSlug = slug.split('/').pop() ?? slug
      setOpen(null)
      requestedRef.current = targetSlug
      setOpening(true)
      setOpenErr('')
      try {
        const page = await api.get<WikiPage>(
          withLib(`/wiki/pages/${encodeURIComponent(targetSlug)}`, lib),
        )
        setOpen(page)
        const next = new URLSearchParams(params)
        next.set('lib', lib)
        next.set('page', targetSlug)
        setParams(next)
      } catch {
        setOpenErr(`跨库页面「${slug}」打开失败：目标库或页面可能不存在`)
      } finally {
        setOpening(false)
      }
      return
    }
    setOpening(true)
    setOpenErr('')
    requestedRef.current = slug
    try {
      const page = await api.get<WikiPage>(withLib(`/wiki/pages/${encodeURIComponent(slug)}`, lib))
      setOpen(page)
      const next = new URLSearchParams(params)
      next.set('page', slug)
      setParams(next)
    } catch {
      setOpenErr(`页面「${slug}」打开失败：可能不存在或服务异常`)
      void load() // 页面可能已删除：刷新目录树
    } finally {
      setOpening(false)
    }
  }

  const toggleFolder = (p: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev)
      if (next.has(p)) next.delete(p)
      else next.add(p)
      lsSet('engram-wiki-collapsed', [...next])
      return next
    })
  }

  // 树 / 阅读分割线拖拽（#6，唯一新增交互）
  const startDrag = (e: ReactMouseEvent) => {
    e.preventDefault()
    const startX = e.clientX
    const startW = treeW
    const onMove = (ev: MouseEvent) => setTreeW(Math.min(TREE_W_MAX, Math.max(TREE_W_MIN, startW + ev.clientX - startX)))
    const onUp = () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      document.body.style.cursor = ''
      setTreeW((w) => {
        lsSet('engram-wiki-split', w)
        return w
      })
    }
    document.body.style.cursor = 'col-resize'
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }

  // 高度实算（#2）：工作区只在树/图视图锁定视口高度（3rem = main 上下 py-6），收件箱/运维自然流不受限
  const bounded = panel === 'none'
  return (
    <div className={cn('flex flex-col gap-4 lg:gap-5', bounded && 'lg:h-[calc(100vh-3rem)]')}>
      <div className="shrink-0">
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
      </div>
      {panel === 'inbox' && <InboxPane libSlug={lib} />}
      {panel === 'ops' && <OpsPanel lib={lib} />}
      {panel === 'none' && view === 'tree' && (
        <div className="flex min-h-0 flex-1 flex-col gap-4 lg:flex-row">
          <div
            className="w-full min-h-0 lg:w-[var(--tree-w)] lg:shrink-0"
            style={{ '--tree-w': `${treeW}px` } as CSSProperties}
          >
            <TreePane
              pages={pages}
              loadErr={loadErr}
              onRetry={load}
              openId={open?.id ?? null}
              collapsed={collapsed}
              onSelect={onSelect}
              onToggleFolder={toggleFolder}
            />
          </div>
          <div
            role="separator"
            aria-orientation="vertical"
            onMouseDown={startDrag}
            title="拖拽调整目录树宽度"
            className="hidden w-1 shrink-0 cursor-col-resize self-stretch rounded bg-border transition-colors hover:bg-foreground/40 lg:block"
          />
          <div className="flex min-h-[45vh] flex-1 flex-col lg:min-h-0">
            <PageReader
              page={open}
              opening={opening}
              openErr={openErr}
              hasPages={!!pages && pages.length > 0}
              libSlug={lib}
              onOpenInbox={() => setPanel('inbox')}
              onSaved={load}
              onNavigateSlug={onSelect}
            />
          </div>
        </div>
      )}
      {panel === 'none' && view === 'graph' && <GraphPane libSlug={lib} />}
    </div>
  )
}

/** 文档收件箱：上传 / URL / 列表 / 阅读，织入后页面进目录树。key=libSlug 切库时重挂载刷新。 */
function InboxPane({ libSlug }: { libSlug: string }) {
  return (
    <div className="space-y-3">
      <h2 className="text-sm font-semibold">收件箱</h2>
      <p className="text-sm text-muted-foreground">
        上传或粘贴文档 → 自动解析、分块、织入 Wiki 页面树（原料可检索原文，页面由 LLM 增量维护）。
      </p>
      <DocumentsPane key={libSlug} libSlug={libSlug} />
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

/** 左栏：目录树面板（层次底色 #17 / 错误态 #24 / 截断诚实提示 #25 / 独立滚动 #3）。 */
function TreePane({
  pages,
  loadErr,
  onRetry,
  openId,
  collapsed,
  onSelect,
  onToggleFolder,
}: {
  pages: WikiPage[] | null
  loadErr: string
  onRetry: () => void
  openId: string | null
  collapsed: Set<string>
  onSelect: (slug: string) => void
  onToggleFolder: (p: string) => void
}) {
  const tree = useMemo(() => (pages ? buildFolders(pages) : null), [pages])
  return (
    <Card className="flex max-h-[45vh] flex-col overflow-hidden bg-muted/30 lg:h-full lg:max-h-none">
      {loadErr ? (
        <div className="space-y-2 p-3">
          <ErrorBox msg={loadErr} />
          <Button size="sm" variant="outline" onClick={onRetry}>
            重试
          </Button>
        </div>
      ) : !pages || !tree ? (
        <Spinner />
      ) : pages.length === 0 ? (
        <div className="p-3">
          <Empty text="还没有页面——去收件箱上传文档，织入后这里会长出目录树" />
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto p-2 [scrollbar-gutter:stable]">
          <nav aria-label="Wiki 目录树">
            {/* WAI-ARIA tree：容器 tree / 行 treeitem+aria-level / 子层级 group */}
            <div role="tree" aria-label="Wiki 页面目录">
              <FolderTree
                node={tree}
                path=""
                depth={0}
                openId={openId}
                onSelect={onSelect}
                collapsed={collapsed}
                onToggleFolder={onToggleFolder}
              />
            </div>
          </nav>
        </div>
      )}
    </Card>
  )
}

function FolderTree({
  node,
  path,
  depth,
  openId,
  onSelect,
  collapsed,
  onToggleFolder,
}: {
  node: FolderNode
  path: string
  depth: number
  openId: string | null
  onSelect: (slug: string) => void
  collapsed: Set<string>
  onToggleFolder: (p: string) => void
}) {
  const level = depth + 1
  return (
    <div>
      {node.folders.map((f) => {
        const fp = path ? `${path}/${f.name}` : f.name
        const isCollapsed = collapsed.has(fp)
        return (
          <div key={fp}>
            <button
              type="button"
              role="treeitem"
              aria-level={level}
              onClick={() => onToggleFolder(fp)}
              aria-expanded={!isCollapsed}
              className="flex w-full items-center gap-1.5 rounded px-2 py-1 text-sm text-muted-foreground hover:bg-muted hover:text-foreground"
            >
              <ChevronRight className={cn('size-3.5 shrink-0 transition-transform', !isCollapsed && 'rotate-90')} />
              {isCollapsed ? <Folder className="size-3.5 shrink-0" /> : <FolderOpen className="size-3.5 shrink-0" />}
              <span className="truncate" title={f.name}>
                {f.name}
              </span>
              <span aria-hidden="true" className="ml-auto font-mono text-xs tabular-nums text-muted-foreground/80">
                {countPages(f)}
              </span>
            </button>
            {!isCollapsed && (
              <div role="group" className="ml-2 border-l border-border/60 pl-2">
                <FolderTree
                  node={f}
                  path={fp}
                  depth={level}
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
          role="treeitem"
          aria-level={level}
          onClick={() => onSelect(p.slug)}
          aria-current={openId === p.id || undefined}
          className={cn(
            'flex w-full items-center gap-1.5 rounded px-2 py-1 text-left text-sm',
            openId === p.id ? 'bg-foreground text-background' : 'text-foreground hover:bg-muted',
          )}
        >
          <FileText className="size-3.5 shrink-0" />
          <span className="truncate" title={p.title}>
            {p.title}
          </span>
          {p.origin === 'human' && (
            <span
              aria-hidden="true"
              title="人工维护"
              className="ml-auto shrink-0 font-mono text-xs text-muted-foreground/80"
            >
              人
            </span>
          )}
        </button>
      ))}
    </div>
  )
}

function PageReader({
  page,
  opening,
  openErr,
  hasPages,
  libSlug,
  onOpenInbox,
  onSaved,
  onNavigateSlug,
}: {
  page: WikiPage | null
  opening: boolean
  openErr: string
  hasPages: boolean
  libSlug: string
  onOpenInbox: () => void
  onSaved: () => void
  onNavigateSlug: (slug: string) => void
}) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')
  const [title, setTitle] = useState('')
  const [folder, setFolder] = useState('')
  const [saving, setSaving] = useState(false)
  const [saveErr, setSaveErr] = useState('')
  if (opening && !page) {
    return (
      <Card className="flex min-h-0 flex-1 items-center justify-center">
        <Spinner label="正在打开页面…" />
      </Card>
    )
  }
  // 空态撑满 + 引导（#1 / #18）
  if (!page) {
    return (
      <Card className="flex min-h-0 flex-1 flex-col">
        <div
          data-testid="wiki-empty-guide"
          className="m-3 flex min-h-0 flex-1 flex-col items-center justify-center gap-3 rounded-lg border border-dashed border-border px-6 py-14 text-center"
        >
          <Inbox className="size-6 text-muted-foreground/40" />
          {openErr ? (
            <p className="text-sm text-destructive">{openErr}</p>
          ) : (
            <p className="text-sm text-muted-foreground">从左侧目录树选一页开始阅读；正文里的 wikilink 可直接跳转</p>
          )}
          {hasPages && <p className="text-xs text-muted-foreground/80">织入的页面按文件夹层级自动归档</p>}
          <Button size="sm" variant="outline" onClick={onOpenInbox}>
            上传第一份文档
          </Button>
        </div>
      </Card>
    )
  }
  if (!editing) {
    return (
      <Card className="flex min-h-0 flex-1 flex-col">
        <div className="flex items-start justify-between gap-3 border-b border-border px-4 py-2.5 md:px-6">
          <div className="min-w-0">
            <h2 className="text-base font-semibold">{page.title}</h2>
            <p className="mt-0.5 text-xs text-muted-foreground">
              {page.slug} · {page.page_type} · v{page.version} ·{' '}
              <span title={fmtTime(page.updated_at)}>{relTime(page.updated_at)}</span>
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
              setSaveErr('')
              setEditing(true)
            }}
          >
            编辑
          </Button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto p-4 [scrollbar-gutter:stable] md:p-6">
          <div className="mx-auto w-full max-w-4xl">
            <WikiMarkdown content={page.content} onNavigateSlug={onNavigateSlug} />
          </div>
        </div>
      </Card>
    )
  }
  return (
    <Card className="flex min-h-0 flex-1 flex-col p-4 md:p-6">
      <div className="flex shrink-0 items-center justify-between gap-3">
        <h2 className="text-sm font-semibold">编辑页面（保存为人工版，蒸馏不覆盖）</h2>
        <Button size="sm" variant="ghost" onClick={() => setEditing(false)}>
          取消
        </Button>
      </div>
      <div className="mt-3 flex min-h-0 flex-1 flex-col gap-3">
        <input
          className={cn(inputCls, 'w-full')}
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          aria-label="页面标题"
        />
        <input
          className={cn(inputCls, 'w-full')}
          value={folder}
          onChange={(e) => setFolder(e.target.value)}
          placeholder="文件夹（如 技术/Rust，留空=根目录）"
          aria-label="文件夹"
        />
        <textarea
          className={cn(inputCls, 'min-h-48 w-full flex-1 leading-relaxed')}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          aria-label="页面内容"
        />
        <div className="flex shrink-0 items-center gap-2">
          <Button
            size="sm"
            disabled={saving}
            onClick={async () => {
              setSaving(true)
              setSaveErr('')
              try {
                await api.put(withLib(`/wiki/pages/${encodeURIComponent(page.slug)}`, libSlug), {
                  title,
                  content: draft,
                  folder: folder.trim() || undefined,
                })
                setEditing(false)
                onSaved()
              } catch (e) {
                setSaveErr(e instanceof Error ? e.message : '保存失败')
              } finally {
                setSaving(false)
              }
            }}
          >
            {saving ? '保存中…' : '保存（人工版）'}
          </Button>
          {saveErr && <span className="text-xs text-destructive">{saveErr}</span>}
        </div>
      </div>
    </Card>
  )
}

// ---------- 图谱独立视图 ----------

function GraphPane({ libSlug }: { libSlug: string }) {
  const [g, setG] = useState<GraphDto | null>(null)
  const [err, setErr] = useState('')
  // 规模化 task-5：子图过滤——社区（Louvain 全图编号）+ 页型。全量 communities
  // 缓存下来做下拉选项（子图响应只含选中社区，选项不能跟着丢）。
  const [allComms, setAllComms] = useState<{ id: number; size: number }[] | null>(null)
  const [community, setCommunity] = useState<number | null>(null)
  const [pageType, setPageType] = useState<string>('')
  const load = useCallback(() => {
    const qs = new URLSearchParams({ lib: libSlug })
    if (community !== null) qs.set('community', String(community))
    if (pageType) qs.set('page_type', pageType)
    return api
      .get<GraphDto>(`/wiki/graph?${qs.toString()}`)
      .then((r) => {
        setG(r)
        if (community === null && !pageType) setAllComms((r.communities ?? []).map((c) => ({ id: c.id, size: c.size })))
      })
      .catch((e: unknown) => setErr(e instanceof Error ? e.message : '图谱加载失败'))
  }, [libSlug, community, pageType])
  useEffect(() => {
    void load()
  }, [load])
  if (err) return <ErrorBox msg={err} />
  if (!g) return <Spinner />
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <select
          aria-label="按社区过滤"
          className={selectCls}
          value={community ?? ''}
          onChange={(e) => setCommunity(e.target.value === '' ? null : Number(e.target.value))}
        >
          <option value="">全部社区</option>
          {(allComms ?? []).map((c) => (
            <option key={c.id} value={c.id}>
              社区 {c.id}（{c.size} 页）
            </option>
          ))}
        </select>
        <select
          aria-label="按页型过滤"
          className={selectCls}
          value={pageType}
          onChange={(e) => setPageType(e.target.value)}
        >
          <option value="">全部页型</option>
          {['entity', 'concept', 'source', 'synthesis', 'comparison', 'analysis'].map((t) => (
            <option key={t} value={t}>
              {t}
            </option>
          ))}
        </select>
        <span className="text-muted-foreground">
          {g.nodes.length} 节点 / {g.edges.length} 边
        </span>
      </div>
      <WikiGraph graph={g} />
    </div>
  )
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

function OpsPanel({ lib }: { lib: string }) {
  const [section, setSection] = useState<OpsSection>('insights')
  return (
    <div className="space-y-4">
      <h2 className="text-sm font-semibold">运维</h2>
      <Tabs items={OPS_SECTIONS} value={section} onChange={setSection} />
      {section === 'insights' && <InsightsPanel onHighlight={() => {}} libSlug={lib} />}
      {section === 'lint' && <LintPane libSlug={lib} />}
      {section === 'proposals' && <ReviewAndProposals libSlug={lib} />}
      {section === 'sources' && <SourcesPane libSlug={lib} />}
      {section === 'purpose' && <PurposePane libSlug={lib} />}
    </div>
  )
}

/** Review 队列 + 人工页提案合流 */
function ReviewAndProposals({ libSlug }: { libSlug: string }) {
  return (
    <div className="space-y-6">
      <section>
        <h2 className="mb-2 text-sm font-medium">人审队列</h2>
        <ReviewQueue libSlug={libSlug} />
      </section>
      <section>
        <h2 className="mb-2 text-sm font-medium">人工页更新提案</h2>
        <ProposalsPane libSlug={libSlug} />
      </section>
    </div>
  )
}

/** sources 管理（级联删除） */
function SourcesPane({ libSlug }: { libSlug: string }) {
  const [rows, setRows] = useState<{ id: string; title: string | null; status: string }[] | null>(null)
  const [confirming, setConfirming] = useState<string | null>(null)
  const [report, setReport] = useState<{ deleted_pages: string[]; updated_shared: string[]; cleaned_links: number } | null>(null)
  const load = useCallback(
    () =>
      api
        .get<{ id: string; title: string | null; status: string }[]>(withLib('/wiki/sources', libSlug))
        .then(setRows)
        .catch(() => {}),
    [libSlug],
  )
  useEffect(() => {
    load()
  }, [load])
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
                            const rep = await api.del<typeof report>(withLib(`/wiki/sources/${r.id}`, libSlug))
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

function LintPane({ libSlug }: { libSlug: string }) {
  const [r, setR] = useState<LintReport | null>(null)
  const [err, setErr] = useState('')
  return (
    <div className="space-y-4">
      <Button
        size="sm"
        onClick={async () => {
          try {
            setR(await api.post<LintReport>(withLib('/wiki/lint', libSlug)))
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

function ProposalsPane({ libSlug }: { libSlug: string }) {
  const [events, setEvents] = useState<
    { job_id: string; data: { page_slug: string; proposal_content: string }; ts: string }[] | null
  >(null)
  const load = useCallback(async () => {
    // 服务端聚合（GET /wiki/proposals 一条 SQL 每job取最新提案）——替代 jobs + 逐 job events 的 N+1
    const rows = await api.get<
      { job_id: string; data: unknown; ts: string; message: string }[]
    >(withLib('/wiki/proposals', libSlug))
    const all = rows
      .filter((ev) => ev.message.includes('提案'))
      .map((ev) => ({
        job_id: ev.job_id,
        data: ev.data as { page_slug: string; proposal_content: string },
        ts: ev.ts,
      }))
    setEvents(all)
  }, [libSlug])
  useEffect(() => {
    // oxlint-disable-next-line react/set-state-in-effect -- load 异步拉取，setEvents 在 await 之后，非同步级联（误报）
    void load()
  }, [load])
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
              const page = await api.get<WikiPage>(withLib(`/wiki/pages/${encodeURIComponent(e.data.page_slug)}`, libSlug))
              await api.post(withLib('/wiki/proposals/apply', libSlug), {
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
function PurposePane({ libSlug }: { libSlug: string }) {
  const [p, setP] = useState<Purpose | null>(null)
  const [goals, setGoals] = useState('')
  const [questions, setQuestions] = useState('')
  const [scope, setScope] = useState('')
  const [msg, setMsg] = useState('')
  useEffect(() => {
    api
      .get<Purpose>(withLib('/wiki/purpose', libSlug))
      .then((p) => {
        setP(p)
        setGoals(p.goals.join('\n'))
        setQuestions(p.key_questions.join('\n'))
        setScope(p.scope.join('\n'))
      })
      .catch(() => {})
  }, [libSlug])
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
              await api.put(withLib('/wiki/purpose', libSlug), {
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



/** Wiki 域：Obsidian 式浏览 —— 目录树（folder 层级）+ Markdown 阅读 + 图谱独立视图。
 *  洞察/Lint/原料/目标 = 运维二级入口（2026-10-05 拍板：运维收敛为只读巡检报告，maintain_wiki Agent 自动化）。
 *  单库终局（2026-09-20）：库选择/建库/删库 UI 已移除，lib 固定 main，/wiki/* 请求仍带 ?lib=；
 *  POST /wiki/search 例外走 body.library（当前前端无该调用点）。
 *  2026-09-03 审计 28 项全修：布局骨架 / 状态提升与 URL / 视觉层次 / 排版 / 可访问性 / 健壮性。 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { CSSProperties, MouseEvent as ReactMouseEvent } from 'react'
import { ChevronRight, FileText, Folder, FolderOpen, Inbox } from 'lucide-react'
import WikiGraph from '@/components/WikiGraph'
import WikiMarkdown from '@/components/WikiMarkdown'
import { useSearchParams } from 'react-router-dom'
import { api, type GraphDto, type WikiPage, type WikiPageMeta } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner, Tabs } from '@/components/ui-bits'
import { fmtTime, inputCls, relTime, selectCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

type View = 'tree' | 'graph'
type Panel = 'none' | 'ops'
const TREE_W_MIN = 220
const TREE_W_MAX = 480

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
  // —— 状态提升（审计 #4）：切图谱 / 运维再回来，选中与折叠不丢 ——
  const [pages, setPages] = useState<WikiPageMeta[] | null>(null)
  // 目录骨架索引（folder → 页数，规模化 2026-09-20 懒加载）：先渲染结构，页面按需拉
  const [folderIndex, setFolderIndex] = useState<Record<string, number>>({})
  // 已拉取的 folder 子树（'' = 根层）——避免重复请求
  const loadedRef = useRef<Set<string>>(new Set())
  const pagesRef = useRef<WikiPageMeta[]>([])
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

  // 规模化（2026-09-20，用户反馈「加载非常卡」）：首屏只拉目录骨架 + 根层页面，
  // 子 folder 在展开时按需拉取——万页下从 18MB/3.2s 降到几 KB 起步。
  const load = useCallback(() => {
    return Promise.all([
      api.get<[string, number][]>('/wiki/folders'),
      api.get<WikiPageMeta[]>('/wiki/pages?folder='),
    ])
      .then(([idx, rootPages]) => {
        setFolderIndex(Object.fromEntries(idx))
        loadedRef.current = new Set([''])
        pagesRef.current = rootPages
        setPages(rootPages)
        setLoadErr('')
      })
      .catch((e: unknown) => setLoadErr(e instanceof Error ? e.message : '目录树加载失败'))
  }, [lib])

  /** 展开 folder 时按需拉它的子树（幂等，同一 folder 只拉一次；失败允许重试）。 */
  const ensureFolder = useCallback(
    async (path: string) => {
      if (loadedRef.current.has(path)) return
      loadedRef.current.add(path)
      try {
        const rows = await api.get<WikiPageMeta[]>(
          `/wiki/pages?folder=${encodeURIComponent(path)}`,
        )
        const seen = new Set(pagesRef.current.map((p) => p.slug))
        const add = rows.filter((r) => !seen.has(r.slug))
        if (add.length) {
          pagesRef.current = [...pagesRef.current, ...add]
          setPages(pagesRef.current)
        }
      } catch {
        loadedRef.current.delete(path)
      }
    },
    [lib],
  )
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
    // oxlint-disable-next-line react/set-state-in-effect -- 置 loading 先于异步拉取，非同步级联
    setOpening(true)
    let stale = false
    api
      .get<WikiPage>(`/wiki/pages/${encodeURIComponent(slug)}`)
      .then((p) => {
        if (stale) return
        setOpen(p)
        setOpenErr('')
        // 懒加载：深链页所在 folder 的子树按需拉取（根层已拉）
        if (p.folder) void ensureFolder(p.folder)
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
          `/wiki/pages/${encodeURIComponent(targetSlug)}`,
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
      const page = await api.get<WikiPage>(`/wiki/pages/${encodeURIComponent(slug)}`)
      setOpen(page)
      if (page.folder) void ensureFolder(page.folder)
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
    const expanding = collapsed.has(p)
    setCollapsed((prev) => {
      const next = new Set(prev)
      if (next.has(p)) next.delete(p)
      else next.add(p)
      lsSet('engram-wiki-collapsed', [...next])
      return next
    })
    // 懒加载：展开时按需拉该 folder 子树（已拉过则直接返回）
    if (expanding) void ensureFolder(p)
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

  // 高度实算（#2）：工作区只在树/图视图锁定视口高度（3rem = main 上下 py-6），运维自然流不受限
  const bounded = panel === 'none'
  return (
    <div className={cn('flex flex-col gap-4 lg:gap-5', bounded && 'lg:h-[calc(100vh-3rem)]')}>
      <div className="shrink-0">
        <PageHeader title="Wiki">
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
              <Button size="sm" variant="outline" onClick={() => setPanel('ops')}>
                运维
              </Button>
            </>
          )}
        </PageHeader>
      </div>
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
              counts={folderIndex}
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
              onSaved={load}
              onPageUpdated={setOpen}
              onNavigateSlug={onSelect}
            />
          </div>
        </div>
      )}
      {panel === 'none' && view === 'graph' && <GraphPane libSlug={lib} />}
    </div>
  )
}

interface FolderNode {
  name: string
  folders: FolderNode[]
  pages: WikiPageMeta[]
}

function buildFolders(pages: WikiPageMeta[], index: Record<string, number> = {}): FolderNode {
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
  for (const path of Object.keys(index)) {
    const parts = path
      .split('/')
      .map((s) => s.trim())
      .filter(Boolean)
    if (parts.length) ensure(root, parts)
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
  counts,
}: {
  pages: WikiPageMeta[] | null
  loadErr: string
  onRetry: () => void
  openId: string | null
  collapsed: Set<string>
  onSelect: (slug: string) => void
  onToggleFolder: (p: string) => void
  counts: Record<string, number>
}) {
  const tree = useMemo(() => (pages ? buildFolders(pages, counts) : null), [pages, counts])
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
          <Empty text="还没有页面——在「原料」tab 上传文档，维护 Agent 会整理出目录树" />
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
                counts={counts}
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
  counts,
}: {
  node: FolderNode
  path: string
  depth: number
  openId: string | null
  onSelect: (slug: string) => void
  collapsed: Set<string>
  onToggleFolder: (p: string) => void
  counts: Record<string, number>
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
                {counts[fp] ?? countPages(f)}
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
                  counts={counts}
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
  onSaved,
  onPageUpdated,
  onNavigateSlug,
}: {
  page: WikiPage | null
  opening: boolean
  openErr: string
  hasPages: boolean
  libSlug: string
  onSaved: () => void
  onPageUpdated: (p: WikiPage) => void
  onNavigateSlug: (slug: string) => void
}) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')
  const [title, setTitle] = useState('')
  const [folder, setFolder] = useState('')
  const [saving, setSaving] = useState(false)
  const [saveErr, setSaveErr] = useState('')
  const [showVersions, setShowVersions] = useState(false)
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
          {hasPages && <p className="text-xs text-muted-foreground/80">Agent 整理的页面按文件夹层级自动归档</p>}
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
          <div className="flex shrink-0 gap-2">
          <Button size="sm" variant="outline" onClick={() => setShowVersions(true)}>
            历史
          </Button>
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
        </div>
        {showVersions ? (
          <VersionsPanel
            slug={page.slug}
            currentContent={page.content}
            onClose={() => setShowVersions(false)}
            onRestored={onSaved}
            onPageUpdated={onPageUpdated}
          />
        ) : (
          <div className="min-h-0 flex-1 overflow-y-auto p-4 [scrollbar-gutter:stable] md:p-6">
            <div className="mx-auto w-full max-w-4xl">
              <WikiMarkdown content={page.content} onNavigateSlug={onNavigateSlug} />
            </div>
          </div>
        )}
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
                // P019-M3：folder 恒传字符串——空串 = 回根目录（后端 COALESCE 只对 null 保持原值，
                // 旧实现 || undefined 使清空无效，placeholder 承诺无法兑现）
                const updated = await api.put<WikiPage>(
                  `/wiki/pages/${encodeURIComponent(page.slug)}`,
                  {
                    title,
                    content: draft,
                    folder: folder.trim(),
                  },
                )
                setEditing(false)
                // P019-M3：保存后用返回的新页刷新阅读区（旧实现只刷目录树，正文/版本号停在旧内容）
                onPageUpdated(updated)
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

interface PageVersionRow {
  version: number
  created_at: string
  via?: string | null
  title?: string | null
  [k: string]: unknown
}

/** 版本历史：时间线+预览对比+回滚。 */
function VersionsPanel({
  slug,
  currentContent,
  onClose,
  onRestored,
  onPageUpdated,
}: {
  slug: string
  currentContent: string
  onClose: () => void
  onRestored: () => void
  onPageUpdated: (p: WikiPage) => void
}) {
  const [rows, setRows] = useState<PageVersionRow[] | null>(null)
  const [sel, setSel] = useState<number | null>(null)
  const [selContent, setSelContent] = useState('')
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState('')

  const load = useCallback(() => {
    api
      .get<{ versions: PageVersionRow[] }>(
        `/wiki/pages/${encodeURIComponent(slug)}/versions`,
      )
      .then((v) => setRows(v.versions))
      .catch((e) => setErr(String(e)))
  }, [slug])
  useEffect(() => {
    load()
  }, [load])

  const preview = (v: number) => {
    setSel(v)
    api
      .get<{ content?: string; content_markdown?: string }>(
        `/wiki/pages/${encodeURIComponent(slug)}/versions/${v}`,
      )
      .then((r) => setSelContent(r.content ?? r.content_markdown ?? ''))
      .catch((e) => setErr(String(e)))
  }

  const restore = async (v: number) => {
    if (!window.confirm(`回滚到 v${v}？当前内容会先生成新快照，可再滚回来。`)) return
    setBusy(true)
    setErr('')
    try {
      await api.post(`/wiki/pages/${encodeURIComponent(slug)}/restore`, { version: v })
      // P019-M3：回滚后重取当前页刷新阅读区（旧实现只刷目录树，正文/版本号停在旧快照）
      const p = await api.get<WikiPage>(`/wiki/pages/${encodeURIComponent(slug)}`)
      onPageUpdated(p)
      onRestored()
      onClose()
    } catch (e) {
      setErr(String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="min-h-0 flex-1 overflow-y-auto p-4 md:p-6">
      <div className="mb-3 flex items-center justify-between">
        <h3 className="text-sm font-semibold">版本历史 · {slug}</h3>
        <Button size="sm" variant="outline" onClick={onClose}>
          ← 返回页面
        </Button>
      </div>
      {err && <ErrorBox msg={err} />}
      {rows === null ? (
        <Spinner />
      ) : rows.length === 0 ? (
        <Empty text="还没有历史版本" />
      ) : (
        <div className="space-y-4">
          <ol className="space-y-1">
            {rows.map((r) => (
              <li key={r.version} className="flex items-center gap-3 rounded border px-3 py-2 text-sm">
                <span className="font-mono text-xs">v{r.version}</span>
                <span className="text-xs text-muted-foreground">{fmtTime(String(r.created_at))}</span>
                {r.via && <span className="text-xs opacity-60">{String(r.via)}</span>}
                <div className="ml-auto flex gap-1">
                  <Button size="sm" variant="ghost" onClick={() => preview(r.version)}>
                    预览
                  </Button>
                  <Button size="sm" variant="outline" disabled={busy} onClick={() => restore(r.version)}>
                    回滚到此版
                  </Button>
                </div>
              </li>
            ))}
          </ol>
          {sel !== null && (
            <div className="grid gap-3 md:grid-cols-2">
              <div>
                <div className="mb-1 text-xs font-medium">v{sel} 快照</div>
                <textarea readOnly className={inputCls + ' h-72 font-mono text-xs'} value={selContent} />
              </div>
              <div>
                <div className="mb-1 text-xs font-medium">当前内容</div>
                <textarea readOnly className={inputCls + ' h-72 font-mono text-xs'} value={currentContent} />
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  )
}

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

// ---------- 巡检报告（P015 后运维收敛：节律自动巡逻，用户只看报告） ----------

interface PatrolReport {
  lib: string
  lint_issues: number
  lint_checked_pages: number
  lint_summary: [string, number][]
  repair_actions: number
  repair_checked_pages: number
  repair_detail: [string, string][]
  embedding_backfilled: number
  duplicate_candidates: number
  lint_deep_job: string | null
  agent_summary?: string | null
  manual_actions?: string[] | null
  markdown?: string
}

interface PatrolListItem {
  job_id: string
  status: string
  finished_at: string | null
  lint_issues: number
  repair_actions: number
  duplicate_candidates: number
  summary: string | null
}

/** 巡检历史（工单式主从）：左列表（每次巡逻一条）右 Markdown 报告，两栏独立滚动。 */
function PatrolPane() {
  const [list, setList] = useState<PatrolListItem[] | null>(null)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [detail, setDetail] = useState<{ status: string; finished_at: string | null; report: PatrolReport | null } | null>(null)
  const [busy, setBusy] = useState(false)
  const [msg, setMsg] = useState('')
  const load = () =>
    api
      .get<{ items: PatrolListItem[] }>('/wiki/patrol/list')
      .then((v) => {
        setList(v.items)
        if (!selectedId && v.items.length > 0) setSelectedId(v.items[0].job_id)
      })
      .catch((e) => setMsg(String(e)))
  useEffect(() => {
    load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  useEffect(() => {
    if (!selectedId) return
    api
      .get<{ status: string; finished_at: string | null; report: PatrolReport | null }>(`/wiki/patrol/${selectedId}`)
      .then(setDetail)
      .catch((e) => setMsg(String(e)))
  }, [selectedId])
  const trigger = async () => {
    setBusy(true)
    setMsg('')
    try {
      const v = await api.post<{ already_running: boolean; hint?: string }>('/wiki/patrol', {})
      setMsg(v.already_running ? (v.hint ?? '巡逻进行中') : '巡逻已触发——完成后出现在列表里')
      setTimeout(load, 3000)
    } catch (e) {
      setMsg(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }
  const r = detail?.report
  return (
    <div className="space-y-3" data-testid="patrol-pane">
      <div className="flex items-center justify-between">
        <p className="text-xs text-muted-foreground">
          维护 Agent 每天自动巡逻（lint 体检 → 确定性修复 → 重复页报告 → 深度检查），每次巡逻一条报告。
        </p>
        <Button size="sm" variant="outline" disabled={busy} onClick={trigger}>
          {busy ? '触发中…' : '立即巡逻'}
        </Button>
      </div>
      {msg && <p className="text-xs text-muted-foreground">{msg}</p>}
      {!list ? (
        <Spinner />
      ) : list.length === 0 ? (
        <Empty text="还没有巡逻记录——等节律自动跑，或点上面立即巡逻" />
      ) : (
        <div
          className={cn(
            'grid grid-cols-1 gap-4',
            'lg:grid-cols-[minmax(0,18rem)_minmax(0,1fr)] lg:h-[calc(100dvh-13rem)]',
          )}
        >
          {/* 左：巡逻历史列表（独立滚动） */}
          <div className="min-w-0 space-y-1.5 lg:h-full lg:overflow-y-auto lg:pr-1">
            {list.map((it) => (
              <button
                key={it.job_id}
                onClick={() => setSelectedId(it.job_id)}
                className={cn(
                  'w-full rounded-md border p-2.5 text-left transition-colors',
                  selectedId === it.job_id
                    ? 'border-ring bg-muted/60'
                    : 'border-border hover:bg-muted/40',
                  it.status !== 'succeeded' && 'opacity-70',
                )}
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="text-sm font-medium">
                    {it.finished_at ? fmtTime(it.finished_at) : it.job_id.slice(0, 8)}
                  </span>
                  <span
                    className={cn(
                      'text-xs',
                      it.status === 'succeeded' ? 'text-muted-foreground' : 'text-warning',
                    )}
                  >
                    {it.status === 'succeeded' ? '完成' : it.status}
                  </span>
                </div>
                <div className="mt-0.5 text-xs text-muted-foreground">
                  lint {it.lint_issues} · 修复 {it.repair_actions} · 重复 {it.duplicate_candidates}
                </div>
                {it.summary && <div className="mt-1 line-clamp-2 text-xs text-muted-foreground">{it.summary}</div>}
              </button>
            ))}
          </div>
          {/* 右：Markdown 巡逻报告（独立滚动） */}
          <div className="min-w-0 lg:h-full lg:overflow-y-auto lg:pl-1">
            {!detail ? (
              <Spinner />
            ) : detail.status !== 'succeeded' ? (
              <Empty text={`该次巡逻未完成（${detail.status}）`} />
            ) : !r ? (
              <Empty text="报告缺失" />
            ) : r.markdown ? (
              <Card className="p-5">
                <WikiMarkdown content={r.markdown} />
              </Card>
            ) : (
              <Card className="space-y-3 p-4">
                <div className="text-sm font-medium">
                  巡检报告 · {r.lint_checked_pages} 页体检 / {r.repair_checked_pages} 页修复扫描
                </div>
                <div className="text-xs text-muted-foreground">
                  lint {r.lint_issues} · 修复 {r.repair_actions} · 重复候选 {r.duplicate_candidates}
                </div>
                {r.agent_summary && <p className="text-xs text-muted-foreground">{r.agent_summary}</p>}
                {r.manual_actions && r.manual_actions.length > 0 && (
                  <ul className="list-disc space-y-0.5 pl-4 text-xs text-muted-foreground">
                    {r.manual_actions.map((a, i) => (
                      <li key={i}>{a}</li>
                    ))}
                  </ul>
                )}
              </Card>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

/** 运维入口（收敛后只剩巡检报告——lint/repair/duplicates/insights 全部 Agent 自动化）。 */
function OpsPanel({ lib }: { lib: string }) {
  return <PatrolPane key={lib} />
}

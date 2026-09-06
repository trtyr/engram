/**
 * 项目详情：Wiki 式左树右内容——左树 = 位置（多主机）+ 分类（文档计数），
 * 右侧 = 项目概览 / 位置详情 / 文档 markdown 阅读与编辑。
 * 「规划」是 dev 类型的一个分类（0004 plan-tree 消融），非特殊区块。
 */
import { useCallback, useEffect, useState } from 'react'
import { Link, useParams } from 'react-router-dom'
import { api, type ProjectDetailDto, type ProjectDocDto, type ProjectLocationDto } from '@/lib/api'
import { Card, ErrorBox, Spinner } from '@/components/ui-bits'
import { fmtTime, inputCls, selectCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import WikiMarkdown from '@/components/WikiMarkdown'

const TYPE_LABEL: Record<string, string> = { dev: '开发', research: '调研' }
const STATUS_LABEL: Record<string, string> = {
  active: '进行中',
  paused: '暂停',
  done: '完成',
  abandoned: '放弃',
}

type Sel =
  | { kind: 'overview' }
  | { kind: 'location'; loc: ProjectLocationDto }
  | { kind: 'doc'; doc: ProjectDocDto }

export default function ProjectDetail() {
  const { id } = useParams<{ id: string }>()
  const [detail, setDetail] = useState<ProjectDetailDto | null>(null)
  const [err, setErr] = useState('')
  const [sel, setSel] = useState<Sel>({ kind: 'overview' })
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())

  const load = useCallback(() => {
    if (!id) return
    api
      .get<ProjectDetailDto>(`/projects/${id}`)
      .then(setDetail)
      .catch((e) => setErr(e instanceof Error ? e.message : '加载失败'))
  }, [id])

  useEffect(() => {
    load()
  }, [load])

  const toggleGroup = (key: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }

  if (err && !detail) return <ErrorBox msg={err} />
  if (!detail) return <Spinner />

  const docsByCat = (cat: string) => detail.docs.filter((d) => d.category === cat)

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <Link to="/projects" className="text-sm text-muted-foreground hover:text-foreground">
          ← 项目
        </Link>
        <h2 className="text-lg font-semibold">{detail.name}</h2>
        <span className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">
          {TYPE_LABEL[detail.type] ?? detail.type}
        </span>
        <span className="text-xs text-muted-foreground">{STATUS_LABEL[detail.status] ?? detail.status}</span>
      </div>

      {detail.description && <p className="text-sm text-muted-foreground">{detail.description}</p>}

      <div className="flex gap-4">
        {/* 左树 */}
        <div className="w-64 shrink-0">
          <Card className="p-2">
            <nav aria-label="项目目录" className="space-y-0.5">
              <GroupHeader label="📄 文档" count={detail.docs.length} open={!collapsed.has('docs')} onToggle={() => toggleGroup('docs')} />

              {!collapsed.has('docs') &&
                detail.categories.map((cat) => {
                  const docs = docsByCat(cat)
                  const key = `cat-${cat}`
                  return (
                    <div key={cat} className="pl-2">
                      <div className="flex items-center justify-between">
                        <button
                          type="button"
                          onClick={() => toggleGroup(key)}
                          className="flex-1 truncate rounded px-2 py-1 text-left text-sm hover:bg-muted"
                        >
                          {collapsed.has(key) ? '▸' : '▾'} {cat}
                          <span className="ml-1 text-xs text-muted-foreground">({docs.length})</span>
                        </button>
                        <button
                          type="button"
                          title={`在「${cat}」下新建文档`}
                          onClick={() => setSel({ kind: 'doc', doc: newDoc(cat) })}
                          className="px-1 text-xs text-muted-foreground hover:text-foreground"
                        >
                          +
                        </button>
                      </div>
                      {!collapsed.has(key) &&
                        docs.map((d) => (
                          <button
                            key={d.id}
                            type="button"
                            onClick={() => setSel({ kind: 'doc', doc: d })}
                            className="block w-full truncate rounded px-2 py-1 pl-4 text-left text-sm text-muted-foreground hover:bg-muted"
                          >
                            {d.title}
                          </button>
                        ))}
                    </div>
                  )
                })}
            </nav>
          </Card>
        </div>

        {/* 右内容 */}
        <div className="min-w-0 flex-1">
          {sel.kind === 'overview' && (
            <OverviewPane detail={detail} onOpenLoc={(loc) => setSel({ kind: 'location', loc })} onAddLoc={() => setSel({ kind: 'location', loc: NEW_LOC })} onOpenDoc={(d) => setSel({ kind: 'doc', doc: d })} />
          )}
          {sel.kind === 'location' && (
            <LocationPane
              projectId={detail.id}
              loc={sel.loc}
              onChanged={() => {
                setSel({ kind: 'overview' })
                load()
              }}
            />
          )}
          {sel.kind === 'doc' && (
            <DocPane
              projectId={detail.id}
              doc={sel.doc}
              categories={detail.categories}
              onChanged={() => {
                setSel({ kind: 'overview' })
                load()
              }}
            />
          )}
        </div>
      </div>
      {err && <ErrorBox msg={err} />}
    </div>
  )
}

/** 新建位置的占位对象（id 为空表示「新建」模式）。 */
const NEW_LOC: ProjectLocationDto = {
  id: '',
  project_id: '',
  ip: '',
  host: '',
  os: '',
  path: '',
  purpose: null,
  sort_order: 0,
  created_at: '',
  updated_at: '',
}

function newDoc(category: string): ProjectDocDto {
  return {
    id: '',
    project_id: '',
    category,
    title: '',
    content: '',
    frontmatter: {},
    created_at: '',
    updated_at: '',
  }
}

function GroupHeader({
  label,
  count,
  open,
  onToggle,
  onAdd,
}: {
  label: string
  count: number
  open: boolean
  onToggle: () => void
  onAdd?: () => void
}) {
  return (
    <div className="flex items-center justify-between">
      <button type="button" onClick={onToggle} className="flex-1 rounded px-2 py-1 text-left text-sm font-medium hover:bg-muted">
        {open ? '▾' : '▸'} {label}
        <span className="ml-1 text-xs text-muted-foreground">({count})</span>
      </button>
      {onAdd && (
        <button type="button" title="新增" onClick={onAdd} className="px-1 text-xs text-muted-foreground hover:text-foreground">
          +
        </button>
      )}
    </div>
  )
}

/** 概览：位置元数据（项目基本信息）+ 分类文档统计。 */
function OverviewPane({
  detail,
  onOpenLoc,
  onAddLoc,
  onOpenDoc,
}: {
  detail: ProjectDetailDto
  onOpenLoc: (loc: ProjectLocationDto) => void
  onAddLoc: () => void
  onOpenDoc: (doc: ProjectDocDto) => void
}) {
  return (
    <div className="space-y-4">
      <Card className="p-4">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold">📍 位置（{detail.locations.length}）</h3>
          <button type="button" onClick={onAddLoc} className="px-1 text-xs text-muted-foreground hover:text-foreground" title="登记位置">
            + 登记
          </button>
        </div>
        {detail.locations.length === 0 ? (
          <p className="mt-2 text-sm text-muted-foreground">还没有登记位置——记录项目在哪个 IP + 主机 + 路径（元数据）。</p>
        ) : (
          <ul className="mt-2 space-y-2">
            {detail.locations.map((loc) => (
              <li key={loc.id}>
                <button type="button" onClick={() => onOpenLoc(loc)} className="w-full rounded border border-border px-3 py-2 text-left text-sm hover:bg-muted/40">
                  <div className="flex items-center gap-2">
                    <span className="font-mono font-medium">{loc.ip}</span>
                    <span className="text-muted-foreground">{loc.host}</span>
                    <span className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">{loc.os}</span>
                  </div>
                  <div className="mt-0.5 text-xs text-muted-foreground">
                    {loc.path}
                    {loc.purpose && <span className="ml-1">（{loc.purpose}）</span>}
                  </div>
                </button>
              </li>
            ))}
          </ul>
        )}
      </Card>

      <Card className="p-4">
        <h3 className="text-sm font-semibold">📄 文档（{detail.docs.length}）</h3>
        {detail.categories.map((cat) => {
          const docs = detail.docs.filter((d) => d.category === cat)
          return (
            <div key={cat} className="mt-3">
              <p className="text-xs font-medium text-muted-foreground">
                {cat}（{docs.length}）
              </p>
              {docs.length === 0 ? (
                <p className="mt-1 text-xs text-muted-foreground/70">暂无文档</p>
              ) : (
                <ul className="mt-1 space-y-1">
                  {docs.map((d) => (
                    <li key={d.id}>
                      <button type="button" onClick={() => onOpenDoc(d)} className="text-left text-sm hover:underline">
                        {d.title}
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )
        })}
      </Card>
    </div>
  )
}

/** 位置编辑（新建或编辑）。 */
function LocationPane({
  projectId,
  loc,
  onChanged,
}: {
  projectId: string
  loc: ProjectLocationDto
  onChanged: () => void
}) {
  const isNew = loc.id === ''
  const [ip, setIp] = useState(loc.ip)
  const [host, setHost] = useState(loc.host)
  const [os, setOs] = useState(loc.os)
  const [path, setPath] = useState(loc.path)
  const [purpose, setPurpose] = useState(loc.purpose ?? '')
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState('')

  async function save() {
    if (!ip.trim() || !host.trim() || !os.trim() || !path.trim()) return
    setBusy(true)
    try {
      const body = { ip: ip.trim(), host: host.trim(), os: os.trim(), path: path.trim(), purpose: purpose.trim() || null }
      if (isNew) {
        await api.post(`/projects/${projectId}/locations`, body)
      } else {
        await api.put(`/projects/${projectId}/locations/${loc.id}`, body)
      }
      onChanged()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '保存失败')
    } finally {
      setBusy(false)
    }
  }

  async function remove() {
    if (isNew) return onChanged()
    setBusy(true)
    try {
      await api.del(`/projects/${projectId}/locations/${loc.id}`)
      onChanged()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '删除失败')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card className="space-y-3 p-4">
      <h3 className="text-sm font-semibold">{isNew ? '新增位置' : '编辑位置'}</h3>
      <input className={`${inputCls} w-full`} placeholder="IP（内网/公网/IPv6，如 192.168.1.5 / 82.157.147.224）" value={ip} onChange={(e) => setIp(e.target.value)} />
      <input className={`${inputCls} w-full`} placeholder="主机名（如 tencent-beijing / MacBook Pro）" value={host} onChange={(e) => setHost(e.target.value)} />
      <input className={`${inputCls} w-full`} placeholder="操作系统（macOS / Ubuntu / Windows…）" value={os} onChange={(e) => setOs(e.target.value)} />
      <input className={`${inputCls} w-full`} placeholder="路径（如 ~/Documents/Code/Rust/engram）" value={path} onChange={(e) => setPath(e.target.value)} />
      <input className={`${inputCls} w-full`} placeholder="用途（开发 / 部署 / …，可选）" value={purpose} onChange={(e) => setPurpose(e.target.value)} />
      {err && <ErrorBox msg={err} />}
      <div className="flex gap-2">
        <Button size="sm" disabled={busy || !ip.trim() || !host.trim() || !os.trim() || !path.trim()} onClick={save}>
          {isNew ? '添加' : '保存'}
        </Button>
        {!isNew && (
          <Button size="sm" variant="destructive" disabled={busy} onClick={remove}>
            删除
          </Button>
        )}
      </div>
    </Card>
  )
}

/** 文档阅读与编辑。 */
function DocPane({
  projectId,
  doc,
  categories,
  onChanged,
}: {
  projectId: string
  doc: ProjectDocDto
  categories: string[]
  onChanged: () => void
}) {
  const isNew = doc.id === ''
  const [editing, setEditing] = useState(isNew)
  const [category, setCategory] = useState(doc.category)
  const [title, setTitle] = useState(doc.title)
  const [content, setContent] = useState(doc.content)
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState('')

  async function save() {
    if (!title.trim()) return
    setBusy(true)
    try {
      if (isNew) {
        await api.post(`/projects/${projectId}/docs`, { category, title: title.trim(), content })
      } else {
        await api.put(`/projects/${projectId}/docs/${doc.id}`, { category, title: title.trim(), content })
      }
      onChanged()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '保存失败')
    } finally {
      setBusy(false)
    }
  }

  async function remove() {
    if (isNew) return onChanged()
    setBusy(true)
    try {
      await api.del(`/projects/${projectId}/docs/${doc.id}`)
      onChanged()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '删除失败')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card className="space-y-3 p-4">
      {editing ? (
        <>
          <div className="flex gap-2">
            <select className={selectCls} value={category} onChange={(e) => setCategory(e.target.value)}>
              {categories.map((c) => (
                <option key={c}>{c}</option>
              ))}
              {!categories.includes(category) && <option>{category}</option>}
            </select>
            <input className={`${inputCls} flex-1`} placeholder="文档标题" value={title} onChange={(e) => setTitle(e.target.value)} />
          </div>
          <textarea
            className={`${inputCls} min-h-[40vh] w-full font-mono text-sm`}
            placeholder="# Markdown 正文"
            value={content}
            onChange={(e) => setContent(e.target.value)}
          />
          {err && <ErrorBox msg={err} />}
          <div className="flex gap-2">
            <Button size="sm" disabled={busy || !title.trim()} onClick={save}>
              {isNew ? '创建' : '保存'}
            </Button>
            <Button size="sm" variant="ghost" onClick={() => (isNew ? onChanged() : setEditing(false))}>
              取消
            </Button>
            {!isNew && (
              <Button size="sm" variant="destructive" disabled={busy} onClick={remove}>
                删除
              </Button>
            )}
          </div>
        </>
      ) : (
        <>
          <div className="flex items-center justify-between">
            <div>
              <h3 className="text-base font-semibold">{doc.title}</h3>
              <p className="mt-0.5 text-xs text-muted-foreground">
                {doc.category} · {fmtTime(doc.updated_at)}
              </p>
            </div>
            <Button size="sm" variant="outline" onClick={() => setEditing(true)}>
              编辑
            </Button>
          </div>
          <WikiMarkdown content={doc.content} />
        </>
      )}
    </Card>
  )
}

/**
 * 项目详情：Wiki 式左树右内容——左树 = 位置（多主机）+ 分类（文档计数），
 * 右侧 = 项目概览 / 位置详情 / 文档 markdown 阅读与编辑。
 * 「规划」是 dev 类型的一个分类（0004 plan-tree 消融），非特殊区块。
 */
import { useCallback, useEffect, useState } from 'react'
import { Link, useParams } from 'react-router-dom'
import {
  api,
  type AssetDto,
  type ProjectDetailDto,
  type ProjectDocDto,
  type ProjectFileDto,
  type ProjectLocationDto,
} from '@/lib/api'
import { Card, ErrorBox, Spinner } from '@/components/ui-bits'
import { fmtTime, inputCls, selectCls } from '@/lib/ui'
import { cn } from '@/lib/utils'
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
  | { kind: 'file'; name: string | null }

/** folder 树节点（category 内的子目录递归结构）。 */
interface FolderNode {
  name: string
  path: string
  folders: FolderNode[]
  docs: ProjectDocDto[]
}

function buildFolderTree(docs: ProjectDocDto[]): { rootDocs: ProjectDocDto[]; folders: FolderNode[] } {
  const rootDocs: ProjectDocDto[] = []
  const folders: FolderNode[] = []
  for (const d of docs) {
    const segs = (d.folder ?? '').split('/').filter(Boolean)
    let list = folders
    let node: FolderNode | undefined
    let path = ''
    for (const seg of segs) {
      path = path ? `${path}/${seg}` : seg
      node = list.find((n) => n.name === seg)
      if (!node) {
        node = { name: seg, path, folders: [], docs: [] }
        list.push(node)
      }
      list = node.folders
    }
    if (node) node.docs.push(d)
    else rootDocs.push(d)
  }
  const sortRec = (ns: FolderNode[]) => {
    ns.sort((a, b) => a.name.localeCompare(b.name))
    ns.forEach((n) => {
      n.docs.sort((a, b) => a.title.localeCompare(b.title))
      sortRec(n.folders)
    })
  }
  sortRec(folders)
  rootDocs.sort((a, b) => a.title.localeCompare(b.title))
  return { rootDocs, folders }
}

function countAll(n: FolderNode): number {
  return n.docs.length + n.folders.reduce((s, f) => s + countAll(f), 0)
}

/** 递归 folder 树：folder 节点可折叠，叶子为文档。 */
function FolderTreeView({
  folders,
  rootDocs,
  treeKey,
  depth,
  collapsed,
  onToggle,
  selectedId,
  onSelect,
}: {
  folders: FolderNode[]
  rootDocs: ProjectDocDto[]
  treeKey: string
  depth: number
  collapsed: Set<string>
  onToggle: (key: string) => void
  selectedId: string | null
  onSelect: (d: ProjectDocDto) => void
}) {
  const pad = { paddingLeft: 8 + depth * 10 }
  return (
    <>
      {folders.map((n) => {
        const key = `${treeKey}/${n.path}`
        const open = !collapsed.has(key)
        return (
          <div key={n.path}>
            <button
              type="button"
              onClick={() => onToggle(key)}
              className="block w-full truncate rounded px-2 py-1 text-left text-xs font-medium text-muted-foreground hover:bg-muted"
              style={pad}
            >
              {open ? '▾' : '▸'} 📁 {n.name}
              <span className="ml-1 text-[10px] opacity-70">({countAll(n)})</span>
            </button>
            {open && (
              <FolderTreeView
                folders={n.folders}
                rootDocs={n.docs}
                treeKey={key}
                depth={depth + 1}
                collapsed={collapsed}
                onToggle={onToggle}
                selectedId={selectedId}
                onSelect={onSelect}
              />
            )}
          </div>
        )
      })}
      {rootDocs.map((d) => (
        <button
          key={d.id}
          type="button"
          onClick={() => onSelect(d)}
          className={cn(
            'block w-full truncate rounded px-2 py-1 text-left text-sm hover:bg-muted',
            selectedId === d.id ? 'bg-muted font-medium' : 'text-muted-foreground',
          )}
          style={pad}
        >
          {d.title}
        </button>
      ))}
    </>
  )
}

export default function ProjectDetail() {
  const { id } = useParams<{ id: string }>()
  const [detail, setDetail] = useState<ProjectDetailDto | null>(null)
  const [err, setErr] = useState('')
  const [sel, setSel] = useState<Sel>({ kind: 'overview' })
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
  const [files, setFiles] = useState<ProjectFileDto[]>([])

  const load = useCallback(() => {
    if (!id) return
    api
      .get<ProjectDetailDto>(`/projects/${id}`)
      .then(setDetail)
      .catch((e) => setErr(e instanceof Error ? e.message : '加载失败'))
    api
      .get<ProjectFileDto[]>(`/projects/${id}/files`)
      .then(setFiles)
      .catch(() => setFiles([]))
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
    <div className="flex flex-col gap-4 lg:h-[calc(100vh-3rem)]">
      <div className="shrink-0">
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
        {detail.description && <p className="mt-1 text-sm text-muted-foreground">{detail.description}</p>}
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-4 lg:flex-row">
        {/* 左树（独立滚动） */}
        <div className="flex w-full shrink-0 flex-col lg:w-64">
          <Card className="flex min-h-0 flex-1 flex-col overflow-hidden p-2">
            <nav aria-label="项目目录" className="min-h-0 flex-1 space-y-0.5 overflow-y-auto">
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
                        (() => {
                          const tree = buildFolderTree(docsByCat(cat))
                          return (
                            <FolderTreeView
                              folders={tree.folders}
                              rootDocs={tree.rootDocs}
                              treeKey={key}
                              depth={1}
                              collapsed={collapsed}
                              onToggle={toggleGroup}
                              selectedId={sel.kind === 'doc' ? sel.doc.id : null}
                              onSelect={(d) => setSel({ kind: 'doc', doc: d })}
                            />
                          )
                        })()}
                    </div>
                  )
                })}

              {/* 📎 项目文件（架构图 HTML 等制品；0045） */}
              <div className="mt-1 border-t border-border pt-1">
                <GroupHeader
                  label="📎 文件"
                  count={files.length}
                  open={!collapsed.has('files')}
                  onToggle={() => toggleGroup('files')}
                  onAdd={() => setSel({ kind: 'file', name: null })}
                />
                {!collapsed.has('files') &&
                  files.map((f) => (
                    <button
                      key={f.id}
                      type="button"
                      onClick={() => setSel({ kind: 'file', name: f.name })}
                      className={cn(
                        'block w-full truncate rounded px-2 py-1 text-left text-sm hover:bg-muted',
                        sel.kind === 'file' && sel.name === f.name ? 'bg-muted font-medium' : 'text-muted-foreground',
                      )}
                    >
                      {f.mime === 'text/html' ? '🖼️' : '📄'} {f.name}
                      <span className="ml-1 text-xs text-muted-foreground">v{f.version}</span>
                    </button>
                  ))}
              </div>
            </nav>
          </Card>
        </div>

        {/* 右内容（独立滚动） */}
        <div className="flex min-h-[45vh] min-w-0 flex-1 flex-col lg:min-h-0">
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
          {sel.kind === 'file' && (
            <FilePane projectId={detail.id} name={sel.name} onChanged={load} />
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
  asset_id: null,
  created_at: '',
  updated_at: '',
}

function newDoc(category: string): ProjectDocDto {
  return {
    id: '',
    project_id: '',
    category,
    folder: '',
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
    <div className="min-h-0 flex-1 space-y-4 overflow-y-auto [scrollbar-gutter:stable] lg:pr-1">
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

      {/* 关系：用到的资产 + 工作线关联（隶属 / 下属 / 相关）——资产身份在台账，这里只展示引用 */}
      <Card className="p-4">
        <h3 className="text-sm font-semibold">🔗 关系</h3>
        <div className="mt-2 space-y-3">
          <div>
            <p className="text-xs font-medium text-muted-foreground">
              用到的资产（{detail.assets.length}）
            </p>
            {detail.assets.length === 0 ? (
              <p className="mt-1 text-xs text-muted-foreground/70">
                还没有引用资产——登记位置时选一条资产台账条目（身份以台账为准，别在这儿重抄）。
              </p>
            ) : (
              <ul className="mt-1 space-y-1">
                {detail.assets.map((a) => (
                  <li key={a.location_id} className="text-sm">
                    <Link to="/assets" className="text-info hover:underline">
                      {a.name}
                    </Link>
                    <span className="text-xs text-muted-foreground">
                      {' '}
                      · {a.host}
                      {a.path && ` · ${a.path}`}
                      {a.purpose && ` · ${a.purpose}`}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </div>
          {(() => {
            const belongs = detail.links.filter(
              (l) => l.kind === 'part_of' && l.from_project === detail.id,
            )
            const owns = detail.links.filter(
              (l) => l.kind === 'part_of' && l.to_project === detail.id,
            )
            const related = detail.links.filter((l) => l.kind === 'related')
            const row = (label: string, items: { id: string; name: string; note: string }[]) =>
              items.length > 0 && (
                <div key={label}>
                  <p className="text-xs font-medium text-muted-foreground">
                    {label}（{items.length}）
                  </p>
                  <ul className="mt-1 space-y-1">
                    {items.map((x) => (
                      <li key={x.id} className="text-sm">
                        <Link to={`/projects/${x.id}`} className="text-info hover:underline">
                          {x.name}
                        </Link>
                        {x.note && (
                          <span className="text-xs text-muted-foreground"> · {x.note}</span>
                        )}
                      </li>
                    ))}
                  </ul>
                </div>
              )
            return (
              <>
                {row(
                  '隶属（我属于）',
                  belongs.map((l) => ({ id: l.to_project, name: l.to_name, note: l.note })),
                )}
                {row(
                  '下属（属于我的）',
                  owns.map((l) => ({ id: l.from_project, name: l.from_name, note: l.note })),
                )}
                {row(
                  '相关',
                  related.map((l) => ({
                    id: l.from_project === detail.id ? l.to_project : l.from_project,
                    name: l.from_project === detail.id ? l.to_name : l.from_name,
                    note: l.note,
                  })),
                )}
                {detail.links.length === 0 && (
                  <p className="text-xs text-muted-foreground/70">
                    没有关联的工作线——只有真有独立推进节奏的子工作线才挂「隶属」（判据见《项目与资产模型 · README》§2.2）。
                  </p>
                )}
              </>
            )
          })()}
        </div>
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
  // 资产引用（0058 真引用）：选中即写 asset_id；选「无」= 纯文本位置
  const [assetId, setAssetId] = useState(loc.asset_id ?? '')
  const [assets, setAssets] = useState<AssetDto[]>([])
  useEffect(() => {
    api.get<AssetDto[]>('/assets').then(setAssets).catch(() => {})
  }, [])

  async function save() {
    if (!path.trim()) return
    if (!assetId && (!ip.trim() || !host.trim() || !os.trim())) return
    setBusy(true)
    try {
      const body = {
        ip: ip.trim(),
        host: host.trim(),
        os: os.trim(),
        path: path.trim(),
        purpose: purpose.trim() || null,
        // PUT 是整体替换：不带上它就会把已有引用抹掉（后端 LocationRequest.asset_id 语义）
        asset_id: assetId || null,
      }
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
    <Card className="min-h-0 flex-1 space-y-3 overflow-y-auto p-4 [scrollbar-gutter:stable]">
      <h3 className="text-sm font-semibold">{isNew ? '新增位置' : '编辑位置'}</h3>
      <select
        className={selectCls}
        value={assetId}
        onChange={(e) => {
          const v = e.target.value
          setAssetId(v)
          const a = assets.find((x) => x.id === v)
          if (a) {
            // 身份以台账为准：空着的字段从资产带出（已有值不覆盖）
            if (!ip.trim()) setIp(a.ip)
            if (!host.trim()) setHost(a.name)
            if (!os.trim()) setOs(a.os)
          }
        }}
        title="关联资产台账条目（身份以台账为准；选「不关联」= 纯文本位置）"
        data-testid="loc-asset-picker"
      >
        <option value="">不关联资产（纯文本位置）</option>
        {assets.map((a) => (
          <option key={a.id} value={a.id}>
            {a.name}
            {a.ip ? ` · ${a.ip}` : ''}
          </option>
        ))}
      </select>
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
  const [folder, setFolder] = useState(doc.folder ?? '')
  const [title, setTitle] = useState(doc.title)
  const [content, setContent] = useState(doc.content)
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState('')

  async function save() {
    if (!title.trim()) return
    setBusy(true)
    try {
      const body = { category, folder: folder.trim(), title: title.trim(), content }
      if (isNew) {
        await api.post(`/projects/${projectId}/docs`, body)
      } else {
        await api.put(`/projects/${projectId}/docs/${doc.id}`, body)
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
    <Card className="flex min-h-0 flex-1 flex-col gap-3 overflow-hidden p-4">
      {editing ? (
        <>
          <div className="flex shrink-0 gap-2">
            <select className={selectCls} value={category} onChange={(e) => setCategory(e.target.value)}>
              {categories.map((c) => (
                <option key={c}>{c}</option>
              ))}
              {!categories.includes(category) && <option>{category}</option>}
            </select>
            <input
              className={`${inputCls} w-44 font-mono`}
              placeholder="子文件夹（可选，如 审计）"
              aria-label="文档 folder"
              value={folder}
              onChange={(e) => setFolder(e.target.value)}
            />
            <input className={`${inputCls} flex-1`} placeholder="文档标题" value={title} onChange={(e) => setTitle(e.target.value)} />
          </div>
          <textarea
            className={`${inputCls} min-h-[200px] w-full flex-1 resize-none font-mono text-sm leading-5`}
            placeholder="# Markdown 正文"
            value={content}
            onChange={(e) => setContent(e.target.value)}
          />
          {err && <div className="shrink-0"><ErrorBox msg={err} /></div>}
          <div className="flex shrink-0 gap-2">
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
          <div className="flex shrink-0 items-center justify-between">
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
          <div className="min-h-0 flex-1 overflow-y-auto [scrollbar-gutter:stable] lg:pr-1">
            <WikiMarkdown content={doc.content} />
          </div>
        </>
      )}
    </Card>
  )
}


/** 项目文件区：新建/查看/编辑/删除；text/html 用 iframe sandbox 渲染（架构图等制品），markdown 走 WikiMarkdown，其余 <pre>。 */
function FilePane({ projectId, name, onChanged }: { projectId: string; name: string | null; onChanged: () => void }) {
  const isNew = name === null
  const [file, setFile] = useState<ProjectFileDto | null>(null)
  const [editing, setEditing] = useState(isNew)
  const [fileName, setFileName] = useState('')
  const [content, setContent] = useState('')
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState('')

  useEffect(() => {
    setEditing(isNew)
    setFileName(isNew ? '' : (name ?? ''))
    setContent('')
    setFile(null)
    setErr('')
    if (!isNew && name) {
      api
        .get<ProjectFileDto>(`/projects/${projectId}/files/${encodeURIComponent(name)}`)
        .then(setFile)
        .catch((e) => setErr(e instanceof Error ? e.message : '加载失败'))
    }
  }, [projectId, name, isNew])

  async function save() {
    if (!fileName.trim()) return
    setBusy(true)
    try {
      await api.put(`/projects/${projectId}/files`, { name: fileName.trim(), content })
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
      await api.del(`/projects/${projectId}/files/${encodeURIComponent(name ?? '')}`)
      onChanged()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '删除失败')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card className="flex min-h-0 flex-1 flex-col gap-3 overflow-hidden p-4">
      {editing || isNew ? (
        <>
          <div className="flex shrink-0 items-center gap-2">
            <input
              className={`${inputCls} w-64 font-mono`}
              placeholder="文件名（如 architecture.html）"
              aria-label="文件名"
              value={fileName}
              onChange={(e) => setFileName(e.target.value)}
            />
            <span className="text-xs text-muted-foreground">{mimeHintOf(fileName)}</span>
          </div>
          <textarea
            className={`${inputCls} min-h-[200px] w-full flex-1 resize-none font-mono text-sm leading-5`}
            placeholder="文件内容（HTML / SVG / JSON / 配置…）"
            value={content}
            onChange={(e) => setContent(e.target.value)}
          />
          {err && (
            <div className="shrink-0">
              <ErrorBox msg={err} />
            </div>
          )}
          <div className="flex shrink-0 gap-2">
            <Button size="sm" disabled={busy || !fileName.trim()} onClick={save}>
              {isNew ? '创建' : '保存（version+1）'}
            </Button>
            <Button size="sm" variant="ghost" onClick={() => onChanged()}>
              取消
            </Button>
            {!isNew && (
              <Button size="sm" variant="destructive" disabled={busy} onClick={remove}>
                删除
              </Button>
            )}
          </div>
        </>
      ) : !file ? (
        <>
          {err && <ErrorBox msg={err} />}
          <Spinner />
        </>
      ) : (
        <>
          <div className="flex shrink-0 items-center justify-between">
            <div>
              <h3 className="font-mono text-sm font-semibold">
                {file.name} <span className="text-xs text-muted-foreground">v{file.version}</span>
              </h3>
              <p className="mt-0.5 text-xs text-muted-foreground">
                {file.mime} · {fmtTime(file.updated_at)}
              </p>
            </div>
            <div className="flex gap-2">
              <Button
                size="sm"
                variant="outline"
                onClick={() =>
                  window.open(`/file-view?project=${projectId}&name=${encodeURIComponent(file.name)}`, '_blank')
                }
              >
                全屏打开
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() => {
                  setFileName(file.name)
                  setContent(file.content)
                  setEditing(true)
                }}
              >
                编辑
              </Button>
              <Button size="sm" variant="destructive" disabled={busy} onClick={remove}>
                删除
              </Button>
            </div>
          </div>
          <div className="min-h-0 flex-1 overflow-hidden rounded-md border border-border">
            {file.mime === 'text/html' ? (
              <iframe
                title={`项目文件 ${file.name}`}
                sandbox="allow-scripts"
                srcDoc={file.content}
                className="h-full w-full bg-white"
              />
            ) : file.mime === 'text/markdown' ? (
              <div className="h-full overflow-y-auto p-3 [scrollbar-gutter:stable]">
                <WikiMarkdown content={file.content} />
              </div>
            ) : (
              <pre className="h-full overflow-auto bg-muted/40 p-3 font-mono text-xs leading-5">{file.content}</pre>
            )}
          </div>
        </>
      )}
    </Card>
  )
}

function mimeHintOf(name: string): string {
  const ext = name.toLowerCase().split('.').pop() ?? ''
  const hints: Record<string, string> = {
    html: 'HTML——保存后点开即渲染',
    md: 'Markdown',
    svg: 'SVG（HTML 文件内嵌或单独保存均可）',
    json: 'JSON',
  }
  return hints[ext] ?? '纯文本'
}

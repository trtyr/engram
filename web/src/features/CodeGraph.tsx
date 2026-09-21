/**
 * 代码图谱 · **查看页**（2026-09-21 与导入页拆开）：看已导入的内容。
 * 布局：**正好铺满一屏**（不把页面撑长）——左侧条目列表、右侧详情工作台（状态 / 落盘 / head / 统计 /
 * 操作（按 `source_kind` 分区）/ 查询栏 / **调用图默认展示并自适应剩余高度、可全屏**）。
 * 导入入口在 `/codegraph/import`（顶栏 tab 切换）；从导入页可带 `?sel=<id>` 直达新条目。
 * 错误全程就地可见（列表加载 / 操作 / 查询 / 图加载各自文案）。
 */
import { useEffect, useMemo, useRef, useState } from 'react'
import { Link, useSearchParams } from 'react-router-dom'
import { appConfirm } from '@/components/confirm'
import { ApiError, api, getToken, type CgGraph, type CgProject } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner, StatusBadge } from '@/components/ui-bits'
import { inputCls, selectCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import ForceGraph from '@/components/ForceGraph/ForceGraph'
import CgTabs from '@/components/CgTabs'

/** 上传响应（POST /codegraph/artifacts，multipart）。 */
interface UploadResp {
  project: CgProject
  db_bytes: number
  hint: string
}

/** 调用图的角色配色（codegraph 的角色语义：中心 / 调用方 / 被调 / 文件）。 */
const CG_ROLE_COLOR: Record<string, string> = {
  center: '#e6772e',
  caller: '#3b82f6',
  callee: '#10b981',
  file: '#3b82f6',
}

/** 列表里的状态圆点配色（ready 绿 / 失败红 / 进行中灰）。 */
function dotCls(status: string): string {
  if (status === 'ready') return 'bg-success'
  if (status === 'error' || status === 'version_mismatch') return 'bg-destructive'
  return 'bg-muted-foreground/50'
}

export default function CodeGraph() {
  const [params] = useSearchParams()
  const selFromUrl = params.get('sel')

  const [rows, setRows] = useState<CgProject[] | null>(null)
  const [err, setErr] = useState('')
  const [selectedId, setSelectedId] = useState<string | null>(null)

  const load = () =>
    api
      .get<CgProject[]>('/codegraph/projects')
      .then((r) => {
        setRows(r)
        setErr('')
      })
      .catch((e) => setErr(e.message))

  useEffect(() => {
    load()
  }, [])

  // 从导入页带 ?sel=<id> 进来 → 直接选中新条目
  useEffect(() => {
    if (selFromUrl) setSelectedId(selFromUrl)
  }, [selFromUrl])

  // 选中项跟随列表：首次选第一条；被删后回落到首条
  const selected = useMemo(() => {
    if (!rows || rows.length === 0) return null
    return rows.find((r) => r.id === selectedId) ?? rows[0]
  }, [rows, selectedId])
  useEffect(() => {
    if (selected && selected.id !== selectedId) setSelectedId(selected.id)
  }, [selected, selectedId])

  // 未收尾的条目存在 → 每 2.5s 轮询（registered = 已 clone 等 job 接手；indexing = job 在跑）
  const pending = rows?.some((p) => p.status === 'indexing' || p.status === 'registered') ?? false
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null)
  useEffect(() => {
    if (!pending) {
      if (pollRef.current) clearInterval(pollRef.current)
      pollRef.current = null
      return
    }
    pollRef.current ??= setInterval(load, 2500)
    return () => {
      if (pollRef.current) clearInterval(pollRef.current)
      pollRef.current = null
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pending])

  if (err && !rows) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  return (
    // 铺满一屏：main 有 py-6（上下 1.5rem），所以减 3rem；视口太矮时由外层 main 滚动兜底
    <div className="flex h-[calc(100vh-3rem)] min-h-[520px] flex-col gap-3">
      <PageHeader
        title="代码图谱"
        desc="看已导入的代码库：左侧选条目，右侧看状态、跑查询、看调用图（可全屏）"
      >
        <CgTabs />
      </PageHeader>

      {err && <ErrorBox msg={err} />}
      {pending && (
        <p className="text-xs text-muted-foreground" aria-live="polite">
          有条目在收尾（clone 完成待索引 / 索引执行中）——自动刷新，无需手动操作…
        </p>
      )}

      {rows.length === 0 ? (
        <Card className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6">
          <Empty text="还没有导入任何代码库" />
          <Link to="/codegraph/import" className="text-xs text-info underline underline-offset-2">
            去导入 →
          </Link>
        </Card>
      ) : (
        <div className="grid min-h-0 flex-1 gap-3 md:grid-cols-[240px_minmax(0,1fr)]">
          <ProjectList rows={rows} selectedId={selected?.id ?? null} onSelect={setSelectedId} />
          {selected ? (
            <ProjectDetail p={selected} onChanged={load} />
          ) : (
            <Card className="p-4">
              <Empty text="左侧选一个条目看详情" />
            </Card>
          )}
        </div>
      )}
    </div>
  )
}

/** 条目列表（左栏）：状态点 + 名字 + 来源标签。 */
function ProjectList({
  rows,
  selectedId,
  onSelect,
}: {
  rows: CgProject[]
  selectedId: string | null
  onSelect: (id: string) => void
}) {
  return (
    <Card className="min-h-0 overflow-auto p-1.5">
      <ul className="space-y-0.5">
        {rows.map((p) => {
          const on = p.id === selectedId
          return (
            <li key={p.id}>
              <button
                type="button"
                onClick={() => onSelect(p.id)}
                aria-current={on}
                className={`flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left ${
                  on ? 'bg-muted' : 'hover:bg-muted/50'
                }`}
              >
                <span aria-hidden="true" className={`size-2 shrink-0 rounded-full ${dotCls(p.status)}`} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm">{p.name}</span>
                  <span className="block truncate text-[11px] text-muted-foreground">
                    {p.source_kind === 'client_upload' ? '上传产物' : '服务端索引'} · {p.status}
                  </span>
                </span>
              </button>
            </li>
          )
        })}
      </ul>
    </Card>
  )
}

/**
 * 详情工作台（右栏）：状态 / 落盘 / head / 统计 / 操作（按 source_kind 分区）/ 查询栏 /
 * **调用图（默认展示，占满剩余高度，可全屏）**。
 * 大库不再「手点才拉」——图照常加载，由**共享引擎的 LOD** 自动出简化图并给「渲染全图」逃生门
 * （见《代码图谱入口收敛 · README》图谱引擎一节）。
 */
function ProjectDetail({ p, onChanged }: { p: CgProject; onChanged: () => void }) {
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState('')
  const [kind, setKind] = useState('search')
  const [target, setTarget] = useState('')
  const [out, setOut] = useState('')

  // 图状态：symbol=null 即整库文件依赖全图
  const [symbol, setSymbol] = useState<string | null>(null)
  const [graph, setGraph] = useState<CgGraph | null>(null)
  const [graphErr, setGraphErr] = useState('')
  const [graphBusy, setGraphBusy] = useState(false)
  const [graphTick, setGraphTick] = useState(0)

  const indexing = p.status === 'indexing'
  const isUpload = p.source_kind === 'client_upload'
  const fileRef = useRef<HTMLInputElement | null>(null)
  const ready = p.status === 'ready'

  // 共享图谱引擎入参：CgGraph → 通用 nodes/edges（配色按角色，线宽按依赖次数）
  const cgView = useMemo(() => {
    const nameOf = new Map<string, string>()
    if (!graph) return { nodes: [], edges: [], nameOf }
    const filesMode = graph.mode === 'files'
    const maxW = Math.max(1, ...graph.edges.map((e) => e.weight ?? 1))
    for (const n of graph.nodes) nameOf.set(n.id, n.name)
    return {
      nameOf,
      nodes: graph.nodes.map((n) => ({
        id: n.id,
        label: n.name,
        color: CG_ROLE_COLOR[n.role] ?? '#6b7280',
        x: n.x,
        y: n.y,
      })),
      edges: graph.edges.map((e) => ({
        source: e.from,
        target: e.to,
        weight: e.weight,
        color: e.rel === 'caller' ? '#8b5cf6' : filesMode ? undefined : '#10b981',
        width: filesMode ? 0.6 + ((e.weight ?? 1) / maxW) * 3 : 1,
      })),
    }
  }, [graph])

  // 切换条目：清错误与查询态，回到全图
  useEffect(() => {
    setErr('')
    setOut('')
    setSymbol(null)
  }, [p.id])

  // 拉图：索引就绪才有图；symbol 有值拉符号子图，否则整库全图
  useEffect(() => {
    if (!ready) {
      setGraph(null)
      setGraphErr('')
      return
    }
    let alive = true
    setGraphBusy(true)
    setGraphErr('')
    const q = symbol ? `?symbol=${encodeURIComponent(symbol)}` : ''
    api
      .get<CgGraph>(`/codegraph/projects/${p.id}/graph${q}`)
      .then((g) => {
        if (alive) setGraph(g)
      })
      .catch((e) => {
        if (alive) {
          setGraph(null)
          setGraphErr(e instanceof Error ? e.message : '调用图加载失败')
        }
      })
      .finally(() => {
        if (alive) setGraphBusy(false)
      })
    return () => {
      alive = false
    }
  }, [p.id, ready, symbol, graphTick])

  // 布局落定后再让 sigma 量一次容器（否则可能沿用初次的尺寸）
  useEffect(() => {
    if (!graph) return
    const t = setTimeout(() => window.dispatchEvent(new Event('resize')), 80)
    return () => clearTimeout(t)
  }, [graph])

  const run = async (fn: () => Promise<unknown>) => {
    setBusy(true)
    setErr('')
    try {
      await fn()
      setTimeout(onChanged, 400)
      setGraphTick((t) => t + 1)
    } catch (e) {
      setErr(e instanceof Error ? e.message : '操作失败')
    } finally {
      setBusy(false)
    }
  }

  async function reupload(f: File) {
    await run(async () => {
      await uploadArtifact(p.name, f, '')
      onChanged()
    })
  }

  async function doQuery() {
    if (!target.trim()) return
    try {
      const r = await api.post<Record<string, unknown>>(`/codegraph/projects/${p.id}/query`, {
        kind,
        target,
      })
      setOut(typeof r.text === 'string' ? r.text : JSON.stringify(r, null, 2))
      setErr('')
    } catch (e) {
      setErr(e instanceof Error ? e.message : '查询失败')
      setOut('')
    }
  }

  return (
    <Card className="flex min-h-0 flex-col gap-2 overflow-y-auto p-3">
      {/* 头部：名字 + 来源/状态 */}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="min-w-0 truncate font-medium">{p.name}</h3>
        <div className="flex shrink-0 items-center gap-1.5">
          <span className="rounded border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground">
            {isUpload ? '上传产物' : '服务端索引'}
          </span>
          <StatusBadge status={p.status} />
        </div>
      </div>

      {/* 元信息 */}
      <div className="space-y-0.5 text-[11px] text-muted-foreground">
        <p className="truncate" title={p.source_uri}>
          {p.source_uri}
        </p>
        <p className="truncate font-mono" title={p.path}>
          落盘：{p.path}
          {p.dest_mode === 'default' ? '（默认：删除连目录清）' : '（自定义：删除保留目录）'}
        </p>
        {p.head ? (
          <p className="truncate font-mono">
            head {p.head.slice(0, 10)}
            {p.last_producer ? ` · 投递者 ${p.last_producer}` : ''}
          </p>
        ) : (
          isUpload && <p>未声明 head——新鲜度无法比对</p>
        )}
        {p.freshness?.hint && <p>{p.freshness.hint}</p>}
        {p.stats && (p.stats.files || p.stats.symbols) && (
          <p className="tabular-nums">
            files={p.stats.files ?? '?'} symbols={p.stats.symbols ?? '?'} edges={p.stats.edges ?? '?'}
          </p>
        )}
      </div>
      {p.error && <p className="line-clamp-3 text-xs text-destructive">{p.error}</p>}

      {/* 操作（按 source_kind 分区） */}
      <div className="flex flex-wrap gap-2">
        {isUpload ? (
          <>
            <input
              ref={fileRef}
              type="file"
              accept=".db,application/octet-stream"
              className="hidden"
              onChange={(e) => {
                const f = e.target.files?.[0]
                e.target.value = ''
                if (f) reupload(f)
              }}
            />
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              title="选本机最新产出的 codegraph.db 覆盖本条目（同名覆盖 + 留痕）"
              onClick={() => fileRef.current?.click()}
            >
              {busy ? '上传中…' : '重新上传'}
            </Button>
          </>
        ) : (
          <>
            <Button
              size="sm"
              variant="outline"
              disabled={busy || indexing}
              title="服务端重新跑 codegraph index（异步 job）"
              onClick={() => run(() => api.post(`/codegraph/projects/${p.id}/index`))}
            >
              {indexing
                ? '索引中…'
                : p.status === 'ready'
                  ? '重建索引'
                  : p.status === 'error'
                    ? '重试建索引'
                    : '建索引'}
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={busy || indexing || !ready}
              title={!ready ? '索引就绪后才能同步' : undefined}
              onClick={() => run(() => api.post(`/codegraph/projects/${p.id}/sync`))}
            >
              同步
            </Button>
          </>
        )}
        <Button
          size="sm"
          variant="ghost"
          disabled={busy}
          onClick={async () => {
            if (
              !(await appConfirm({
                title: `删除项目「${p.name}」？`,
                description:
                  p.dest_mode === 'default'
                    ? '删除注册与索引产物，并清理服务端自建目录（clone 下来的代码一并删除）。'
                    : '只删注册与索引产物；落盘目录（可能是你自己的代码目录）保留，需要时请手动清理。',
                destructive: true,
                confirmLabel: '删除',
              }))
            )
              return
            run(() => api.del(`/codegraph/projects/${p.id}`))
          }}
        >
          删除
        </Button>
      </div>
      {err && <p className="text-xs text-destructive">{err}</p>}

      {/* 查询栏 */}
      <div className="flex flex-wrap gap-2">
        <select
          className={selectCls}
          aria-label="查询类型"
          value={kind}
          onChange={(e) => setKind(e.target.value)}
        >
          {['search', 'explore', 'node', 'callers', 'callees', 'impact'].map((k) => (
            <option key={k}>{k}</option>
          ))}
        </select>
        <input
          className={`${inputCls} min-w-0 flex-1`}
          placeholder={ready ? '输入符号名（如 load / getToken）' : '索引就绪后可查询'}
          aria-label="查询符号"
          value={target}
          disabled={!ready}
          onChange={(e) => setTarget(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && ready) {
              e.preventDefault()
              doQuery()
            }
          }}
        />
        <Button
          size="sm"
          variant="ghost"
          disabled={!ready || !target.trim()}
          title={ready ? '按当前查询类型检索（文本结果在图下方）' : '索引就绪后可查询'}
          onClick={doQuery}
        >
          查询
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={!ready || !target.trim()}
          title="用输入的符号渲染调用子图（中心 / 调用方 / 被调三色）"
          onClick={() => setSymbol(target.trim())}
        >
          看这个符号
        </Button>
        {symbol && (
          <Button size="sm" variant="ghost" onClick={() => setSymbol(null)}>
            返回全图
          </Button>
        )}
      </div>

      {/* 调用图：占满剩余高度（默认展示；全屏按钮在图的右上角，三处共用） */}
      <div className="flex min-h-[300px] flex-1 flex-col gap-2">
        <div className="flex shrink-0 items-center justify-between gap-2">
          <p className="min-w-0 truncate text-xs text-muted-foreground">
            {symbol ? `调用子图：${symbol}` : '文件依赖全图'}
            {ready ? '' : '（索引就绪后展示）'}
          </p>
          <div className="flex shrink-0 gap-2">
            <Button
              size="sm"
              variant="ghost"
              disabled={!ready}
              title="重新拉取调用图"
              onClick={() => setGraphTick((t) => t + 1)}
            >
              刷新图
            </Button>
          </div>
        </div>
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
          {!ready ? (
            <div className="flex h-full items-center justify-center rounded-md border border-dashed border-border">
              <Empty text={p.status === 'error' ? '索引失败——先「重试建索引」' : '索引就绪后这里展示调用图'} />
            </div>
          ) : graphErr ? (
            <div className="flex h-full flex-col items-center justify-center gap-2 rounded-md border border-destructive/40 px-4 text-center">
              <p className="text-xs text-destructive">{graphErr}</p>
              <Button size="sm" variant="outline" onClick={() => setGraphTick((t) => t + 1)}>
                重试
              </Button>
            </div>
          ) : graphBusy && !graph ? (
            <Spinner label="加载调用图…" />
          ) : graph ? (
            <ForceGraph
              nodes={cgView.nodes}
              edges={cgView.edges}
              directed={graph.mode !== 'files'}
              persistKey={p.id}
              onPick={(id) => setTarget(cgView.nameOf.get(id) ?? id)}
              onExpand={(id) => setSymbol(cgView.nameOf.get(id) ?? id)}
              legend={
                graph.mode === 'files' ? (
                  <span>节点大小 = 被依赖程度 · 线粗 = 依赖次数 · 拖节点跟手（松手回弹）</span>
                ) : (
                  <>
                    <span className="flex items-center gap-1">
                      <i className="size-2 rounded-full" style={{ background: CG_ROLE_COLOR.center }} aria-hidden="true" />
                      中心 {graph.symbol}
                    </span>
                    <span className="flex items-center gap-1">
                      <i className="size-2 rounded-full" style={{ background: CG_ROLE_COLOR.caller }} aria-hidden="true" />
                      调用方 {graph.callers}
                    </span>
                    <span className="flex items-center gap-1">
                      <i className="size-2 rounded-full" style={{ background: CG_ROLE_COLOR.callee }} aria-hidden="true" />
                      被调 {graph.callees}
                    </span>
                  </>
                )
              }
            />
          ) : (
            <div className="flex h-full items-center justify-center rounded-md border border-dashed border-border">
              <Empty text="这个库里没有可渲染的依赖关系" />
            </div>
          )}
        </div>
      </div>

      {/* 文本查询结果（图负责视觉，正文放这里） */}
      {out && (
        <div className="shrink-0 overflow-hidden rounded-md border border-border">
          <div className="flex items-center justify-between border-b border-border bg-muted/40 px-3 py-1.5">
            <span className="font-mono text-xs text-muted-foreground">
              {p.name} · {kind} · {target || '—'}
            </span>
            <button
              type="button"
              className="text-[11px] text-muted-foreground hover:text-foreground"
              onClick={() => setOut('')}
            >
              清空
            </button>
          </div>
          <pre className="max-h-56 overflow-auto whitespace-pre-wrap bg-card p-3 font-mono text-xs leading-relaxed">
            {out}
          </pre>
        </div>
      )}
    </Card>
  )
}

/** multipart 上传（绕开 JSON 封装）：错误体与 api 同口径解析，失败文案可行动。 */
async function uploadArtifact(name: string, file: File, head: string): Promise<UploadResp> {
  const fd = new FormData()
  fd.append('name', name)
  if (head) fd.append('head', head)
  fd.append('file', file, 'codegraph.db')
  const t = getToken()
  const resp = await fetch('/codegraph/artifacts', {
    method: 'POST',
    headers: t ? { authorization: `Bearer ${t}` } : {},
    body: fd,
  })
  if (!resp.ok) {
    let msg = `HTTP ${resp.status}`
    let code = 'unknown'
    let retryable = false
    try {
      const e = await resp.json()
      if (e?.error) {
        msg = e.error.message ?? msg
        code = e.error.code ?? code
        retryable = e.error.retryable ?? false
      }
    } catch {
      /* 非 JSON 错误体（如 413）：保底用状态码 */
    }
    throw new ApiError(resp.status, code, msg, retryable)
  }
  return (await resp.json()) as UploadResp
}

/**
 * CodeGraph 域：CLI 状态条 + 项目列表（异步索引轮询/删除）+ 调用图视图 + 查询试验场。
 * 索引/同步走平台 job 队列（202 入队）——页面在 indexing 时自动轮询刷新。
 */
import { useEffect, useRef, useState } from 'react'
import {
  api,
  type CgCliStatus,
  type CgGraph,
  type CgProject,
} from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner, StatusBadge } from '@/components/ui-bits'
import { inputCls, selectCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import CgGraphView from '@/components/CgGraphView'

export default function CodeGraph() {
  const [rows, setRows] = useState<CgProject[] | null>(null)
  const [cli, setCli] = useState<CgCliStatus | null>(null)
  const [name, setName] = useState('')
  const [uri, setUri] = useState('')
  const [err, setErr] = useState('')

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
    api
      .get<CgCliStatus>('/codegraph/status')
      .then(setCli)
      .catch(() => setCli({ available: false, version: null, pin: '1.5.0' }))
  }, [])

  // indexing 中的项目存在 → 每 2.5s 轮询刷新（job 完成后列表自动落到 ready/error）
  const indexing = rows?.some((p) => p.status === 'indexing') ?? false
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null)
  useEffect(() => {
    if (!indexing) {
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
  }, [indexing])

  if (err && !rows) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  return (
    <div className="space-y-5">
      <PageHeader title="代码图谱" desc="代码知识图谱：注册 → 建索引（异步）→ 符号查询 / 调用图">
        <input
          className={`${inputCls} w-40`}
          placeholder="项目名"
          aria-label="项目名"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <input
          className={`${inputCls} w-96 max-w-full`}
          placeholder="本地绝对路径或 git URL"
          aria-label="项目路径"
          value={uri}
          onChange={(e) => setUri(e.target.value)}
        />
        <Button
          size="sm"
          variant="outline"
          type="submit"
          onClick={async () => {
            if (!name.trim() || !uri.trim()) return
            try {
              await api.post('/codegraph/projects', { name: name.trim(), source_uri: uri.trim() })
              setName('')
              setUri('')
              load()
            } catch (ex) {
              setErr(ex instanceof Error ? ex.message : '注册失败')
            }
          }}
        >
          注册
        </Button>
      </PageHeader>

      {/* CLI 状态条 */}
      {cli && (
        <Card
          className={`flex flex-wrap items-center gap-3 px-4 py-2.5 ${!cli.available ? 'border-destructive/40' : ''}`}
        >
          <span
            aria-hidden="true"
            className={`size-2 rounded-full ${cli.available ? 'bg-success' : 'bg-destructive'}`}
          />
          <p className="text-xs">
            {cli.available
              ? `codegraph CLI 已就绪 · v${cli.version}（pin 匹配）`
              : 'codegraph CLI 不可用——安装：npm install -g @colbymchenry/codegraph@1.5.0'}
          </p>
          {cli.available && (
            <span className="font-mono text-[11px] text-muted-foreground">pin {cli.pin}</span>
          )}
        </Card>
      )}
      {err && <ErrorBox msg={err} />}
      {indexing && (
        <p className="text-xs text-muted-foreground" aria-live="polite">
          索引执行中（后台任务）——页面自动刷新，无需手动操作…
        </p>
      )}

      {rows.length === 0 ? (
        <Empty text="暂无项目——注册本地路径或 git URL 开始" />
      ) : (
        <div className="grid gap-4 md:grid-cols-2">
          {rows.map((p) => (
            <ProjectCard key={p.id} p={p} onChanged={load} />
          ))}
        </div>
      )}
    </div>
  )
}

function ProjectCard({ p, onChanged }: { p: CgProject; onChanged: () => void }) {
  const [busy, setBusy] = useState(false)
  const indexing = p.status === 'indexing'

  const run = async (fn: () => Promise<unknown>) => {
    setBusy(true)
    try {
      await fn()
      // 异步 job：立即刷新（拿到 queued/indexing 态），轮询器接管后续
      setTimeout(onChanged, 400)
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card className="p-4">
      <div className="flex items-center justify-between gap-2">
        <h3 className="truncate font-medium">{p.name}</h3>
        <StatusBadge status={p.status} />
      </div>
      <p className="mt-1 truncate text-xs text-muted-foreground" title={p.source_uri}>
        {p.source_uri}
      </p>
      {p.stats && (p.stats.files || p.stats.symbols) && (
        <p className="mt-1 text-xs tabular-nums text-muted-foreground">
          files={p.stats.files ?? '?'} symbols={p.stats.symbols ?? '?'} edges={p.stats.edges ?? '?'}
        </p>
      )}
      {p.error && <p className="mt-1 line-clamp-2 text-xs text-destructive">{p.error}</p>}
      <div className="mt-3 flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={busy || indexing}
          onClick={() => run(() => api.post(`/codegraph/projects/${p.id}/index`))}
        >
          {indexing ? '索引中…' : p.status === 'ready' ? '重建索引' : '建索引'}
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={busy || indexing || p.status !== 'ready'}
          title={p.status !== 'ready' ? '索引就绪后才能同步' : undefined}
          onClick={() => run(() => api.post(`/codegraph/projects/${p.id}/sync`))}
        >
          同步
        </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={busy}
          onClick={() => {
            if (!confirm(`删除项目「${p.name}」？索引数据一并清理（git clone 的工作目录会删除，本地路径项目不动源码）。`)) return
            run(() => api.del(`/codegraph/projects/${p.id}`))
          }}
        >
          删除
        </Button>
      </div>
      <QueryPlayground projectId={p.id} projectName={p.name} status={p.status} />
    </Card>
  )
}

/** 查询试验场 + 调用图弹窗入口（空输入=全项目依赖图，输入符号=符号调用子图）。 */
function QueryPlayground({
  projectId,
  projectName,
  status,
}: {
  projectId: string
  projectName: string
  status: string
}) {
  const [kind, setKind] = useState('search')
  const [target, setTarget] = useState('')
  const [out, setOut] = useState('')
  const [graphOpen, setGraphOpen] = useState(false)
  const [err, setErr] = useState('')
  const ready = status === 'ready'

  async function doQuery(k: string, t: string) {
    if (!t.trim()) return
    try {
      const r = await api.post<Record<string, unknown>>(`/codegraph/projects/${projectId}/query`, {
        kind: k,
        target: t,
      })
      setOut(typeof r.text === 'string' ? r.text : JSON.stringify(r, null, 2))
      setErr('')
    } catch (e) {
      setErr(e instanceof Error ? e.message : '查询失败')
      setOut('')
    }
  }

  return (
    <div className="mt-4 border-t border-border/60 pt-3">
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
          placeholder={ready ? '输入符号名，如 load / getToken' : '索引就绪后可查询'}
          aria-label="查询符号"
          value={target}
          disabled={!ready}
          onChange={(e) => setTarget(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && ready) {
              e.preventDefault()
              doQuery(kind, target)
            }
          }}
        />
        <Button
          size="sm"
          variant="ghost"
          disabled={!ready || !target.trim()}
          title={ready ? '按当前查询类型检索' : '索引就绪后可查询'}
          onClick={() => doQuery(kind, target)}
        >
          查询
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={!ready}
          title={
            !ready
              ? '索引就绪后可看调用图'
              : target.trim()
                ? `以 ${target.trim()} 为中心渲染调用方/被调子图`
                : '看整个项目的文件依赖全图；输入符号名则看该符号的调用子图'
          }
          onClick={() => setGraphOpen(true)}
        >
          看调用图
        </Button>
      </div>
      {err && <p className="mt-1.5 text-xs text-destructive">{err}</p>}
      {/* 调用图弹窗：大画布独立层，项目多也不挤 */}
      {graphOpen && (
        <GraphModal
          projectId={projectId}
          projectName={projectName}
          symbol={target.trim() || null}
          onClose={() => setGraphOpen(false)}
        />
      )}
      {out && (
        <div className="mt-2 overflow-hidden rounded-md border border-border">
          <div className="border-b border-border bg-muted/40 px-3 py-1.5 font-mono text-xs text-muted-foreground">
            {projectName} · {kind} · {target || '—'}
          </div>
          <pre className="max-h-72 overflow-auto whitespace-pre-wrap bg-card p-3 font-mono text-xs leading-relaxed">
            {out}
          </pre>
        </div>
      )}
    </div>
  )
}

/** 调用图弹窗：全屏暗层 + 大画布（90vw×85vh）；Esc / 点背景 / 关闭按钮退出。 */
function GraphModal({
  projectId,
  projectName,
  symbol,
  onClose,
}: {
  projectId: string
  projectName: string
  symbol: string | null
  onClose: () => void
}) {
  const [graph, setGraph] = useState<CgGraph | null>(null)
  const [err, setErr] = useState('')

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  useEffect(() => {
    let alive = true
    const q = symbol ? `?symbol=${encodeURIComponent(symbol)}` : ''
    api
      .get<CgGraph>(`/codegraph/projects/${projectId}/graph${q}`)
      .then((g) => {
        if (alive) setGraph(g)
      })
      .catch((e) => {
        if (alive) setErr(e instanceof Error ? e.message : '调用图加载失败')
      })
    return () => {
      alive = false
    }
  }, [projectId, symbol])

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-foreground/40 p-4"
      role="dialog"
      aria-modal="true"
      aria-label={`${projectName} 调用图`}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <Card className="flex h-[85vh] w-[90vw] max-w-6xl flex-col overflow-hidden p-4">
        <div className="mb-3 flex shrink-0 items-center justify-between gap-3">
          <h3 className="min-w-0 truncate text-sm font-semibold">
            {projectName} · {symbol ? `调用子图：${symbol}` : '文件依赖全图'}
          </h3>
          <Button size="sm" variant="outline" onClick={onClose}>
            关闭
          </Button>
        </div>
        <div className="flex min-h-0 flex-1 flex-col">
          {err ? (
            <ErrorBox msg={err} />
          ) : graph === null ? (
            <Spinner label="渲染调用图…" />
          ) : (
            <CgGraphView graph={graph} />
          )}
        </div>
      </Card>
    </div>
  )
}

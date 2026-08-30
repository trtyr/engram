/** CodeGraph 域：项目卡片 + 注册/索引/同步 + 查询试验场。 */
import { useEffect, useState } from 'react'
import { api, type CgProject } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner, StatusBadge } from '@/components/ui-bits'
import { inputCls, selectCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'

export default function CodeGraph() {
  const [rows, setRows] = useState<CgProject[] | null>(null)
  const [name, setName] = useState('')
  const [uri, setUri] = useState('')
  const [err, setErr] = useState('')
  const load = () => api.get<CgProject[]>('/codegraph/projects').then(setRows).catch((e) => setErr(e.message))
  useEffect(() => {
    load()
  }, [])
  if (err && !rows) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  return (
    <div className="space-y-6">
      <PageHeader title="代码图谱" desc="代码知识图谱：注册 → 建索引 → 符号查询" />

      <form
        className="flex flex-wrap gap-2"
        onSubmit={async (e) => {
          e.preventDefault()
          try {
            await api.post('/codegraph/projects', { name, source_uri: uri })
            setName('')
            setUri('')
            load()
          } catch (ex) {
            setErr(ex instanceof Error ? ex.message : '注册失败')
          }
        }}
      >
        <input
          className={`${inputCls} w-40`}
          placeholder="项目名"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <input
          className={`${inputCls} flex-1`}
          placeholder="本地绝对路径或 git URL"
          value={uri}
          onChange={(e) => setUri(e.target.value)}
        />
        <Button size="sm" variant="outline" type="submit">
          注册
        </Button>
      </form>
      {err && <ErrorBox msg={err} />}

      {rows.length === 0 ? (
        <Empty text="暂无项目" />
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
  return (
    <Card className="p-4">
      <div className="flex items-center justify-between">
        <h3 className="font-medium">{p.name}</h3>
        <StatusBadge status={p.status} />
      </div>
      <p className="mt-1 truncate text-xs text-muted-foreground">{p.source_uri}</p>
      {p.stats && (
        <p className="mt-1 text-xs tabular-nums text-muted-foreground">
          files={p.stats.files ?? '?'} symbols={p.stats.symbols ?? '?'} edges={p.stats.edges ?? '?'}
        </p>
      )}
      {p.error && <p className="mt-1 text-xs text-destructive">{p.error}</p>}
      <div className="mt-3 flex gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          onClick={async () => {
            setBusy(true)
            try {
              await api.post(`/codegraph/projects/${p.id}/index`)
              onChanged()
            } finally {
              setBusy(false)
            }
          }}
        >
          {p.status === 'ready' ? '重建索引' : '建索引'}
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          onClick={async () => {
            setBusy(true)
            try {
              await api.post(`/codegraph/projects/${p.id}/sync`)
              onChanged()
            } finally {
              setBusy(false)
            }
          }}
        >
          同步
        </Button>
      </div>
      <QueryPlayground projectId={p.id} />
    </Card>
  )
}

function QueryPlayground({ projectId }: { projectId: string }) {
  const [kind, setKind] = useState('explore')
  const [target, setTarget] = useState('')
  const [out, setOut] = useState('')
  const [err, setErr] = useState('')
  return (
    <div className="mt-4 border-t border-border/60 pt-3">
      <div className="flex gap-2">
        <select className={selectCls} value={kind} onChange={(e) => setKind(e.target.value)}>
          {['explore', 'search', 'callers', 'callees', 'impact'].map((k) => (
            <option key={k}>{k}</option>
          ))}
        </select>
        <input
          className={`${inputCls} flex-1`}
          placeholder="符号或问题"
          value={target}
          onChange={(e) => setTarget(e.target.value)}
        />
        <Button
          size="sm"
          variant="ghost"
          onClick={async () => {
            try {
              const r = await api.post<Record<string, unknown>>(`/codegraph/projects/${projectId}/query`, { kind, target })
              setOut(typeof r.text === 'string' ? r.text : JSON.stringify(r, null, 2))
              setErr('')
            } catch (e) {
              setErr(e instanceof Error ? e.message : '查询失败')
              setOut('')
            }
          }}
        >
          查询
        </Button>
      </div>
      {err && <p className="mt-1.5 text-xs text-destructive">{err}</p>}
      {out && (
        <div className="mt-2 overflow-hidden rounded-md border border-border">
          <div className="border-b border-border bg-muted/40 px-3 py-1.5 font-mono text-xs text-muted-foreground">
            query · {kind} · {target || '—'}
          </div>
          <pre className="max-h-72 overflow-auto whitespace-pre-wrap bg-card p-3 font-mono text-xs leading-relaxed">
            {out}
          </pre>
        </div>
      )}
    </div>
  )
}

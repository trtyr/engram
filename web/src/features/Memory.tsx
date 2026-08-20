/** Memory 域：会话 / 原子 / 场景 / 画像 / 检索。 */
import { useEffect, useState } from 'react'
import {
  api,
  type Atom,
  type Persona,
  type Scenario,
  type Session,
} from '@/lib/api'
import { Empty, ErrorBox, Spinner, StatusBadge, fmtTime } from '@/components/ui-bits'
import { Button } from '@/components/ui/button'

type Tab = 'sessions' | 'atoms' | 'scenarios' | 'persona' | 'search'

export default function Memory() {
  const [tab, setTab] = useState<Tab>('sessions')
  return (
    <div className="space-y-6">
      <h1 className="text-xl font-semibold">Memory</h1>
      <div className="flex gap-2">
        {(['sessions', 'atoms', 'scenarios', 'persona', 'search'] as Tab[]).map((t) => (
          <Button key={t} variant={tab === t ? 'default' : 'outline'} size="sm" onClick={() => setTab(t)}>
            {t}
          </Button>
        ))}
      </div>
      {tab === 'sessions' && <Sessions />}
      {tab === 'atoms' && <Atoms />}
      {tab === 'scenarios' && <Scenarios />}
      {tab === 'persona' && <PersonaView />}
      {tab === 'search' && <SearchPane />}
    </div>
  )
}

function Sessions() {
  const [rows, setRows] = useState<Session[] | null>(null)
  const [open, setOpen] = useState<Session | null>(null)
  const [err, setErr] = useState('')
  const load = () => api.get<Session[]>('/memory/sessions?limit=50').then(setRows).catch((e) => setErr(e.message))
  useEffect(() => {
    load()
  }, [])
  if (err) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  return (
    <div className="space-y-4">
      <div className="flex gap-2">
        <Button
          size="sm"
          onClick={async () => {
            await api.post('/memory/distill', { full: false })
            load()
          }}
        >
          触发蒸馏
        </Button>
      </div>
      {rows.length === 0 ? (
        <Empty text="暂无会话——POST /memory/sessions 写入" />
      ) : (
        <table className="w-full text-sm">
          <thead className="text-left text-muted-foreground">
            <tr className="border-b">
              <th className="py-1.5 pr-4">时间</th>
              <th className="pr-4">Agent</th>
              <th className="pr-4">轮次</th>
              <th className="pr-4">蒸馏</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rows.map((s) => (
              <tr key={s.id} className="border-b">
                <td className="py-1.5 pr-4">{fmtTime(s.created_at)}</td>
                <td className="pr-4">{s.agent}</td>
                <td className="pr-4">{s.content?.length ?? 0}</td>
                <td className="pr-4">
                  <StatusBadge status={s.distill_status} />
                </td>
                <td className="text-right">
                  <Button variant="ghost" size="sm" onClick={() => setOpen(s)}>
                    详情
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {open && (
        <div className="rounded-lg border p-4">
          <div className="mb-2 flex items-center justify-between">
            <p className="text-sm text-muted-foreground">{open.id}</p>
            <Button
              variant="destructive"
              size="sm"
              onClick={async () => {
                await api.del(`/memory/sessions/${open.id}`)
                setOpen(null)
                load()
              }}
            >
              擦除
            </Button>
          </div>
          <div className="space-y-1">
            {open.content?.map((t, i) => (
              <p key={i} className="text-sm">
                <span className="mr-2 text-muted-foreground">{t.speaker}:</span>
                {t.text}
              </p>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}

function Atoms() {
  const [rows, setRows] = useState<Atom[] | null>(null)
  const [kind, setKind] = useState('')
  const [review, setReview] = useState(false)
  const [err, setErr] = useState('')
  const [editing, setEditing] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  const [superseding, setSuperseding] = useState<string | null>(null)
  useEffect(() => {
    const p = new URLSearchParams({ limit: '200' })
    if (kind) p.set('kind', kind)
    if (review) p.set('needs_review', 'true')
    api.get<Atom[]>(`/memory/atoms?${p}`).then(setRows).catch((e) => setErr(e.message))
  }, [kind, review])
  if (err) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  const kinds = ['preference', 'fact', 'decision', 'event', 'insight', 'correction', 'failure', 'convention']
  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <select className="rounded border bg-transparent px-2 py-1 text-sm" value={kind} onChange={(e) => setKind(e.target.value)}>
          <option value="">全部 kind</option>
          {kinds.map((k) => (
            <option key={k}>{k}</option>
          ))}
        </select>
        <label className="flex items-center gap-1 text-sm">
          <input type="checkbox" checked={review} onChange={(e) => setReview(e.target.checked)} />
          仅人审
        </label>
      </div>
      {superseding && (
        <div className="rounded-lg border border-orange-500/40 bg-orange-500/10 p-4" data-testid="supersede-panel">
          <p className="mb-2 text-sm font-medium">supersede：输入取代旧记忆的新事实</p>
          <form
            className="flex gap-2"
            onSubmit={async (e) => {
              e.preventDefault()
              const old = rows?.find((r) => r.id === superseding)
              // 平台语义：新增新事实（人审），旧条手工归档——矛盾仲裁由蒸馏管道自动处理，
              // 这里人工路径提供等价操作（create + archive 一步完成）
              await api.post('/memory/atoms', { kind: old?.kind ?? 'fact', content: draft, confidence: 0.95 })
              await api.patch(`/memory/atoms/${superseding}`, { status: 'archived' })
              setSuperseding(null)
              setDraft('')
              const p = new URLSearchParams({ limit: '200' })
              if (kind) p.set('kind', kind)
              if (review) p.set('needs_review', 'true')
              setRows(await api.get<Atom[]>(`/memory/atoms?${p}`))
            }}
          >
            <input data-testid="supersede-input" className="flex-1 rounded border bg-transparent px-3 py-1.5 text-sm" placeholder="新事实（取代旧条目）" value={draft} onChange={(e) => setDraft(e.target.value)} />
            <Button size="sm" type="submit" data-testid="supersede-submit">取代</Button>
            <Button size="sm" variant="ghost" type="button" onClick={() => setSuperseding(null)}>取消</Button>
          </form>
        </div>
      )}
      {rows.length === 0 ? (
        <Empty text="暂无原子" />
      ) : (
        <table className="w-full text-sm">
          <thead className="text-left text-muted-foreground">
            <tr className="border-b">
              <th className="py-1.5 pr-4">kind</th>
              <th className="pr-4">内容</th>
              <th className="pr-4">置信</th>
              <th className="pr-4">状态</th>
              <th className="pr-4">命中</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rows.map((a) => (
              <tr key={a.id} className="border-b">
                <td className="py-1.5 pr-4">{a.kind}</td>
                <td className="pr-4">
                  {editing === a.id ? (
                    <span className="flex items-center gap-1">
                      <input
                        data-testid={`atom-edit-${a.id}`}
                        className="w-72 rounded border bg-transparent px-2 py-0.5 text-sm"
                        value={draft}
                        onChange={(e) => setDraft(e.target.value)}
                        onKeyDown={async (e) => {
                          if (e.key === 'Enter') {
                            await api.patch(`/memory/atoms/${a.id}`, { content: draft })
                            setEditing(null)
                            const p = new URLSearchParams({ limit: '200' })
                            if (kind) p.set('kind', kind)
                            if (review) p.set('needs_review', 'true')
                            setRows(await api.get<Atom[]>(`/memory/atoms?${p}`))
                          }
                          if (e.key === 'Escape') setEditing(null)
                        }}
                      />
                      <Button variant="ghost" size="sm" onClick={() => setEditing(null)}>取消</Button>
                    </span>
                  ) : (
                    <span
                      data-testid={`atom-content-${a.id}`}
                      onDoubleClick={() => { setEditing(a.id); setDraft(a.content) }}
                      title="双击编辑"
                      className="cursor-text"
                    >
                      {a.needs_review && <span className="mr-1 rounded bg-orange-500/20 px-1 text-xs text-orange-400">人审</span>}
                      {a.content}
                    </span>
                  )}
                </td>
                <td className="pr-4">{a.confidence.toFixed(2)}</td>
                <td className="pr-4">
                  <StatusBadge status={a.status} />
                  {a.superseded_by && <span className="ml-1 text-xs text-muted-foreground">→ {a.superseded_by.slice(0, 8)}</span>}
                </td>
                <td className="pr-4">{a.hit_count}</td>
                <td className="text-right whitespace-nowrap">
                  {a.status === 'active' && (
                    <>
                      <Button variant="ghost" size="sm" className="mr-1" data-testid={`atom-supersede-${a.id}`} onClick={() => setSuperseding(a.id)}>
                        supersede
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        data-testid={`atom-archive-${a.id}`}
                        onClick={async () => {
                          await api.patch(`/memory/atoms/${a.id}`, { status: 'archived' })
                          setRows(rows.filter((r) => r.id !== a.id))
                        }}
                      >
                        归档
                      </Button>
                    </>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  )
}

function Scenarios() {
  const [rows, setRows] = useState<Scenario[] | null>(null)
  useEffect(() => {
    api.get<Scenario[]>('/memory/scenarios').then(setRows).catch(() => {})
  }, [])
  if (!rows) return <Spinner />
  if (rows.length === 0) return <Empty text="暂无场景（蒸馏组织阶段产出）" />
  return (
    <div className="grid gap-4 md:grid-cols-2">
      {rows.map((s) => (
        <div key={s.id} className="rounded-lg border p-4">
          <div className="flex items-center justify-between">
            <h3 className="font-medium">{s.topic}</h3>
            <span className="text-xs text-muted-foreground">v{s.version}</span>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">{s.summary}</p>
        </div>
      ))}
    </div>
  )
}

function PersonaView() {
  const [rows, setRows] = useState<Persona[] | null>(null)
  const [history, setHistory] = useState<{ aspect: string; versions: Persona[] } | null>(null)
  useEffect(() => {
    api.get<Persona[]>('/memory/persona').then(setRows).catch(() => {})
  }, [])
  if (!rows) return <Spinner />
  if (rows.length === 0) return <Empty text="画像为空（蒸馏 persona 阶段产出）" />
  return (
    <div className="space-y-4">
      <div className="grid gap-4 md:grid-cols-2">
        {rows.map((p) => (
          <div key={p.id} className="rounded-lg border p-4">
            <div className="flex items-center justify-between">
              <h3 className="font-medium">{p.aspect}</h3>
              <div className="flex items-center gap-2">
                <span className="text-xs text-muted-foreground">v{p.version}</span>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={async () => {
                    const h = await api.get<Persona[]>(`/memory/persona/history?aspect=${p.aspect}`)
                    setHistory({ aspect: p.aspect, versions: h })
                  }}
                >
                  历史
                </Button>
                {p.version > 1 && (
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={async () => {
                      await api.post('/memory/persona/rollback', { aspect: p.aspect, to_version: p.version - 1 })
                      const np = await api.get<Persona[]>('/memory/persona')
                      setRows(np)
                    }}
                  >
                    回滚 v{p.version - 1}
                  </Button>
                )}
              </div>
            </div>
            <p className="mt-2 whitespace-pre-wrap text-sm">{p.content}</p>
          </div>
        ))}
      </div>
      {history && (
        <div className="rounded-lg border p-4">
          <div className="mb-2 flex items-center justify-between">
            <h3 className="font-medium">{history.aspect} 版本历史</h3>
            <Button variant="ghost" size="sm" onClick={() => setHistory(null)}>
              关闭
            </Button>
          </div>
          {history.versions.map((v) => (
            <div key={v.id} className="mb-2 border-b pb-2 text-sm">
              <span className="mr-2 rounded bg-gray-500/15 px-1 text-xs">v{v.version}</span>
              <span className="text-muted-foreground">{fmtTime(v.created_at)}</span>
              <p className="mt-1">{v.content}</p>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

function SearchPane() {
  const [q, setQ] = useState('')
  const [r, setR] = useState<{ l1: { id: string; snippet: string; score: number }[]; l2: { id: string; title: string | null; snippet: string }[]; l3: Persona[] } | null>(null)
  const [err, setErr] = useState('')
  return (
    <div className="space-y-4">
      <form
        className="flex gap-2"
        onSubmit={async (e) => {
          e.preventDefault()
          try {
            setR(await api.post('/memory/search', { query: q, max_items: 10 }))
          } catch (ex) {
            setErr(ex instanceof Error ? ex.message : '检索失败')
          }
        }}
      >
        <input className="flex-1 rounded-md border bg-transparent px-3 py-2 text-sm" value={q} onChange={(e) => setQ(e.target.value)} placeholder="中文检索记忆…" />
        <Button type="submit">检索</Button>
      </form>
      {err && <ErrorBox msg={err} />}
      {r && (
        <div className="space-y-4">
          <section>
            <h3 className="mb-1 text-sm font-medium">L1 原子（{r.l1.length}）</h3>
            {r.l1.map((h) => (
              <p key={h.id} className="border-b py-1 text-sm">
                {h.snippet} <span className="text-xs text-muted-foreground">({h.score.toFixed(3)})</span>
              </p>
            ))}
          </section>
          <section>
            <h3 className="mb-1 text-sm font-medium">L2 场景（{r.l2.length}）</h3>
            {r.l2.map((h) => (
              <p key={h.id} className="border-b py-1 text-sm">
                {h.title} — {h.snippet}
              </p>
            ))}
          </section>
          <section>
            <h3 className="mb-1 text-sm font-medium">L3 画像（{r.l3.length}）</h3>
            {r.l3.map((p) => (
              <p key={p.id} className="border-b py-1 text-sm">
                [{p.aspect}] {p.content}
              </p>
            ))}
          </section>
        </div>
      )}
    </div>
  )
}

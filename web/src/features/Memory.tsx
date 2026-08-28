/** Memory 域：会话 / 原子 / 场景 / 画像 / 检索。 */
import { useEffect, useState } from 'react'
import { api, type Atom, type Persona, type Scenario, type Session } from '@/lib/api'
import {
  Card,
  Empty,
  ErrorBox,
  PageHeader,
  Spinner,
  StatusBadge,
  Tabs,
} from '@/components/ui-bits'
import { fmtTime, inputCls, selectCls, tableCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'

type Tab = 'sessions' | 'atoms' | 'scenarios' | 'persona' | 'search'

const TABS: { value: Tab; label: string }[] = [
  { value: 'sessions', label: '会话' },
  { value: 'atoms', label: '原子' },
  { value: 'scenarios', label: '场景' },
  { value: 'persona', label: '画像' },
  { value: 'search', label: '检索' },
]

export default function Memory() {
  const [tab, setTab] = useState<Tab>('sessions')
  return (
    <div className="space-y-6">
      <PageHeader title="Memory" desc="会话 → 蒸馏 → 原子 → 场景 → 画像，全程可溯源" />
      <Tabs items={TABS} value={tab} onChange={setTab} />
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
      <div className="flex justify-end">
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
        <Card className="overflow-hidden">
          <table className={tableCls.root}>
            <thead className={tableCls.thead}>
              <tr>
                <th className={tableCls.th}>时间</th>
                <th className={tableCls.th}>Agent</th>
                <th className={tableCls.th}>轮次</th>
                <th className={tableCls.th}>蒸馏</th>
                <th className={tableCls.th} />
              </tr>
            </thead>
            <tbody>
              {rows.map((s) => (
                <tr key={s.id} className={tableCls.row}>
                  <td className={`${tableCls.td} text-muted-foreground`}>{fmtTime(s.created_at)}</td>
                  <td className={tableCls.td}>{s.agent}</td>
                  <td className={`${tableCls.td} tabular-nums`}>{s.content?.length ?? 0}</td>
                  <td className={tableCls.td}>
                    <StatusBadge status={s.distill_status} />
                  </td>
                  <td className={`${tableCls.td} text-right`}>
                    <Button variant="ghost" size="sm" onClick={() => setOpen(s)}>
                      详情
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}
      {open && (
        <Card className="p-4">
          <div className="mb-3 flex items-center justify-between">
            <p className="font-mono text-xs text-muted-foreground">{open.id}</p>
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
          <div className="space-y-2">
            {open.content?.map((t, i) => (
              <div key={i} className="flex gap-2 text-sm">
                <span className="w-16 shrink-0 text-muted-foreground">{t.speaker}:</span>
                <span className="flex-1">{t.text}</span>
              </div>
            ))}
          </div>
        </Card>
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
  const params = () => {
    const p = new URLSearchParams({ limit: '200' })
    if (kind) p.set('kind', kind)
    if (review) p.set('needs_review', 'true')
    return p
  }
  const load = () => {
    api.get<Atom[]>(`/memory/atoms?${params()}`).then(setRows).catch((e) => setErr(e.message))
  }
  useEffect(() => {
    load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind, review])
  // 蒸馏后台进行时轮询（有 pending 会话即可能有新原子；全部处理完则停）
  useEffect(() => {
    const t = setInterval(load, 5000)
    return () => clearInterval(t)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind, review])
  if (err) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  const kinds = ['preference', 'fact', 'decision', 'event', 'insight', 'correction', 'failure', 'convention']

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <select className={selectCls} value={kind} onChange={(e) => setKind(e.target.value)}>
          <option value="">全部 kind</option>
          {kinds.map((k) => (
            <option key={k}>{k}</option>
          ))}
        </select>
        <label className="flex items-center gap-2 text-sm text-muted-foreground">
          <input
            type="checkbox"
            className="size-3.5 accent-[var(--brand-strong)]"
            checked={review}
            onChange={(e) => setReview(e.target.checked)}
          />
          仅人审
        </label>
      </div>

      {superseding && (
        <Card className="border-orange-500/40 bg-orange-500/10 p-4" data-testid="supersede-panel">
          <p className="mb-2 text-sm font-medium">supersede：输入取代旧记忆的新事实</p>
          <form
            className="flex gap-2"
            onSubmit={async (e) => {
              e.preventDefault()
              const old = rows?.find((r) => r.id === superseding)
              await api.post('/memory/atoms', { kind: old?.kind ?? 'fact', content: draft, confidence: 0.95 })
              await api.patch(`/memory/atoms/${superseding}`, { status: 'archived' })
              setSuperseding(null)
              setDraft('')
              setRows(await api.get<Atom[]>(`/memory/atoms?${params()}`))
            }}
          >
            <input
              data-testid="supersede-input"
              className={`${inputCls} flex-1`}
              placeholder="新事实（取代旧条目）"
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
            />
            <Button size="sm" type="submit" data-testid="supersede-submit">
              取代
            </Button>
            <Button size="sm" variant="ghost" type="button" onClick={() => setSuperseding(null)}>
              取消
            </Button>
          </form>
        </Card>
      )}

      {rows.length === 0 ? (
        <Empty text="暂无原子" />
      ) : (
        <Card className="overflow-hidden">
          <table className={tableCls.root}>
            <thead className={tableCls.thead}>
              <tr>
                <th className={tableCls.th}>kind</th>
                <th className={tableCls.th}>内容</th>
                <th className={tableCls.th}>置信</th>
                <th className={tableCls.th}>状态</th>
                <th className={tableCls.th}>命中</th>
                <th className={tableCls.th} />
              </tr>
            </thead>
            <tbody>
              {rows.map((a) => (
                <tr key={a.id} className={tableCls.row}>
                  <td className={`${tableCls.td} text-muted-foreground`}>{a.kind}</td>
                  <td className={tableCls.td}>
                    {editing === a.id ? (
                      <span className="flex items-center gap-1.5">
                        <input
                          data-testid={`atom-edit-${a.id}`}
                          className={`${inputCls} w-72`}
                          value={draft}
                          onChange={(e) => setDraft(e.target.value)}
                          onKeyDown={async (e) => {
                            if (e.key === 'Enter') {
                              await api.patch(`/memory/atoms/${a.id}`, { content: draft })
                              setEditing(null)
                              setRows(await api.get<Atom[]>(`/memory/atoms?${params()}`))
                            }
                            if (e.key === 'Escape') setEditing(null)
                          }}
                        />
                        <Button variant="ghost" size="sm" onClick={() => setEditing(null)}>
                          取消
                        </Button>
                      </span>
                    ) : (
                      <span
                        data-testid={`atom-content-${a.id}`}
                        onDoubleClick={() => {
                          setEditing(a.id)
                          setDraft(a.content)
                        }}
                        title="双击编辑"
                        className="cursor-text"
                      >
                        {a.needs_review && (
                          <span className="mr-1.5 rounded bg-orange-500/20 px-1.5 py-0.5 text-xs text-orange-400">
                            人审
                          </span>
                        )}
                        {a.content}
                      </span>
                    )}
                  </td>
                  <td className={`${tableCls.td} tabular-nums`}>{a.confidence.toFixed(2)}</td>
                  <td className={tableCls.td}>
                    <StatusBadge status={a.status} />
                    {a.superseded_by && (
                      <span className="ml-1.5 font-mono text-xs text-muted-foreground">
                        → {a.superseded_by.slice(0, 8)}
                      </span>
                    )}
                  </td>
                  <td className={`${tableCls.td} tabular-nums`}>{a.hit_count}</td>
                  <td className={`${tableCls.td} whitespace-nowrap text-right`}>
                    {a.status === 'active' && (
                      <>
                        <Button
                          variant="ghost"
                          size="sm"
                          className="mr-1"
                          data-testid={`atom-supersede-${a.id}`}
                          onClick={() => setSuperseding(a.id)}
                        >
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
        </Card>
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
        <Card key={s.id} className="p-4">
          <div className="flex items-center justify-between">
            <h3 className="font-medium">{s.topic}</h3>
            <span className="text-xs text-muted-foreground">v{s.version}</span>
          </div>
          <p className="mt-1.5 text-sm text-muted-foreground">{s.summary}</p>
        </Card>
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
          <Card key={p.id} className="p-4">
            <div className="flex items-center justify-between">
              <h3 className="font-medium">{p.aspect}</h3>
              <div className="flex items-center gap-1.5">
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
            <p className="mt-2 whitespace-pre-wrap text-sm text-muted-foreground">{p.content}</p>
          </Card>
        ))}
      </div>
      {history && (
        <Card className="p-4">
          <div className="mb-3 flex items-center justify-between">
            <h3 className="font-medium">{history.aspect} 版本历史</h3>
            <Button variant="ghost" size="sm" onClick={() => setHistory(null)}>
              关闭
            </Button>
          </div>
          {history.versions.map((v) => (
            <div key={v.id} className="mb-2 border-b border-border/50 pb-2 text-sm last:border-0">
              <span className="mr-2 rounded bg-white/5 px-1.5 py-0.5 text-xs">v{v.version}</span>
              <span className="text-muted-foreground">{fmtTime(v.created_at)}</span>
              <p className="mt-1">{v.content}</p>
            </div>
          ))}
        </Card>
      )}
    </div>
  )
}

function SearchPane() {
  const [q, setQ] = useState('')
  const [archiveMsg, setArchiveMsg] = useState('')
  const [r, setR] = useState<{
    l1: { id: string; snippet: string; score: number }[]
    l2: { id: string; title: string | null; snippet: string }[]
    l3: Persona[]
  } | null>(null)
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
        <input className={`${inputCls} flex-1`} value={q} onChange={(e) => setQ(e.target.value)} placeholder="中文检索记忆…" />
        <Button type="submit">检索</Button>
      </form>
      {err && <ErrorBox msg={err} />}
      {r && (
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-medium">检索结果</h3>
            <Button
              size="sm"
              variant="outline"
              data-testid="archive-query"
              onClick={async () => {
                try {
                  await api.post('/wiki/queries/archive', {
                    title: `检索：${q}`,
                    question: q,
                    answer: r.l1.map((h) => h.snippet).join('\n\n'),
                  })
                  setArchiveMsg('已存档到 wiki 并触发再摄取')
                } catch (ex) {
                  setArchiveMsg(ex instanceof Error ? ex.message : '存档失败')
                }
              }}
            >
              存档到 wiki
            </Button>
          </div>
          {archiveMsg && <p className="text-xs text-muted-foreground">{archiveMsg}</p>}
          <Card className="divide-y divide-border/50 p-1">
            <ResultSection title="L1 原子" count={r.l1.length}>
              {r.l1.map((h) => (
                <p key={h.id} className="py-2 text-sm">
                  {h.snippet} <span className="text-xs tabular-nums text-muted-foreground">({h.score.toFixed(3)})</span>
                </p>
              ))}
            </ResultSection>
            <ResultSection title="L2 场景" count={r.l2.length}>
              {r.l2.map((h) => (
                <p key={h.id} className="py-2 text-sm">
                  <span className="font-medium">{h.title}</span> — {h.snippet}
                </p>
              ))}
            </ResultSection>
            <ResultSection title="L3 画像" count={r.l3.length}>
              {r.l3.map((p) => (
                <p key={p.id} className="py-2 text-sm">
                  <span className="font-medium">[{p.aspect}]</span> {p.content}
                </p>
              ))}
            </ResultSection>
          </Card>
        </div>
      )}
    </div>
  )
}

function ResultSection({
  title,
  count,
  children,
}: {
  title: string
  count: number
  children: React.ReactNode
}) {
  return (
    <section className="px-3 py-2">
      <h3 className="mb-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {title}（{count}）
      </h3>
      {children}
    </section>
  )
}

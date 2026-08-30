/** Memory 域：会话 / 原子 / 场景 / 画像 / 检索。 */
import { Fragment, useEffect, useState } from 'react'
import { api, type Atom, type Job, type Persona, type Scenario, type Session } from '@/lib/api'
import Galaxy from '@/features/Galaxy'
import {
  Card,
  Empty,
  ErrorBox,
  PageHeader,
  Spinner,
  StatusBadge,
  Tabs,
} from '@/components/ui-bits'
import { fmtTime, relTime, inputCls, selectCls, tableCls } from '@/lib/ui'
import { useSystemStatus } from '@/lib/status'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

type Tab = 'galaxy' | 'sessions' | 'atoms' | 'scenarios' | 'persona' | 'search'

const TABS: { value: Tab; label: string }[] = [
  { value: 'galaxy', label: '星系' },
  { value: 'sessions', label: '会话' },
  { value: 'atoms', label: '原子' },
  { value: 'scenarios', label: '场景' },
  { value: 'persona', label: '画像' },
  { value: 'search', label: '检索' },
]

/** 原子 kind 中英对照（蒸馏产出的 8 类记忆形态）。 */
const KIND_LABEL: Record<string, string> = {
  preference: '偏好',
  fact: '事实',
  decision: '决策',
  event: '事件',
  insight: '洞察',
  correction: '修正',
  failure: '教训',
  convention: '惯例',
}

/** 画像分面中英对照（迁移 0005 CHECK 枚举的 7 个分面）。 */
const ASPECT_LABEL: Record<string, string> = {
  identity: '身份',
  preferences: '偏好',
  skills: '技能',
  constraints: '约束',
  communication_style: '沟通风格',
  goals: '目标',
  routines: '例行',
}

/** 蒸馏管线条：L0→L3 层级与计数一屏可见（签名交互——点击层级直达对应 tab）。 */
function PipelineStrip({ onGo }: { onGo: (t: Tab) => void }) {
  const [counts, setCounts] = useState<{ l0: number; pending: number; l1: number; l2: number; l3: number } | null>(null)
  useEffect(() => {
    const safeLen = (a: unknown[]) => a.length
    Promise.all([
      api.get<Session[]>('/memory/sessions?limit=500').catch(() => []),
      api.get<Atom[]>('/memory/atoms?limit=500').catch(() => []),
      api.get<Scenario[]>('/memory/scenarios?limit=500').catch(() => []),
      api.get<Persona[]>('/memory/persona').catch(() => []),
    ]).then(([s, a, sc, p]) => {
      setCounts({
        l0: safeLen(s),
        pending: s.filter((x) => x.distill_status === 'pending' || x.distill_status === 'processing').length,
        l1: a.filter((x) => x.status === 'active' || x.status === 'candidate').length,
        l2: safeLen(sc),
        l3: safeLen(p),
      })
    })
  }, [])
  const stages: { key: string; tag: string; label: string; n?: number; tab: Tab }[] = [
    { key: 'l0', tag: 'L0', label: '会话', n: counts?.l0, tab: 'sessions' },
    { key: 'l1', tag: 'L1', label: '原子', n: counts?.l1, tab: 'atoms' },
    { key: 'l2', tag: 'L2', label: '场景', n: counts?.l2, tab: 'scenarios' },
    { key: 'l3', tag: 'L3', label: '画像', n: counts?.l3, tab: 'persona' },
  ]
  const distilling = (counts?.pending ?? 0) > 0
  return (
    <div
      className="flex flex-wrap items-stretch gap-px overflow-hidden rounded-md border border-border bg-border/60"
      aria-label="蒸馏管线"
    >
      {stages.map((s, i) => (
        <div key={s.key} className="flex items-center gap-px bg-card">
          {i > 0 && (
            <span
              aria-hidden="true"
              className={distilling ? 'engram-pulse px-1 font-mono text-xs text-info' : 'px-1 font-mono text-xs text-muted-foreground/60'}
            >
              →
            </span>
          )}
          <button
            type="button"
            onClick={() => onGo(s.tab)}
            className="flex items-baseline gap-2 px-3.5 py-2.5 transition-colors hover:bg-muted"
            title={`${s.tag} ${s.label}${s.n === undefined ? '' : `：${s.n}`}`}
          >
            <span className="font-mono text-xs text-muted-foreground">{s.tag}</span>
            <span className="text-sm font-medium">{s.label}</span>
            <span className="font-mono text-sm tabular-nums">{s.n ?? '–'}</span>
          </button>
        </div>
      ))}
      {distilling && (
        <span className="flex items-center bg-card px-3 font-mono text-xs text-info">
          <span className="engram-pulse mr-1.5 inline-block size-1.5 rounded-full bg-info" />
          蒸馏中 ×{counts?.pending}
        </span>
      )}
    </div>
  )
}

export default function Memory() {
  const [tab, setTab] = useState<Tab>(() => {
    // 支持 ?tab= 深链（Dashboard 管线主视觉点击穿透）：仅首次挂载读一次
    const t = new URLSearchParams(window.location.search).get('tab')
    const valid: readonly string[] = ['galaxy', 'sessions', 'atoms', 'scenarios', 'persona', 'search']
    return valid.includes(t ?? '') ? (t as Tab) : 'galaxy'
  })
  return (
    <div className="space-y-6">
      <PageHeader title="用户记忆" desc="会话 → 蒸馏 → 原子 → 场景 → 画像，全程可溯源" />
      <PipelineStrip onGo={setTab} />
      <Tabs items={TABS} value={tab} onChange={setTab} />
      {tab === 'galaxy' && (
        <Galaxy onGoPersona={() => setTab('persona')} onGoAtoms={() => setTab('atoms')} />
      )}
      {tab === 'sessions' && <Sessions />}
      {tab === 'atoms' && <Atoms />}
      {tab === 'scenarios' && <Scenarios />}
      {tab === 'persona' && <PersonaView />}
      {tab === 'search' && <SearchPane onGoAtoms={() => setTab('atoms')} />}
    </div>
  )
}

function Sessions() {
  const [rows, setRows] = useState<Session[] | null>(null)
  const [openId, setOpenId] = useState<string | null>(null)
  const [err, setErr] = useState('')
  const [distillBusy, setDistillBusy] = useState(false)
  const [notice, setNotice] = useState('')
  const load = () => api.get<Session[]>('/memory/sessions?limit=50').then(setRows).catch((e) => setErr(e.message))
  useEffect(() => {
    load()
  }, [])
  if (err) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <p className="min-h-5 text-xs text-muted-foreground">
          {notice && <span className={notice.includes('失败') ? 'text-destructive' : 'text-info'}>{notice}</span>}
        </p>
        <Button
          size="sm"
          disabled={distillBusy}
          onClick={async () => {
            setDistillBusy(true)
            setNotice('')
            try {
              // 202 返回入队的 Job[]——空数组 = 没有待蒸馏会话
              const jobs = await api.post<Job[]>('/memory/distill', { full: false })
              setNotice(jobs.length > 0 ? `已入队 ${jobs.length} 个蒸馏任务，产出将陆续出现在 L1` : '没有待蒸馏的会话')
              load()
            } catch (e) {
              setNotice(e instanceof Error ? `触发失败：${e.message}` : '触发失败')
            } finally {
              setDistillBusy(false)
            }
          }}
        >
          {distillBusy ? '提交中…' : '触发蒸馏'}
        </Button>
      </div>

      {rows.length === 0 ? (
        <Empty text="暂无会话——对话通过 API / MCP 写入后在此列出，蒸馏沉淀为 L1 原子" />
      ) : (
        <Card className="overflow-x-auto">
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
                <Fragment key={s.id}>
                  <tr className={tableCls.row}>
                    <td className={`${tableCls.td} text-muted-foreground`}>{fmtTime(s.created_at)}</td>
                    <td className={tableCls.td}>{s.agent}</td>
                    <td className={tableCls.tdMono}>{s.content?.length ?? 0}</td>
                    <td className={tableCls.td}>
                      <StatusBadge status={s.distill_status} />
                    </td>
                    <td className={`${tableCls.td} text-right`}>
                      <Button
                        variant="ghost"
                        size="sm"
                        aria-expanded={openId === s.id}
                        onClick={() => setOpenId(openId === s.id ? null : s.id)}
                      >
                        {openId === s.id ? '收起' : '详情'}
                      </Button>
                    </td>
                  </tr>
                  {/* 手风琴：紧贴该行下方展开逐轮对话，视线不断裂 */}
                  {openId === s.id && (
                    <tr>
                      <td colSpan={5} className="border-b border-border p-0">
                        <div className="bg-muted/30 px-4 py-3">
                          <div className="mb-2.5 flex items-center justify-between">
                            <p className="font-mono text-xs text-muted-foreground">{s.id}</p>
                            <Button
                              variant="destructive"
                              size="sm"
                              onClick={async () => {
                                if (!confirm('擦除该会话？关联原子的溯源将标记为 erased，不可恢复。')) return
                                await api.del(`/memory/sessions/${s.id}`)
                                setOpenId(null)
                                load()
                              }}
                            >
                              擦除
                            </Button>
                          </div>
                          <div className="space-y-1.5">
                            {s.content?.map((t, i) => (
                              <div key={i} className="flex gap-2 text-sm">
                                <span className="w-14 shrink-0 font-mono text-xs leading-5 text-muted-foreground">
                                  {t.speaker}
                                </span>
                                {t.ts && (
                                  <span className="shrink-0 font-mono text-xs leading-5 text-muted-foreground/60">
                                    {fmtTime(t.ts)}
                                  </span>
                                )}
                                <span className="flex-1">{t.text}</span>
                              </div>
                            ))}
                          </div>
                        </div>
                      </td>
                    </tr>
                  )}
                </Fragment>
              ))}
            </tbody>
          </table>
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
  // 蒸馏进行中（系统状态轮询源）才有新原子产出——闲时不轮询，省请求
  const { distilling } = useSystemStatus()
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
  useEffect(() => {
    if (!distilling) return
    const t = setInterval(load, 5000)
    return () => clearInterval(t)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [distilling, kind, review])
  if (err) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  const kinds = ['preference', 'fact', 'decision', 'event', 'insight', 'correction', 'failure', 'convention']

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <select className={selectCls} value={kind} onChange={(e) => setKind(e.target.value)}>
          <option value="">全部 kind</option>
          {kinds.map((k) => (
            <option key={k} value={k}>
              {KIND_LABEL[k] ?? k} {k}
            </option>
          ))}
        </select>
        <label className="flex items-center gap-2 text-sm text-muted-foreground">
          <input
            type="checkbox"
            className="size-3.5 accent-foreground"
            checked={review}
            onChange={(e) => setReview(e.target.checked)}
          />
          仅人审
        </label>
      </div>

      {superseding && (
        <Card className="border-warning/40 bg-warning/10 p-4" data-testid="supersede-panel">
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
        <Empty text="暂无原子——会话蒸馏后在 L1 层沉淀记忆原子" />
      ) : (
        <>
          <Card className="overflow-x-auto">
            <table className={tableCls.root}>
              <thead className={tableCls.thead}>
                <tr>
                  <th className={tableCls.th}>kind</th>
                  <th className={tableCls.th}>内容</th>
                  <th className={tableCls.th}>置信</th>
                  <th className={tableCls.th}>状态</th>
                  <th className={tableCls.th}>溯源</th>
                  <th className={tableCls.th}>命中</th>
                  <th className={tableCls.th} />
                </tr>
              </thead>
              <tbody>
                {rows.map((a) => {
                  const refIds = (a.source_refs ?? [])
                    .map((r) => (r.session_id ? r.session_id.slice(0, 8) + (r.erased ? '（已擦除）' : '') : ''))
                    .filter(Boolean)
                    .join(' · ')
                  return (
                    <tr key={a.id} className={tableCls.row}>
                      <td className={tableCls.td}>
                        {/* 中英对照：中文为主（可读），kind 英文 mono 为数据标识 */}
                        <span className="block text-sm">{KIND_LABEL[a.kind] ?? a.kind}</span>
                        <span className="block font-mono text-xs text-muted-foreground">{a.kind}</span>
                      </td>
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
                            className="-mx-1 cursor-text rounded-sm px-1 transition-colors hover:bg-muted/40"
                          >
                            {a.needs_review && (
                              <span className="mr-1.5 rounded bg-warning/15 px-1.5 py-0.5 font-mono text-xs text-warning">
                                人审
                              </span>
                            )}
                            {a.content}
                          </span>
                        )}
                      </td>
                      <td className={cn(tableCls.tdMono, a.confidence < 0.6 && 'text-warning')} title="置信度（低于 0.60 黄色提示）">
                        {a.confidence.toFixed(2)}
                      </td>
                      <td className={tableCls.td}>
                        <StatusBadge status={a.status} />
                        {a.superseded_by && (
                          <span className="ml-1.5 font-mono text-xs text-muted-foreground">
                            → {a.superseded_by.slice(0, 8)}
                          </span>
                        )}
                      </td>
                      <td className={tableCls.tdMono} title={refIds || '无溯源'}>
                        {a.source_refs?.length ?? 0}
                      </td>
                      <td className={tableCls.tdMono}>{a.hit_count}</td>
                      <td className={`${tableCls.td} whitespace-nowrap text-right`}>
                        {a.status === 'active' && (
                          <>
                            <Button
                              variant="ghost"
                              size="sm"
                              className="mr-1"
                              data-testid={`atom-supersede-${a.id}`}
                              title="用新事实取代该记忆"
                              onClick={() => setSuperseding(a.id)}
                            >
                              取代
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
                  )
                })}
              </tbody>
            </table>
          </Card>
          {rows.length === 200 && (
            <p className="text-xs text-muted-foreground">已显示前 200 条——收窄 kind / 人审筛选，或提高 limit 查看更多</p>
          )}
        </>
      )}
    </div>
  )
}

function Scenarios() {
  const [rows, setRows] = useState<Scenario[] | null>(null)
  const [openId, setOpenId] = useState<string | null>(null)
  useEffect(() => {
    api.get<Scenario[]>('/memory/scenarios').then(setRows).catch(() => {})
  }, [])
  if (!rows) return <Spinner />
  if (rows.length === 0) return <Empty text="暂无场景——蒸馏组织阶段把相关原子聚合成场景" />
  return (
    <div className="grid gap-4 md:grid-cols-2">
      {rows.map((s) => (
        <Card key={s.id} className="p-4">
          <div className="flex items-center justify-between gap-2">
            <button
              type="button"
              className="text-left font-medium transition-colors hover:text-muted-foreground"
              onClick={() => setOpenId(openId === s.id ? null : s.id)}
              aria-expanded={openId === s.id}
              title="点击展开/收起聚合的原子"
            >
              {s.topic}
            </button>
            <span className="shrink-0 font-mono text-xs text-muted-foreground">v{s.version}</span>
          </div>
          <p className="mt-1.5 text-sm text-muted-foreground">{s.summary}</p>
          <p className="mt-3 flex items-center gap-2 font-mono text-xs text-muted-foreground/70">
            <span>{s.atom_refs.length} 原子</span>
            <span aria-hidden="true">·</span>
            <span>{relTime(s.updated_at)}</span>
          </p>
          {/* 展开看场景聚合的原子清单（L2 的溯源面） */}
          {openId === s.id && (
            <div className="mt-3 border-t border-border pt-2.5">
              <p className="mb-1.5 font-mono text-xs text-muted-foreground">atom_refs</p>
              <ul className="space-y-1">
                {s.atom_refs.map((id) => (
                  <li key={id} className="font-mono text-xs text-muted-foreground">
                    {id}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </Card>
      ))}
    </div>
  )
}

function PersonaView() {
  const [rows, setRows] = useState<Persona[] | null>(null)
  // 历史随卡片内展开（per-aspect 按需拉取），不再全局面板
  const [openAspect, setOpenAspect] = useState<string | null>(null)
  const [history, setHistory] = useState<Persona[] | null>(null)
  useEffect(() => {
    api.get<Persona[]>('/memory/persona').then(setRows).catch(() => {})
  }, [])
  if (!rows) return <Spinner />
  if (rows.length === 0) return <Empty text="画像为空——蒸馏 persona 阶段从原子与场景提炼长期画像" />

  const loadHistory = async (aspect: string) => {
    if (openAspect === aspect) {
      setOpenAspect(null)
      setHistory(null)
      return
    }
    setHistory(null)
    setOpenAspect(aspect)
    setHistory(await api.get<Persona[]>(`/memory/persona/history?aspect=${aspect}`))
  }

  return (
    <div className="grid items-start gap-4 md:grid-cols-2">
      {rows.map((p) => (
        <Card key={p.id} className="p-4">
          <div className="flex items-center justify-between gap-2">
            <div>
              <h3 className="font-medium">{ASPECT_LABEL[p.aspect] ?? p.aspect}</h3>
              <p className="font-mono text-xs text-muted-foreground">{p.aspect}</p>
            </div>
            <div className="flex shrink-0 items-center gap-1.5">
              <span className="font-mono text-xs text-muted-foreground">v{p.version}</span>
              <Button
                variant="ghost"
                size="sm"
                aria-expanded={openAspect === p.aspect}
                onClick={() => loadHistory(p.aspect)}
              >
                历史
              </Button>
              {p.version > 1 && (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={async () => {
                    if (!confirm(`回滚 ${ASPECT_LABEL[p.aspect] ?? p.aspect} 到 v${p.version - 1}？当前版本会存入历史。`)) return
                    await api.post('/memory/persona/rollback', { aspect: p.aspect, to_version: p.version - 1 })
                    const np = await api.get<Persona[]>('/memory/persona')
                    setRows(np)
                    // 历史面板保持展开并刷新——回滚效果在版本列表里立刻可见
                    if (openAspect === p.aspect) {
                      setHistory(await api.get<Persona[]>(`/memory/persona/history?aspect=${p.aspect}`))
                    }
                  }}
                >
                  回滚 v{p.version - 1}
                </Button>
              )}
            </div>
          </div>
          <p className="mt-2 whitespace-pre-wrap text-sm text-muted-foreground">{p.content}</p>
          <p className="mt-3 font-mono text-xs text-muted-foreground/70">{relTime(p.created_at)}</p>
          {openAspect === p.aspect && (
            <div className="mt-3 border-t border-border pt-2.5">
              {history === null ? (
                <p className="font-mono text-xs text-muted-foreground">加载历史…</p>
              ) : (
                history.map((v) => (
                  <div key={v.id} className="mb-2.5 border-b border-border/50 pb-2.5 text-sm last:mb-0 last:border-0 last:pb-0">
                    <span className="mr-2 rounded border border-border px-1.5 py-px font-mono text-xs text-muted-foreground">
                      v{v.version}
                    </span>
                    <span className="text-xs text-muted-foreground">{fmtTime(v.created_at)}</span>
                    <p className="mt-1">{v.content}</p>
                  </div>
                ))
              )}
            </div>
          )}
        </Card>
      ))}
    </div>
  )
}

function SearchPane({ onGoAtoms }: { onGoAtoms: () => void }) {
  const [q, setQ] = useState('')
  const [busy, setBusy] = useState(false)
  const [archiveMsg, setArchiveMsg] = useState('')
  const [r, setR] = useState<{
    l1: { id: string; snippet: string; score: number }[]
    l2: { id: string; title: string | null; snippet: string }[]
    l3: Persona[]
  } | null>(null)
  const [err, setErr] = useState('')
  const empty = r !== null && r.l1.length + r.l2.length + r.l3.length === 0
  return (
    <div className="space-y-4">
      <form
        className="flex gap-2"
        onSubmit={async (e) => {
          e.preventDefault()
          if (busy || !q.trim()) return
          setBusy(true)
          setErr('')
          setArchiveMsg('')
          try {
            setR(await api.post('/memory/search', { query: q, max_items: 10 }))
          } catch (ex) {
            setErr(ex instanceof Error ? ex.message : '检索失败')
          } finally {
            setBusy(false)
          }
        }}
      >
        <input className={`${inputCls} flex-1`} value={q} onChange={(e) => setQ(e.target.value)} placeholder="中文检索记忆…" />
        <Button type="submit" disabled={busy}>
          {busy ? '检索中…' : '检索'}
        </Button>
      </form>
      {err && <ErrorBox msg={err} />}
      {empty && <Empty text="无匹配记忆——换个说法或先在会话/知识里积累素材" />}
      {r && !empty && (
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
          {archiveMsg && (
            <p className={cn('text-xs', archiveMsg.includes('失败') ? 'text-destructive' : 'text-success')}>{archiveMsg}</p>
          )}
          <Card className="divide-y divide-border/50 p-1">
            <ResultSection title="L1 原子" count={r.l1.length}>
              {r.l1.map((h) => (
                <p key={h.id} className="flex items-start gap-2 py-2 text-sm">
                  <span className="flex-1">
                    {h.snippet}
                    <span className="ml-2 font-mono text-xs text-muted-foreground">{h.score.toFixed(3)}</span>
                  </span>
                  <button
                    type="button"
                    className="shrink-0 text-xs text-muted-foreground underline underline-offset-4 transition-colors hover:text-foreground"
                    onClick={onGoAtoms}
                  >
                    查看
                  </button>
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

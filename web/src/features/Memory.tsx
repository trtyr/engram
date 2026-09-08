/** Memory 域：会话 / 原子 / 场景 / 画像 / 检索。 */
import { Fragment, useEffect, useState } from 'react'
import { Navigate, useLocation, useNavigate } from 'react-router-dom'
import { api, type Atom, type Job, type Persona, type Scenario, type Session } from '@/lib/api'
import {
  Card,
  Checkbox,
  Empty,
  ErrorBox,
  PageHeader,
  Spinner,
  StatusBadge,
  Tabs,
} from '@/components/ui-bits'
import { PersonaHistoryDrawer } from '@/components/PersonaHistory'
import { fmtTime, relTime, inputCls, selectCls, tableCls } from '@/lib/ui'
import { useSystemStatus } from '@/lib/status'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

type Tab = 'sessions' | 'atoms' | 'review' | 'scenarios' | 'persona' | 'search'

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
/** 蒸馏管线条：已退役——计数融进 tab 标签（Tabs 的 count/pulse），省一整行 chrome。 */

/** 路由入口：按 search 键重挂载——palette/概览带 ?tab=&entity= 深链进来时重新读参。 */
export default function MemoryRoute() {
  const search = useLocation().search
  // 旧深链兼容：圈子 2026-09-01 拆独立页（/circle），galaxy tab 不复存在
  if (new URLSearchParams(search).get('tab') === 'galaxy')
    return <Navigate to="/circle" replace />
  return <MemoryPage key={search} />
}

function MemoryPage() {
  const navigate = useNavigate()
  const [tab, setTab] = useState<Tab>(() => {
    // 支持 ?tab= 深链（Dashboard 管线主视觉点击穿透 / palette 实体直达）：仅首次挂载读一次
    const t = new URLSearchParams(window.location.search).get('tab')
    const valid: readonly string[] = ['sessions', 'atoms', 'review', 'scenarios', 'persona', 'search']
    return valid.includes(t ?? '') ? (t as Tab) : 'sessions'
  })
  // tab 计数（原管线条带的职责）：挂载时取一次；蒸馏脉冲只表「正在炼」（processing）
  const [counts, setCounts] = useState<{ l0: number; l1: number; l2: number; l3: number; review: number } | null>(null)
  useEffect(() => {
    Promise.all([
      api.get<Session[]>('/memory/sessions?limit=500').catch(() => []),
      api.get<Atom[]>('/memory/atoms?limit=500').catch(() => []),
      api.get<Scenario[]>('/memory/scenarios?limit=500').catch(() => []),
      api.get<Persona[]>('/memory/persona').catch(() => []),
    ]).then(([s, a, sc, p]) => {
      setCounts({
        l0: s.length,
        l1: a.filter((x) => x.status === 'active' || x.status === 'candidate').length,
        l2: sc.length,
        l3: p.length,
        review: a.filter((x) => x.needs_review).length,
      })
    })
  }, [])
  const { distilling } = useSystemStatus()

  const tabs = [
    { value: 'sessions' as Tab, label: '会话', count: counts?.l0, pulse: distilling > 0 },
    { value: 'atoms' as Tab, label: '原子', count: counts?.l1 },
    { value: 'review' as Tab, label: '人审', count: counts?.review },
    { value: 'scenarios' as Tab, label: '场景', count: counts?.l2 },
    { value: 'persona' as Tab, label: '画像', count: counts?.l3 },
  ]

  return (
    <div className="space-y-6">
      <PageHeader title="用户记忆" desc="会话 → 蒸馏 → 原子 → 场景 → 画像，全程可溯源" />
      <Tabs items={tabs} value={tab} onChange={setTab} />
      {tab === 'sessions' && <Sessions />}
      {tab === 'atoms' && <Atoms />}

      {tab === 'review' && <ReviewQueue onGoAtoms={() => setTab('atoms')} />}
      {tab === 'scenarios' && <Scenarios />}
      {tab === 'persona' && <PersonaView onGoScenario={() => setTab('scenarios')} />}
      {tab === 'search' && (
        <SearchPane
          onGoAtoms={() => setTab('atoms')}
          onGoEntity={(id) => navigate(`/circle?entity=${id}`)}
        />
      )}
    </div>
  )
}

function Sessions() {
  const [rows, setRows] = useState<Session[] | null>(null)
  const [openId, setOpenId] = useState<string | null>(null)
  const [err, setErr] = useState('')
  const [distillBusy, setDistillBusy] = useState(false)
  const [bulkBusy, setBulkBusy] = useState(false)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [notice, setNotice] = useState('')
  const load = () => api.get<Session[]>('/memory/sessions?limit=50').then(setRows).catch((e) => setErr(e.message))
  useEffect(() => {
    load()
  }, [])
  if (err) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  // 积压（pending 未蒸馏）是存量不是进行时——灰字静默，不全局闪
  const backlog = rows.filter((s) => s.distill_status === 'pending').length
  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  const runBulk = async (path: string, confirmMsg: string | null) => {
    if (confirmMsg && !confirm(confirmMsg)) return
    setBulkBusy(true)
    setNotice('')
    try {
      const r = await api.post<{ succeeded: number; failed: { id: string; error: string }[] }>(
        path,
        { ids: [...selected] },
      )
      setNotice(
        `完成 ${r.succeeded} 条${r.failed.length > 0 ? `，失败 ${r.failed.length} 条（多为状态不符）` : ''}`,
      )
      load()
      setSelected(new Set())
    } catch (e) {
      setNotice(e instanceof Error ? `操作失败：${e.message}` : '操作失败')
    } finally {
      setBulkBusy(false)
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <p className="min-h-5 text-xs text-muted-foreground">
          {notice ? (
            <span className={notice.includes('失败') ? 'text-destructive' : 'text-info'}>{notice}</span>
          ) : (
            backlog > 0 && <span>{backlog} 条未蒸馏</span>
          )}
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

      {/* 批量操作条：勾选即现——「全选已作废」直击测试残留清场场景 */}
      {selected.size > 0 && (
        <div className="flex flex-wrap items-center gap-2 rounded-lg border border-border bg-muted/30 px-3 py-2">
          <span className="text-xs text-muted-foreground">已选 {selected.size} 条</span>
          <Button size="sm" variant="ghost" onClick={() => setSelected(new Set(rows.filter((s) => s.distill_status === 'void').map((s) => s.id)))}>
            全选已作废
          </Button>
          <Button
            size="sm"
            disabled={bulkBusy}
            onClick={() => runBulk('/memory/sessions/batch-restore', null)}
          >
            {bulkBusy ? '处理中…' : '恢复所选'}
          </Button>
          <Button
            size="sm"
            variant="destructive"
            disabled={bulkBusy}
            onClick={() =>
              runBulk(
                '/memory/sessions/batch-erase',
                `擦除所选 ${selected.size} 条会话？物理删除（含蒸馏产物级联），不可恢复。`,
              )
            }
          >
            {bulkBusy ? '处理中…' : '擦除所选'}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setSelected(new Set())}>
            取消选择
          </Button>
        </div>
      )}

      {rows.length === 0 ? (
        <Empty text="暂无会话——对话通过 API / MCP 写入后在此列出，蒸馏沉淀为 L1 原子" />
      ) : (
        <Card className="overflow-x-auto">
          <table className={tableCls.root}>
            <thead className={tableCls.thead}>
              <tr>
                <th className={`${tableCls.th} w-10`}>
                  <Checkbox
                    checked={selected.size === rows.length && rows.length > 0}
                    onChange={(checked) =>
                      setSelected(checked ? new Set(rows.map((s) => s.id)) : new Set())
                    }
                    label="全选本页"
                  />
                </th>
                <th className={tableCls.th}>预览</th>
                <th className={tableCls.th}>Agent</th>
                <th className={tableCls.th}>轮次</th>
                <th className={tableCls.th}>蒸馏</th>
                <th className={tableCls.th}>时间</th>
                <th className={tableCls.th} />
              </tr>
            </thead>
            <tbody>
              {rows.map((s) => (
                <Fragment key={s.id}>
                  <tr className={tableCls.row}>
                    <td className={`${tableCls.td} w-10`}>
                      <Checkbox
                        checked={selected.has(s.id)}
                        onChange={() => toggle(s.id)}
                        label="选择该会话"
                      />
                    </td>
                    <td className={`${tableCls.td} max-w-96 truncate font-medium`} title={s.content?.[0]?.text ?? ''}>
                      {s.content?.[0]?.text ?? '（空会话）'}
                    </td>
                    <td className={`${tableCls.tdMono}`}>{s.agent}</td>
                    <td className={tableCls.tdMono}>{s.content?.length ?? 0}</td>
                    <td className={tableCls.td}>
                      <StatusBadge status={s.distill_status} />
                    </td>
                    <td className={`${tableCls.td} whitespace-nowrap text-muted-foreground`}>{fmtTime(s.created_at)}</td>
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
                      <td colSpan={7} className="border-b border-border p-0">
                        <div className="bg-muted/30 px-4 py-3">
                          <div className="mb-2.5 flex items-center justify-between">
                            <p className="font-mono text-xs text-muted-foreground">{s.id}</p>
                            <div className="flex items-center gap-2">
                              {s.distill_status === 'void' && (
                                <Button
                                  variant="outline"
                                  size="sm"
                                  onClick={async () => {
                                    // 撤销作废：会话回作废前状态，被级联归档的原子一并恢复（非破坏性，无需确认）
                                    await api.post(`/memory/sessions/${s.id}/restore`)
                                    setOpenId(null)
                                    load()
                                  }}
                                >
                                  恢复
                                </Button>
                              )}
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
  const [historyAtom, setHistoryAtom] = useState<Atom | null>(null)
  // 重嵌修复：缺失向量可见 + 一键补嵌（换 embedding 供应商后的修复路径）
  const [missing, setMissing] = useState<{ atoms_missing: number; scenarios_missing: number } | null>(null)
  const [reembedMsg, setReembedMsg] = useState('')
  // 蒸馏进行中（系统状态轮询源）才有新原子产出——闲时不轮询，省请求
  const { distilling } = useSystemStatus()
  useEffect(() => {
    api
      .get<{ atoms_missing: number; scenarios_missing: number }>('/memory/embeddings/status')
      .then(setMissing)
      .catch(() => {})
  }, [])
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
      {missing && missing.atoms_missing + missing.scenarios_missing > 0 && (
        <div className="flex flex-wrap items-center gap-2 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2">
          <span className="text-xs text-warning">
            {missing.atoms_missing} 条原子 / {missing.scenarios_missing} 条场景缺向量——混合检索对它们退化为纯 FTS
          </span>
          <Button
            size="sm"
            variant="outline"
            onClick={async () => {
              setReembedMsg('')
              try {
                await api.post('/memory/reembed')
                setReembedMsg('重嵌任务已入队，完成后向量通道自动恢复')
              } catch (ex) {
                setReembedMsg(ex instanceof Error ? ex.message : '入队失败')
              }
            }}
          >
            重嵌缺失向量
          </Button>
          {reembedMsg && <span className="text-xs text-muted-foreground">{reembedMsg}</span>}
        </div>
      )}
      <div className="flex items-center gap-3">
        <select className={selectCls} value={kind} onChange={(e) => setKind(e.target.value)}>
          <option value="">全部 kind</option>
          {kinds.map((k) => (
            <option key={k} value={k}>
              {KIND_LABEL[k] ?? k} {k}
            </option>
          ))}
        </select>
        <Checkbox checked={review} onChange={setReview}>
          仅人审
        </Checkbox>
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
                        <Button
                          variant="ghost"
                          size="sm"
                          className={cn('mr-1', a.sensitive && 'text-warning')}
                          title={a.sensitive ? '敏感原子（检索/快照隐身，点击取消）' : '标记敏感（医疗/感情/财务等，检索与快照隐身）'}
                          onClick={async () => {
                            await api.patch(`/memory/atoms/${a.id}`, { sensitive: !a.sensitive })
                            setRows(
                              rows.map((r) => (r.id === a.id ? { ...r, sensitive: !a.sensitive } : r)),
                            )
                          }}
                        >
                          {a.sensitive ? '已敏感' : '敏感'}
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          className="mr-1"
                          title="改写留痕历史"
                          onClick={() => setHistoryAtom(a)}
                        >
                          历史
                        </Button>
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
      {historyAtom && (
        <AtomHistoryDrawer atom={historyAtom} onClose={() => setHistoryAtom(null)} />
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

function PersonaView({ onGoScenario }: { onGoScenario: () => void }) {
  const [rows, setRows] = useState<Persona[] | null>(null)
  // 历史走右侧抽屉（2026-08-31 P1.1）：内联展开会把同行等高卡一起拉爆
  const [drawerAspect, setDrawerAspect] = useState<string | null>(null)
  const [editingAspect, setEditingAspect] = useState<string | null>(null)
  const [aspectDraft, setAspectDraft] = useState('')
  const refresh = () => api.get<Persona[]>('/memory/persona').then(setRows).catch(() => {})
  useEffect(() => {
    refresh()
  }, [])
  if (!rows) return <Spinner />
  // F3：空内容分面不展示（素材清空后分面退休为空版本——快照与源同生共死）
  const visible = rows.filter((p) => p.content.trim() !== '')
  if (visible.length === 0)
    return <Empty text="画像为空——蒸馏 persona 阶段从原子与场景提炼长期画像" />

  return (
    <>
    {/* 同行等高（去掉 items-start——那让每张卡各自为高，参差）；Card 需 flex 撑满 */}
    <div className="grid gap-4 md:grid-cols-2">
      {visible.map((p) => (
        <Card key={p.id} className="flex flex-col p-4">
          <div className="flex items-center justify-between gap-2">
            <div>
              <h3 className="font-medium">{ASPECT_LABEL[p.aspect] ?? p.aspect}</h3>
              <p className="font-mono text-xs text-muted-foreground">
                {p.aspect} · v{p.version} · {relTime(p.created_at)}
              </p>
            </div>
            <div className="flex shrink-0 items-center gap-1.5">
              {p.manually_edited && (
                <span
                  className="rounded bg-success/15 px-1.5 py-0.5 font-mono text-xs text-success"
                  title="用户钉住：蒸馏绕开此分面（清退仍优先）"
                >
                  已钉住
                </span>
              )}
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  setEditingAspect(p.aspect)
                  setAspectDraft(p.content)
                }}
              >
                编辑
              </Button>
              <Button
                variant="ghost"
                size="sm"
                title={p.manually_edited ? '解除钉住：回归蒸馏管辖' : '钉住：蒸馏不覆盖此分面'}
                onClick={async () => {
                  await api.patch('/memory/persona', { aspect: p.aspect, pinned: !p.manually_edited })
                  await refresh()
                }}
              >
                {p.manually_edited ? '解锁' : '钉住'}
              </Button>
              <Button variant="ghost" size="sm" onClick={() => setDrawerAspect(p.aspect)}>
                历史
              </Button>
            </div>
          </div>
          {editingAspect === p.aspect ? (
            <div className="mt-2 space-y-2">
              <textarea
                aria-label="分面内容"
                className={`${inputCls} min-h-32 w-full`}
                value={aspectDraft}
                onChange={(e) => setAspectDraft(e.target.value)}
              />
              <div className="flex gap-1.5">
                <Button
                  size="sm"
                  onClick={async () => {
                    await api.patch('/memory/persona', { aspect: p.aspect, content: aspectDraft })
                    setEditingAspect(null)
                    await refresh()
                  }}
                >
                  保存（钉住）
                </Button>
                <Button variant="ghost" size="sm" onClick={() => setEditingAspect(null)}>
                  取消
                </Button>
              </div>
            </div>
          ) : (
            <p className="mt-2 flex-1 whitespace-pre-wrap text-sm text-muted-foreground">{p.content}</p>
          )}
          <p className="mt-3 font-mono text-xs text-muted-foreground/70">
            {(() => {
              const ev = (p.evidence_refs && typeof p.evidence_refs === 'object' ? p.evidence_refs : {}) as {
                atoms?: string[]
                sessions?: string[]
                scenarios?: string[]
              }
              return `证据 ${ev.atoms?.length ?? 0} 原子 · ${ev.sessions?.length ?? 0} 会话 · ${ev.scenarios?.length ?? 0} 场景`
            })()}
          </p>
        </Card>
      ))}
    </div>
    {/* 抽屉在网格容器外——fixed 元素不该做 grid 子项 */}
    {drawerAspect && (
      <PersonaHistoryDrawer
        key={drawerAspect}
        aspect={drawerAspect}
        label={ASPECT_LABEL[drawerAspect] ?? drawerAspect}
        onClose={() => setDrawerAspect(null)}
        onGoScenario={onGoScenario}
        onMutated={refresh}
      />
    )}
    </>
  )
}

interface EntityHit {
  id: string
  title: string | null
  snippet: string
  score: number
  kind: string | null
}

/** 人审队列：低置信原子（needs_review）集中审核——通过 / 取代 / 丢弃，支持批量。 */
function ReviewQueue({ onGoAtoms }: { onGoAtoms: () => void }) {
  const [rows, setRows] = useState<Atom[] | null>(null)
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [superseding, setSuperseding] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  const [err, setErr] = useState('')

  const load = () =>
    api
      .get<Atom[]>('/memory/atoms?needs_review=true&limit=200')
      .then(setRows)
      .catch((e) => setErr(e instanceof Error ? e.message : String(e)))
  useEffect(() => {
    load()
  }, [])

  const act = async (id: string, patch: Record<string, unknown>) => {
    await api.patch(`/memory/atoms/${id}`, patch)
    setRows((r) => (r ? r.filter((a) => a.id !== id) : r))
    setPicked((s) => {
      const n = new Set(s)
      n.delete(id)
      return n
    })
  }

  const bulk = async (patch: Record<string, unknown>) => {
    await Promise.all([...picked].map((id) => api.patch(`/memory/atoms/${id}`, patch)))
    setRows((r) => (r ? r.filter((a) => !picked.has(a.id)) : r))
    setPicked(new Set())
  }

  if (err) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  const target = rows.find((r) => r.id === superseding)

  return (
    <div className="space-y-4">
      {rows.length === 0 ? (
        <Empty text="没有待审原子——蒸馏低置信产出（confidence < 0.55）会进这里等人判定" />
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-mono text-xs text-muted-foreground">{rows.length} 待审</span>
            {picked.size > 0 && (
              <>
                <span className="text-xs text-muted-foreground">已选 {picked.size}</span>
                <Button size="sm" variant="outline" onClick={() => bulk({ needs_review: false })}>
                  批量通过
                </Button>
                <Button size="sm" variant="ghost" className="hover:bg-destructive/10 hover:text-destructive" onClick={() => bulk({ status: 'archived' })}>
                  批量丢弃
                </Button>
              </>
            )}
            <Button size="sm" variant="ghost" className="ml-auto" onClick={onGoAtoms}>
              去原子表 →
            </Button>
          </div>

          {superseding && target && (
            <Card className="border-warning/40 bg-warning/10 p-4" data-testid="review-supersede-panel">
              <p className="mb-2 text-sm font-medium">取代「{target.content.slice(0, 24)}…」：输入新事实</p>
              <form
                className="flex gap-2"
                onSubmit={async (e) => {
                  e.preventDefault()
                  await api.post('/memory/atoms', { kind: target.kind, content: draft, confidence: 0.95 })
                  await api.patch(`/memory/atoms/${superseding}`, { status: 'archived' })
                  setRows((r) => (r ? r.filter((a) => a.id !== superseding) : r))
                  setSuperseding(null)
                  setDraft('')
                }}
              >
                <input
                  className={`${inputCls} flex-1`}
                  placeholder="新事实（取代旧条目）"
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                />
                <Button size="sm" type="submit">
                  取代
                </Button>
                <Button size="sm" variant="ghost" type="button" onClick={() => setSuperseding(null)}>
                  取消
                </Button>
              </form>
            </Card>
          )}

          <Card className="divide-y divide-border/60">
            {rows.map((a) => (
              <div key={a.id} className="flex items-start gap-3 px-4 py-3">
                <Checkbox
                  className="mt-1"
                  label={`选中 ${a.content.slice(0, 12)}`}
                  checked={picked.has(a.id)}
                  onChange={(v) =>
                    setPicked((s) => {
                      const n = new Set(s)
                      if (v) n.add(a.id)
                      else n.delete(a.id)
                      return n
                    })
                  }
                />
                <div className="min-w-0 flex-1">
                  <p className="text-sm">{a.content}</p>
                  <p className="mt-0.5 font-mono text-xs text-muted-foreground">
                    {KIND_LABEL[a.kind] ?? a.kind} · 置信 {a.confidence.toFixed(2)} · {relTime(a.created_at)}
                  </p>
                </div>
                <div className="flex shrink-0 items-center gap-1">
                  <Button variant="ghost" size="sm" onClick={() => act(a.id, { needs_review: false })}>
                    通过
                  </Button>
                  <Button variant="ghost" size="sm" onClick={() => { setSuperseding(a.id); setDraft('') }}>
                    取代
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="hover:bg-destructive/10 hover:text-destructive"
                    onClick={() => act(a.id, { status: 'archived' })}
                  >
                    丢弃
                  </Button>
                </div>
              </div>
            ))}
          </Card>
        </>
      )}
    </div>
  )
}

function SearchPane({
  onGoAtoms,
  onGoEntity,
}: {
  onGoAtoms: () => void
  onGoEntity: (id: string) => void
}) {
  const [q, setQ] = useState('')
  const [busy, setBusy] = useState(false)
  const [archiveMsg, setArchiveMsg] = useState('')
  const [r, setR] = useState<{
    entities: EntityHit[]
    l1: { id: string; snippet: string; score: number }[]
    l2: { id: string; title: string | null; snippet: string }[]
    l3: Persona[]
  } | null>(null)
  const [err, setErr] = useState('')
  const empty = r !== null && r.entities.length + r.l1.length + r.l2.length + r.l3.length === 0
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
            {r.entities.length > 0 && (
              <ResultSection title="实体" count={r.entities.length}>
                {r.entities.map((e) => (
                  <button
                    key={e.id}
                    type="button"
                    className="flex w-full items-center gap-2 py-2 text-left text-sm transition-colors hover:text-foreground"
                    onClick={() => onGoEntity(e.id)}
                  >
                    <span className="font-medium">{e.title}</span>
                    {e.kind && (
                      <span className="rounded border border-border px-1.5 py-px font-mono text-xs text-muted-foreground">
                        {e.kind}
                      </span>
                    )}
                    <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">{e.snippet}</span>
                    <span className="shrink-0 font-mono text-xs text-muted-foreground">{e.score.toFixed(1)}</span>
                  </button>
                ))}
              </ResultSection>
            )}
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

/** AtomRevision（GET /memory/atoms/{id}/revisions） */
interface AtomRevision {
  id: string
  atom_id: string
  old_content: string
  old_kind: string
  old_confidence: number
  edited_by: string
  created_at: string
}

/** 原子改写历史抽屉（编辑能力：留痕可溯——复用 persona 抽屉模式）。 */
function AtomHistoryDrawer({ atom, onClose }: { atom: Atom; onClose: () => void }) {
  const [rows, setRows] = useState<AtomRevision[] | null>(null)
  useEffect(() => {
    let alive = true
    api.get<AtomRevision[]>(`/memory/atoms/${atom.id}/revisions`).then((r) => {
      if (alive) setRows(r)
    })
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => {
      alive = false
      window.removeEventListener('keydown', onKey)
    }
  }, [atom.id, onClose])

  return (
    <div className="fixed inset-0 z-50 flex justify-end" role="dialog" aria-label="原子改写历史">
      <div className="absolute inset-0 bg-foreground/20" onClick={onClose} aria-hidden />
      <aside className="relative flex h-full w-[440px] max-w-[92vw] flex-col border-l border-border bg-background shadow-lg">
        <header className="flex items-center justify-between gap-2 border-b border-border px-4 py-3">
          <div className="min-w-0">
            <h3 className="truncate font-medium">改写历史</h3>
            <p className="font-mono text-xs text-muted-foreground">
              {atom.id.slice(0, 8)} · 现值「{atom.content.slice(0, 24)}…」
            </p>
          </div>
          <Button variant="ghost" size="sm" onClick={onClose}>
            关闭
          </Button>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto p-4">
          {!rows ? (
            <p className="font-mono text-xs text-muted-foreground">加载留痕…</p>
          ) : rows.length === 0 ? (
            <p className="text-sm text-muted-foreground">还没有改写留痕——内容编辑（用户）会在这里留一条旧值。</p>
          ) : (
            <ul className="space-y-3">
              {rows.map((r) => (
                <li key={r.id} className="rounded-lg border border-border p-3">
                  <div className="flex flex-wrap items-center gap-1.5">
                    <span className="rounded border border-border px-1.5 py-px font-mono text-xs text-muted-foreground">
                      {r.old_kind}
                    </span>
                    <span className="text-xs text-muted-foreground">{relTime(r.created_at)}</span>
                    <span className="font-mono text-[10px] text-muted-foreground/60">by {r.edited_by}</span>
                    <span className="ml-auto font-mono text-xs text-muted-foreground">置信 {r.old_confidence.toFixed(2)}</span>
                  </div>
                  <p className="mt-1.5 line-clamp-3 text-sm text-muted-foreground">{r.old_content}</p>
                </li>
              ))}
            </ul>
          )}
        </div>
      </aside>
    </div>
  )
}

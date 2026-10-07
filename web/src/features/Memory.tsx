/** Memory 域：会话 / 原子 / 场景 / 画像 / 检索。 */
import { useEffect, useState } from 'react'
import { Navigate, useLocation, useNavigate } from 'react-router-dom'
import { api, type Atom, type Job, type Session } from '@/lib/api'
import {
  Card,
  Checkbox,
  DataTable,
  Empty,
  ErrorBox,
  Spinner,
  StatusBadge,
  Tabs,
  type DataTableSort,
} from '@/components/ui-bits'
import Pager from '@/components/Pager'
import { fmtTime, relTime, inputCls, selectCls, tableCls } from '@/lib/ui'
import { useSystemStatus } from '@/lib/status'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { appConfirm, type ConfirmOptions } from '@/components/confirm'
import WikiMarkdown from '@/components/WikiMarkdown'
import {
  Bookmark,
  Calendar,
  Heart,
  Lightbulb,
  PenLine,
  Repeat,
  Scale,
  ShieldAlert,
  TriangleAlert,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

type Tab = 'sessions' | 'atoms' | 'persona' | 'search' | 'kv'

/** 原子 kind 枚举（迁移 0005 CHECK）——筛选器与表格共用。 */
const ATOM_KINDS = ['preference', 'fact', 'decision', 'event', 'insight', 'correction', 'failure', 'convention']

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

/** 原子 kind 图标 + 色（八类八色，一眼认形态）。 */
const KIND_ICON: Record<string, { icon: LucideIcon; color: string }> = {
  preference: { icon: Heart, color: '#f43f5e' },
  fact: { icon: Bookmark, color: '#3b82f6' },
  decision: { icon: Scale, color: '#8b5cf6' },
  event: { icon: Calendar, color: '#f59e0b' },
  insight: { icon: Lightbulb, color: '#eab308' },
  correction: { icon: PenLine, color: '#06b6d4' },
  failure: { icon: TriangleAlert, color: '#ef4444' },
  convention: { icon: Repeat, color: '#10b981' },
}

/** kind 图标瓦片：色底色字圆角方块。 */
function KindTile({ kind }: { kind: string }) {
  const spec = KIND_ICON[kind] ?? { icon: Bookmark, color: '#6b7280' }
  const Icon = spec.icon
  return (
    <span
      className="flex size-6 shrink-0 items-center justify-center rounded-md"
      style={{ background: `${spec.color}1f`, color: spec.color }}
      title={KIND_LABEL[kind] ?? kind}
    >
      <Icon className="size-3.5" aria-hidden="true" />
    </span>
  )
}

/** 置信度微型量表：细条 + 数值——低置信一眼可见。 */
function ConfidenceMeter({ v }: { v: number }) {
  const pct = Math.round(v * 100)
  return (
    <span className="inline-flex items-center gap-1.5" title={`置信度 ${v.toFixed(2)}`}>
      <span className="h-1 w-10 overflow-hidden rounded-full bg-muted">
        <span
          className={cn(
            'block h-full rounded-full',
            v < 0.6 ? 'bg-yellow-500' : 'bg-emerald-500/70',
          )}
          style={{ width: `${pct}%` }}
        />
      </span>
      <span className={cn('font-mono text-xs', v < 0.6 && 'text-warning')}>{v.toFixed(2)}</span>
    </span>
  )
}

/** 画像分面中英对照（迁移 0005 CHECK 枚举的 7 个分面）。 */
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
    const valid: readonly string[] = ['sessions', 'atoms', 'search', 'kv']
    return valid.includes(t ?? '') ? (t as Tab) : 'sessions'
  })
  // 原子筛选状态提升——筛选器挂在 tab 行右侧（与蒸馏条同款布局，省一整行）
  const [atomKind, setAtomKind] = useState('')
  const [atomStatus, setAtomStatus] = useState('active')
  // tab 计数（原管线条带的职责）：挂载时取一次；蒸馏脉冲只表「正在炼」（processing）
  const [counts, setCounts] = useState<{ l0: number; l1: number; kv: number } | null>(null)
  useEffect(() => {
    Promise.all([
      api.get<Session[]>('/memory/sessions?limit=500').catch(() => []),
      api.get<Atom[]>('/memory/atoms?limit=500').catch(() => []),
      api.get<KvEntry[]>('/memory/kv?limit=500').catch(() => []),
    ]).then(([s, a, kv]) => {
      setCounts({
        l0: s.length,
        l1: a.filter((x) => x.status === 'active' || x.status === 'candidate').length,
        kv: kv.length,
      })
    })
  }, [])
  const { distilling } = useSystemStatus()

  const tabs = [
    { value: 'sessions' as Tab, label: '会话', count: counts?.l0, pulse: distilling > 0 },
    { value: 'atoms' as Tab, label: '原子', count: counts?.l1 },
    { value: 'persona' as Tab, label: '画像' },
    { value: 'kv' as Tab, label: 'KV', count: counts?.kv },
  ]

  return (
    <div className="space-y-6">
      {/* 头部压缩（P014）：标题 + 标签页 + 右侧操作同一行 */}
      <div className="flex flex-wrap items-center gap-4">
        <h1 className="text-lg font-semibold tracking-tight">用户记忆</h1>
        <Tabs items={tabs} value={tab} onChange={setTab} />
        <div className="ml-auto flex items-center gap-3">
          {tab === 'sessions' && <DistillBar />}
          {tab === 'atoms' && (
            <>
              <select
                className={selectCls}
                value={atomStatus}
                onChange={(e) => setAtomStatus(e.target.value)}
                aria-label="状态筛选"
              >
                <option value="active">生效</option>
                <option value="superseded">被取代</option>
                <option value="archived">归档</option>
                <option value="all">全部状态</option>
              </select>
              <select
                className={selectCls}
                value={atomKind}
                onChange={(e) => setAtomKind(e.target.value)}
                aria-label="kind 筛选"
              >
                <option value="">全部 kind</option>
                {ATOM_KINDS.map((k) => (
                  <option key={k} value={k}>
                    {KIND_LABEL[k] ?? k} {k}
                  </option>
                ))}
              </select>
            </>
          )}
        </div>
      </div>
      {tab === 'sessions' && <Sessions />}
      {tab === 'atoms' && <Atoms kind={atomKind} status={atomStatus} />}

      {tab === 'persona' && <PersonaDocPane />}
      {tab === 'kv' && <KvPane />}
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
  const [bulkBusy, setBulkBusy] = useState(false)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [notice, setNotice] = useState('')
  // 翻页（前端切页：拉满后本地分页，支持页码直跳）
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(20)
  // 列头排序（DataTable 受控三态；本地排序——数据已全量在前端）
  const [sort, setSort] = useState<DataTableSort | null>(null)
  const load = () => api.get<Session[]>('/memory/sessions?limit=200').then(setRows).catch((e) => setErr(e.message))
  useEffect(() => {
    load()
  }, [])
  if (err) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  // 翻页钳制：批量操作后总数变少时当前页可能越界
  const maxPage = Math.max(1, Math.ceil(rows.length / pageSize))
  const cur = Math.min(page, maxPage)
  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  const runBulk = async (path: string, confirmOpts: ConfirmOptions | null) => {
    if (confirmOpts && !(await appConfirm(confirmOpts))) return
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
      {notice && (
        <p className={cn('text-xs', notice.includes('失败') ? 'text-destructive' : 'text-info')}>{notice}</p>
      )}

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
              runBulk('/memory/sessions/batch-erase', {
                title: `擦除所选 ${selected.size} 条会话？`,
                description: '物理删除（含蒸馏产物级联），不可恢复。',
                destructive: true,
                confirmLabel: '擦除',
              })
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
        <>
          <DataTable
            columns={[
              {
                key: 'select',
                label: '',
                thClassName: 'w-10',
                tdClassName: 'w-10',
                render: (s) => (
                  <span onClick={(e) => e.stopPropagation()}>
                    <Checkbox checked={selected.has(s.id)} onChange={() => toggle(s.id)} label="选择该会话" />
                  </span>
                ),
              },
              {
                key: 'preview',
                label: '预览',
                tdClassName: 'w-full max-w-0 truncate font-medium',
                title: (s) => s.content?.[0]?.text ?? '',
                render: (s) => s.content?.[0]?.text ?? '（空会话）',
              },
              { key: 'agent', label: 'Agent', tdClassName: 'whitespace-nowrap font-mono text-xs text-muted-foreground' },
              { key: 'turns', label: '轮次', tdClassName: 'whitespace-nowrap font-mono text-xs', render: (s) => s.content?.length ?? 0 },
              {
                key: 'distill',
                label: '蒸馏',
                tdClassName: 'whitespace-nowrap',
                render: (s) => <StatusBadge status={s.distill_status} />,
              },
              {
                key: 'time',
                label: '时间',
                tdClassName: 'whitespace-nowrap text-muted-foreground',
                title: (s) => new Date(s.created_at).toLocaleString(),
                render: (s) => relTime(s.created_at),
              },
            ]}
            rows={(() => {
              // 全量排序后再分页（数据 limit=200 已全量在前端）
              const sorted = sort
                ? [...rows].sort((a, b) => {
                    const dir = sort.dir === 'asc' ? 1 : -1
                    return dir * String(a.created_at).localeCompare(String(b.created_at))
                  })
                : rows
              return sorted.slice((cur - 1) * pageSize, cur * pageSize)
            })()}
            rowKey={(s) => s.id}
            sort={sort}
            onSortChange={setSort}
            onRowClick={(s) => setOpenId(openId === s.id ? null : s.id)}
            isExpanded={(s) => openId === s.id}
            expandable={(s) => (
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
                        if (
                          !(await appConfirm({
                            title: '擦除该会话？',
                            description: '关联原子的溯源将标记为 erased，不可恢复。',
                            destructive: true,
                            confirmLabel: '擦除',
                          }))
                        )
                          return
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
            )}
          />
          <Pager
            total={rows.length}
            page={cur}
            pageSize={pageSize}
            onPage={setPage}
            onPageSize={(n) => {
              setPageSize(n)
              setPage(1)
            }}
          />
        </>
      )}
    </div>
  )
}

/** 蒸馏条：积压提示 + 触发蒸馏——挂在 tab 行右侧，不再独占一行。 */
function DistillBar() {
  const [busy, setBusy] = useState(false)
  const [backlog, setBacklog] = useState(0)
  const [msg, setMsg] = useState('')
  const count = () =>
    api
      .get<Session[]>('/memory/sessions?limit=200')
      .then((r) => setBacklog(r.filter((s) => s.distill_status === 'pending').length))
      .catch(() => undefined)
  useEffect(() => {
    count()
  }, [])
  return (
    <div className="flex items-center gap-2">
      {msg && <span className={cn('text-xs', msg.includes('失败') ? 'text-destructive' : 'text-info')}>{msg}</span>}
      {!msg && backlog > 0 && <span className="text-xs text-muted-foreground">{backlog} 条未蒸馏</span>}
      <Button
        size="sm"
        disabled={busy}
        onClick={async () => {
          setBusy(true)
          setMsg('')
          try {
            // 202 返回入队的 Job[]——空数组 = 没有待蒸馏会话
            const jobs = await api.post<Job[]>('/memory/distill', { full: false })
            setMsg(jobs.length > 0 ? `已入队 ${jobs.length} 个蒸馏（可在日志页看进度）` : '没有待蒸馏的会话')
            count()
          } catch (e) {
            setMsg(e instanceof Error ? `触发失败：${e.message}` : '触发失败')
          } finally {
            setBusy(false)
          }
        }}
      >
        {busy ? '提交中…' : '触发蒸馏'}
      </Button>
    </div>
  )
}

function Atoms({ kind, status }: { kind: string; status: string }) {
  const [rows, setRows] = useState<Atom[] | null>(null)
  const [err, setErr] = useState('')
  const [editing, setEditing] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  const [superseding, setSuperseding] = useState<string | null>(null)
  const [historyAtom, setHistoryAtom] = useState<Atom | null>(null)
  // 重嵌修复：缺失向量可见 + 一键补嵌（换 embedding 供应商后的修复路径）
  const [missing, setMissing] = useState<{ atoms_missing: number } | null>(null)
  const [reembedMsg, setReembedMsg] = useState('')
  // 翻页（前端切页：拉满后本地分页，支持页码直跳）
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(50)
  // 蒸馏进行中（系统状态轮询源）才有新原子产出——闲时不轮询，省请求
  const { distilling } = useSystemStatus()
  useEffect(() => {
    api
      .get<{ atoms_missing: number }>('/memory/embeddings/status')
      .then(setMissing)
      .catch(() => {})
  }, [])
  const params = () => {
    const p = new URLSearchParams({ limit: '200' })
    if (status && status !== 'all') p.set('status', status)
    if (kind) p.set('kind', kind)
    return p
  }
  const load = () => {
    api.get<Atom[]>(`/memory/atoms?${params()}`).then(setRows).catch((e) => setErr(e.message))
  }
  useEffect(() => {
    load()
    setPage(1) // 筛选变化回到第一页
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind, status])
  useEffect(() => {
    if (!distilling) return
    const t = setInterval(load, 5000)
    return () => clearInterval(t)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [distilling, kind, status])
  if (err) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  // 翻页钳制：蒸馏刷新后总数变少时当前页可能越界
  const maxPage = Math.max(1, Math.ceil(rows.length / pageSize))
  const cur = Math.min(page, maxPage)

  return (
    <div className="space-y-4">
      {missing && missing.atoms_missing > 0 && (
        <div className="flex flex-wrap items-center gap-2 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2">
          <span className="text-xs text-warning">
            {missing.atoms_missing} 条原子缺向量——写侧已即时嵌入，此提示通常为迁移/重嵌残留
          </span>
          <Button
            size="sm"
            variant="outline"
            onClick={async () => {
              setReembedMsg('')
              try {
                await api.post('/memory/reembed')
                setReembedMsg('重嵌已入队，完成后向量通道自动恢复（日志页可见）')
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
          <DataTable
            columns={[
              {
                key: 'kind',
                label: '类型',
                render: (a) => (
                  <span className="flex items-center gap-2 whitespace-nowrap">
                    <KindTile kind={a.kind} />
                    <span className="text-sm">{KIND_LABEL[a.kind] ?? a.kind}</span>
                  </span>
                ),
              },
              {
                key: 'content',
                label: '内容',
                thClassName: 'w-[60%]',
                render: (a) =>
                  editing === a.id ? (
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
                      {a.content}
                    </span>
                  ),
              },
              {
                key: 'confidence',
                label: '置信',
                tdClassName: 'whitespace-nowrap',
                title: () => '置信度（低于 0.60 黄色提示）',
                render: (a) => <ConfidenceMeter v={a.confidence} />,
              },
              {
                key: 'status',
                label: '状态',
                render: (a) => (
                  <>
                    <StatusBadge status={a.status} />
                    {a.superseded_by && (
                      <span className="ml-1.5 font-mono text-xs text-muted-foreground">
                        → {a.superseded_by.slice(0, 8)}
                      </span>
                    )}
                  </>
                ),
              },
              {
                key: 'refs',
                label: '溯源',
                thClassName: 'whitespace-nowrap',
                tdClassName: tableCls.tdMono,
                title: (a) =>
                  (a.source_refs ?? [])
                    .map((r) => (r.session_id ? r.session_id.slice(0, 8) + (r.erased ? '（已擦除）' : '') : ''))
                    .filter(Boolean)
                    .join(' · ') || '无溯源',
                render: (a) => a.source_refs?.length ?? 0,
              },
              { key: 'hits', label: '命中', thClassName: 'whitespace-nowrap', tdClassName: tableCls.tdMono, render: (a) => a.hit_count },
              {
                key: 'actions',
                label: '',
                tdClassName: 'whitespace-nowrap text-right',
                render: (a) => (
                  <>
                    <Button
                      variant="ghost"
                      size="sm"
                      className={cn('mr-1 gap-1', a.sensitive && 'text-warning')}
                      title={a.sensitive ? '敏感原子（检索/快照隐身，点击取消）' : '标记敏感（医疗/感情/财务等，检索与快照隐身）'}
                      onClick={async () => {
                        await api.patch(`/memory/atoms/${a.id}`, { sensitive: !a.sensitive })
                        setRows(rows.map((r) => (r.id === a.id ? { ...r, sensitive: !a.sensitive } : r)))
                      }}
                    >
                      {a.sensitive ? (
                        <>
                          <ShieldAlert className="size-3.5" aria-hidden="true" />
                          已敏感
                        </>
                      ) : (
                        '敏感'
                      )}
                    </Button>
                    <Button variant="ghost" size="sm" className="mr-1" title="改写留痕历史" onClick={() => setHistoryAtom(a)}>
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
                  </>
                ),
              },
            ]}
            rows={rows.slice((cur - 1) * pageSize, cur * pageSize)}
            rowKey={(a) => a.id}
          />
          <Pager
            total={rows.length}
            page={cur}
            pageSize={pageSize}
            onPage={setPage}
            onPageSize={(n) => {
              setPageSize(n)
              setPage(1)
            }}
            hint={rows.length >= 200 ? '仅载入最近 200 条' : undefined}
          />
        </>
      )}
      {historyAtom && (
        <AtomHistoryDrawer atom={historyAtom} onClose={() => setHistoryAtom(null)} />
      )}
    </div>
  )
}


interface EntityHit {
  id: string
  title: string | null
  snippet: string
  score: number
  kind: string | null
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
  } | null>(null)
  const [err, setErr] = useState('')
  const empty = r !== null && r.entities.length + r.l1.length === 0
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
                  setArchiveMsg('已存档到 wiki')
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


/** 画像活文档（persona_doc）：离线整理 Agent 维护的单份 Markdown 活文档 + 版本史。
 *  版本管理式两栏：左版本史（当前版在顶），右正文 Markdown 渲染 / 逐版行级 diff。 */
interface PersonaDocData {
  doc: { content: string; summary: string | null; version: number; updated_at: string } | null
  history: { version: number; content: string; summary: string | null; created_at: string }[]
}

/** 行级 LCS diff（画像文档小，O(n·m) 足够）——版本管理感的核心。 */
function diffLines(a: string, b: string): { t: 'same' | 'add' | 'del'; s: string }[] {
  const x = a.split('\n')
  const y = b.split('\n')
  const n = x.length
  const m = y.length
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0))
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--)
      dp[i][j] = x[i] === y[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1])
  const out: { t: 'same' | 'add' | 'del'; s: string }[] = []
  let i = 0
  let j = 0
  while (i < n && j < m) {
    if (x[i] === y[j]) {
      out.push({ t: 'same', s: x[i] })
      i++
      j++
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      out.push({ t: 'del', s: x[i] })
      i++
    } else {
      out.push({ t: 'add', s: y[j] })
      j++
    }
  }
  while (i < n) out.push({ t: 'del', s: x[i++] })
  while (j < m) out.push({ t: 'add', s: y[j++] })
  return out
}

/** diff 视图：红删绿增、前缀 +/-——跟 git 终端同语言。 */
function LineDiff({ oldText, newText }: { oldText: string; newText: string }) {
  const rows = diffLines(oldText, newText)
  return (
    <div className="overflow-hidden rounded-lg border border-border font-mono text-xs leading-5">
      {rows.map((r, i) => (
        <div
          key={i}
          className={cn(
            'whitespace-pre-wrap px-3 py-0.5',
            r.t === 'add' && 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-300',
            r.t === 'del' && 'bg-red-500/10 text-red-500',
          )}
        >
          <span className="mr-2 select-none text-muted-foreground/50">
            {r.t === 'add' ? '+' : r.t === 'del' ? '-' : ' '}
          </span>
          {r.s || ' '}
        </div>
      ))}
    </div>
  )
}

function PersonaDocPane() {
  const [data, setData] = useState<PersonaDocData | null>(null)
  // 选中版本（null = 当前版）；diff 模式对比「选中版 vs 它的上一版」
  const [sel, setSel] = useState<number | null>(null)
  const [diffMode, setDiffMode] = useState(false)
  useEffect(() => {
    api
      .get<PersonaDocData>('/memory/persona-doc?history_limit=20')
      .then(setData)
      .catch(() => setData({ doc: null, history: [] }))
  }, [])
  if (!data) return <Spinner />
  const { doc, history } = data
  if (!doc)
    return <Empty text="画像尚未生成——记忆蒸馏后由离线整理 Agent 自动维护" />

  // 版本链：当前版在顶，历史随后（新→旧）
  const versions: PersonaDocData['history'] = [
    {
      version: doc.version,
      content: doc.content,
      summary: doc.summary,
      created_at: doc.updated_at,
    },
    ...history,
  ]
  const cur = versions.find((v) => v.version === (sel ?? doc.version)) ?? versions[0]
  const base = (() => {
    const idx = versions.findIndex((v) => v.version === cur.version)
    return idx >= 0 && idx + 1 < versions.length ? versions[idx + 1] : null
  })()

  return (
    <div className="grid grid-cols-1 gap-4 lg:h-[calc(100dvh-7.5rem)] lg:grid-cols-[15rem_minmax(0,1fr)]">
      {/* 左：版本史——版本管理的主入口，不藏折叠 */}
      <div className="min-h-0 space-y-1.5 lg:h-full lg:overflow-y-auto lg:pr-1">
        <div className="px-1 text-[11px] font-medium uppercase tracking-wider text-muted-foreground/80">
          版本 · 新 → 旧
        </div>
        {versions.map((v, i) => {
          const active = v.version === cur.version
          return (
            <button
              key={v.version}
              className={cn(
                'block w-full rounded-lg border px-3 py-2 text-left transition-colors',
                active
                  ? 'border-primary/45 bg-primary/5 shadow-[inset_2px_0_0_0] shadow-primary/50'
                  : 'border-border hover:border-foreground/20 hover:bg-muted/40',
              )}
              onClick={() => {
                setSel(v.version)
                setDiffMode(false)
              }}
            >
              <div className="flex items-center justify-between gap-2">
                <span className="font-mono text-xs font-medium">
                  v{v.version}
                  {i === 0 && (
                    <span className="ml-1.5 rounded bg-emerald-500/15 px-1 py-0.5 text-[10px] text-emerald-500">
                      当前
                    </span>
                  )}
                </span>
                <span className="shrink-0 text-[10px] text-muted-foreground">{relTime(v.created_at)}</span>
              </div>
              {v.summary && (
                <div className="mt-0.5 truncate text-[11px] text-muted-foreground">{v.summary}</div>
              )}
            </button>
          )
        })}
      </div>

      {/* 右：正文（Markdown 渲染）或版本间 diff */}
      <Card className="flex min-h-0 flex-col p-4 lg:h-full">
        <div className="mb-3 flex items-center justify-between gap-2">
          <div className="min-w-0">
            <span className="text-sm font-medium">画像 v{cur.version}</span>
            <span className="ml-2 text-xs text-muted-foreground">{fmtTime(cur.created_at)}</span>
          </div>
          {base && (
            <Button
              size="sm"
              variant={diffMode ? 'default' : 'outline'}
              onClick={() => setDiffMode((m) => !m)}
            >
              {diffMode ? '退出 diff' : `diff v${base.version} → v${cur.version}`}
            </Button>
          )}
        </div>
        {cur.summary && !diffMode && (
          <p className="mb-3 text-xs text-muted-foreground">{cur.summary}</p>
        )}
        {/* 正文区独立滚动——留白吃掉，长文档不出页 */}
        <div className="min-h-0 flex-1 overflow-y-auto">
          {diffMode && base ? (
            <LineDiff oldText={base.content} newText={cur.content} />
          ) : (
            <WikiMarkdown content={cur.content} />
          )}
        </div>
      </Card>
    </div>
  )
}

/** KV 精确值条目（EN-60 治理面只读；写入唯一通道是 MCP memory.kv_put）。 */
interface KvEntry {
  key: string
  value: string
  context: string
  tags: string[]
  source: string
  updated_at: string
}

/** KV 治理区：AI 管道存的权威精确值（序列号/UUID/路径…），人只读。 */
function KvPane() {
  const [rows, setRows] = useState<KvEntry[] | null>(null)
  const [err, setErr] = useState('')
  const [q, setQ] = useState('')
  const [openKey, setOpenKey] = useState<string | null>(null)
  // 翻页（前端切页：KV 一次拉全，支持页码直跳）
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(50)
  const load = (query: string) =>
    api
      .get<KvEntry[]>(`/memory/kv?limit=500${query ? `&q=${encodeURIComponent(query)}` : ''}`)
      .then(setRows)
      .catch((e) => setErr(e.message))
  useEffect(() => {
    load('')
  }, [])
  if (err) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  // 翻页钳制
  const maxPage = Math.max(1, Math.ceil(rows.length / pageSize))
  const cur = Math.min(page, maxPage)
  // value 尽量按 JSON 美化（parse 失败按原样展示）
  const pretty = (v: string) => {
    try {
      return JSON.stringify(JSON.parse(v), null, 2)
    } catch {
      return v
    }
  }
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <input
          className={inputCls}
          aria-label="KV 检索"
          placeholder="按键名 / 值 / 上下文子串检索…"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') load(q)
          }}
        />
        <Button size="sm" variant="secondary" onClick={() => load(q)}>
          检索
        </Button>
      </div>
      {rows.length === 0 ? (
        <Empty text="没有命中的 KV 条目" />
      ) : (
        <>
        <DataTable
          columns={[
            {
              key: 'key',
              label: '键名',
              tdClassName: `${tableCls.tdMono} max-w-52 truncate font-medium`,
              title: (r) => r.key,
              render: (r) => r.key,
            },
            {
              key: 'value',
              label: '值',
              tdClassName: `${tableCls.tdMono} max-w-64 truncate`,
              title: (r) => r.value,
              render: (r) => r.value,
            },
            {
              key: 'context',
              label: '上下文',
              tdClassName: `${tableCls.td} max-w-72 truncate text-muted-foreground`,
              title: (r) => r.context,
              render: (r) => r.context,
            },
            {
              key: 'source',
              label: '来源',
              thClassName: 'whitespace-nowrap',
              tdClassName: `${tableCls.td} whitespace-nowrap text-muted-foreground`,
              render: (r) => r.source,
            },
            {
              key: 'updated',
              label: '更新',
              thClassName: 'whitespace-nowrap',
              tdClassName: `${tableCls.td} whitespace-nowrap text-muted-foreground`,
              render: (r) => relTime(r.updated_at),
            },
          ]}
          rows={rows.slice((cur - 1) * pageSize, cur * pageSize)}
          rowKey={(r) => r.key}
          onRowClick={(r) => setOpenKey(openKey === r.key ? null : r.key)}
        />
        <Pager
            total={rows.length}
            page={cur}
            pageSize={pageSize}
            onPage={setPage}
            onPageSize={(n) => {
              setPageSize(n)
              setPage(1)
            }}
          />
        </>
      )}
      {openKey &&
        (() => {
          const r = rows.find((x) => x.key === openKey)
          if (!r) return null
          return (
            <div className="rounded-md border border-border bg-muted/20 p-4 text-sm leading-6">
              <div className="flex items-center justify-between">
                <h3 className="font-mono text-sm font-semibold break-all">{r.key}</h3>
                <button
                  type="button"
                  className="text-xs text-muted-foreground hover:text-foreground"
                  onClick={() => setOpenKey(null)}
                >
                  收起
                </button>
              </div>
              <dl className="mt-3 space-y-2">
                <div>
                  <dt className="text-xs text-muted-foreground">值（逐字保存）</dt>
                  <dd className="mt-0.5 max-h-80 overflow-auto break-all rounded bg-background p-2 font-mono text-xs whitespace-pre-wrap">
                    {pretty(r.value)}
                  </dd>
                </div>
                {r.context && (
                  <div>
                    <dt className="text-xs text-muted-foreground">上下文</dt>
                    <dd className="mt-0.5 text-xs break-words">{r.context}</dd>
                  </div>
                )}
                <div className="flex gap-4 text-xs text-muted-foreground">
                  <span>来源：{r.source}</span>
                  <span>标签：{r.tags.length ? r.tags.join(' / ') : '—'}</span>
                  <span>更新：{fmtTime(r.updated_at)}</span>
                </div>
              </dl>
            </div>
          )
        })()}
    </div>
  )
}

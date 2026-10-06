/**
 * 领域工作台（/study/:id · 2026-10-06 重设计）：
 * 一个学习命题一个专属界面——路线图画布（roadmap.sh 式）+ 节点详情面板 +
 * 本领域复习队列（Leitner 记得升档/忘了回档）+ 快速日志。
 * 分工边界不变：study 只管过程状态；知识本体归 wiki，感悟归 memory。
 */
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { api } from '@/lib/api'
import { cn } from '@/lib/utils'
import { Card, Empty, ErrorBox, Spinner, StatusBadge } from '@/components/ui-bits'
import { inputCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import {
  DAY,
  LADDER,
  type StudyItem,
  type StudyJournalRow,
  type TopicFull,
  currentIntervalDays,
  dueIn,
  nextLadderDays,
  ProgressRing,
  dayKey,
} from './Study'

function relTime(ts: string): string {
  const s = Math.floor((Date.now() - new Date(ts).getTime()) / 1000)
  if (s < 60) return '刚刚'
  if (s < 3600) return `${Math.floor(s / 60)} 分钟前`
  if (s < 86400) return `${Math.floor(s / 3600)} 小时前`
  return `${Math.floor(s / 86400)} 天前`
}

const STATUS_LABEL: Record<string, string> = {
  not_started: '待学',
  learning: '进行中',
  learned: '已学',
}

const STATUS_DOT: Record<string, string> = {
  not_started: 'border-2 border-muted-foreground/50 bg-transparent',
  learning: 'border-2 border-amber-500 bg-amber-500/40 animate-pulse',
  learned: 'border-2 border-emerald-500 bg-emerald-500',
}

export default function StudyWorkspace() {
  const { id } = useParams<{ id: string }>()
  const nav = useNavigate()
  const [full, setFull] = useState<TopicFull | null>(null)
  const [journal, setJournal] = useState<StudyJournalRow[]>([])
  const [reviews, setReviews] = useState<StudyItem[]>([])
  const [err, setErr] = useState('')
  const [expanded, setExpanded] = useState<string | null>(null)
  const [newItem, setNewItem] = useState('')
  const [journalNote, setJournalNote] = useState('')
  const [editing, setEditing] = useState(false)
  const [eName, setEName] = useState('')
  const [eGoal, setEGoal] = useState('')

  const load = useCallback(async () => {
    if (!id) return
    try {
      const [f, j, r] = await Promise.all([
        api.get<TopicFull>(`/study/topics/${id}`),
        api.get<{ journal: StudyJournalRow[] }>(`/study/topics/${id}/journal`),
        api.get<{ reviews: StudyItem[] }>('/study/reviews').catch(() => ({ reviews: [] as StudyItem[] })),
      ])
      setFull(f)
      setJournal(j.journal)
      setReviews(r.reviews.filter((it) => it.track_id === id))
    } catch (e) {
      setErr(String(e))
    }
  }, [id])
  useEffect(() => {
    load()
  }, [load])

  const dueForTrack = reviews
  const streak = useMemo(() => {
    const days = new Set(journal.map((j) => dayKey(j.created_at)))
    const today = new Date()
    today.setHours(12, 0, 0, 0)
    let n = 0
    const start = days.has(dayKey(today.getTime())) ? 0 : 1
    for (let i = start; i < 400; i++) {
      if (days.has(dayKey(today.getTime() - i * DAY))) n++
      else break
    }
    return n
  }, [journal])

  if (err && !full) return <ErrorBox msg={err} />
  if (!full) return <Spinner />

  const pct = full.progress.total ? Math.round((full.progress.learned / full.progress.total) * 100) : 0

  // ---------- 动作 ----------
  async function patchItem(itemId: string, body: Record<string, unknown>) {
    setErr('')
    try {
      await api.patch(`/study/items/${itemId}`, body)
      await load()
    } catch (e) {
      setErr(String(e))
    }
  }

  async function markRemembered(it: StudyItem) {
    await patchItem(it.id, { needs_review: true, review_due_at: dueIn(nextLadderDays(it.review_due_at, it.updated_at)) })
  }

  async function markForgot(it: StudyItem) {
    await patchItem(it.id, { needs_review: true, review_due_at: dueIn(1) })
  }

  async function startReview(it: StudyItem) {
    await patchItem(it.id, { needs_review: true, review_due_at: dueIn(3) })
  }

  async function addWiki(it: StudyItem, slug: string) {
    if (!slug.trim()) return
    await patchItem(it.id, { wiki_slugs: [...(it.wiki_slugs ?? []), slug.trim()] })
  }

  async function setTopicStatus(status: string) {
    setErr('')
    try {
      await api.patch(`/study/topics/${id}`, { status })
      await load()
    } catch (e) {
      setErr(String(e))
    }
  }

  async function saveTopic() {
    setErr('')
    try {
      await api.patch(`/study/topics/${id}`, { name: eName.trim(), goal: eGoal.trim() })
      setEditing(false)
      await load()
    } catch (e) {
      setErr(String(e))
    }
  }

  async function addItem() {
    if (!newItem.trim()) return
    setErr('')
    try {
      await api.post(`/study/topics/${id}/items`, { name: newItem.trim() })
      setNewItem('')
      await load()
    } catch (e) {
      setErr(String(e))
    }
  }

  async function delItem(itemId: string) {
    setErr('')
    try {
      await api.del(`/study/items/${itemId}`)
      setExpanded(null)
      await load()
    } catch (e) {
      setErr(String(e))
    }
  }

  async function addJournal() {
    if (!journalNote.trim()) return
    setErr('')
    try {
      await api.post(`/study/topics/${id}/journal`, { note: journalNote.trim() })
      setJournalNote('')
      await load()
    } catch (e) {
      setErr(String(e))
    }
  }

  // ---------- 节点卡片 ----------
  function NodeCard({ it, index }: { it: StudyItem; index: number }) {
    const isOpen = expanded === it.id
    const overdue =
      it.needs_review && it.review_due_at && new Date(it.review_due_at) < new Date()
    return (
      <div className="relative pl-7">
        {/* 路径连线 + 状态点 */}
        <span
          className={cn(
            'absolute top-4 left-[7px] h-1.5 w-1.5 rounded-full',
            STATUS_DOT[it.status]?.replace('animate-pulse', '') ?? '',
          )}
        />
        {index > 0 && <span className="absolute top-0 left-[13px] h-4 w-px bg-border" />}
        <div
          className={cn(
            'rounded-lg border p-3 transition-colors',
            isOpen ? 'border-primary/60 bg-primary/5' : 'border-border hover:bg-muted/40',
          )}
        >
          <button
            className="flex w-full items-center gap-2 text-left"
            onClick={() => setExpanded(isOpen ? null : it.id)}
          >
            <span className="text-xs text-muted-foreground">{String(index + 1).padStart(2, '0')}</span>
            <span className="font-medium">{it.name}</span>
            <span
              className={cn(
                'rounded px-1.5 py-0.5 text-xs',
                it.status === 'learned'
                  ? 'bg-emerald-500/15 text-emerald-600 dark:text-emerald-400'
                  : it.status === 'learning'
                    ? 'bg-amber-500/15 text-amber-600 dark:text-amber-400'
                    : 'bg-muted text-muted-foreground',
              )}
            >
              {STATUS_LABEL[it.status] ?? it.status}
            </span>
            {overdue && (
              <span className="rounded bg-red-500/15 px-1.5 py-0.5 text-xs text-red-400">
                复习逾期 {relTime(it.review_due_at!)}
              </span>
            )}
            {it.wiki_slugs?.length > 0 && (
              <span className="text-xs text-muted-foreground">🔗 {it.wiki_slugs.length}</span>
            )}
            <span className="ml-auto text-xs text-muted-foreground">{isOpen ? '收起 ▲' : '展开 ▼'}</span>
          </button>

          {/* 节点详情面板 */}
          {isOpen && (
            <div className="mt-3 space-y-3 border-t border-border pt-3">
              {/* 三态切换 */}
              <div className="flex items-center gap-1.5">
                <span className="text-xs text-muted-foreground">状态</span>
                {(['not_started', 'learning', 'learned'] as const).map((s) => (
                  <button
                    key={s}
                    className={cn(
                      'rounded border px-2 py-0.5 text-xs',
                      it.status === s
                        ? 'border-primary/60 bg-primary/10 font-medium'
                        : 'border-border text-muted-foreground hover:bg-muted',
                    )}
                    onClick={() => patchItem(it.id, { status: s })}
                  >
                    {STATUS_LABEL[s]}
                  </button>
                ))}
                {it.learned_at && (
                  <span className="text-xs text-muted-foreground">· 学于 {new Date(it.learned_at).toLocaleDateString()}</span>
                )}
              </div>

              {/* 复习调度（Leitner 阶梯） */}
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-xs text-muted-foreground">复习</span>
                {it.needs_review ? (
                  <>
                    <span className="text-xs">
                      {it.review_due_at
                        ? new Date(it.review_due_at) < new Date()
                          ? '已到期'
                          : `${new Date(it.review_due_at).toLocaleDateString()} 到期`
                        : '未排期'}
                      {it.review_due_at && ` · 当前间隔 ${currentIntervalDays(it.review_due_at, it.updated_at)} 天`}
                    </span>
                    <Button size="sm" variant="outline" onClick={() => markRemembered(it)}>
                      记得 →{nextLadderDays(it.review_due_at, it.updated_at)}天
                    </Button>
                    <Button size="sm" variant="outline" onClick={() => markForgot(it)}>
                      忘了 →1天
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => patchItem(it.id, { needs_review: false })}>
                      停止复习
                    </Button>
                  </>
                ) : (
                  <>
                    <span className="text-xs text-muted-foreground">未加入复习</span>
                    <Button size="sm" variant="outline" disabled={it.status !== 'learned'} onClick={() => startReview(it)}>
                      加入复习（3 天后到期）
                    </Button>
                    {it.status !== 'learned' && (
                      <span className="text-xs text-muted-foreground">学了才能复习</span>
                    )}
                  </>
                )}
              </div>

              {/* wiki 挂链 */}
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="text-xs text-muted-foreground">资料</span>
                {(it.wiki_slugs ?? []).map((s) => (
                  <a
                    key={s}
                    className="rounded bg-muted px-1.5 py-0.5 text-xs hover:bg-muted/70"
                    href={`/wiki?slug=${encodeURIComponent(s)}`}
                  >
                    [[{s}]]
                  </a>
                ))}
                <WikiAddForm onAdd={(slug) => addWiki(it, slug)} />
              </div>

              <div className="flex justify-end">
                <Button size="sm" variant="ghost" className="text-destructive" onClick={() => delItem(it.id)}>
                  删除节点
                </Button>
              </div>
            </div>
          )}
        </div>
      </div>
    )
  }

  const items = [...full.items].sort((a, b) => a.position - b.position)

  return (
    <div className="space-y-4">
      {/* 头部：返回 + 领域身份 + 进度环 */}
      <div className="flex flex-wrap items-center gap-3">
        <button
          className="rounded border border-border px-2 py-1 text-xs text-muted-foreground hover:bg-muted"
          onClick={() => nav('/study')}
        >
          ← 返回总览
        </button>
        {editing ? (
          <div className="flex flex-1 flex-wrap items-center gap-2">
            <input className={inputCls + ' w-56'} value={eName} onChange={(e) => setEName(e.target.value)} />
            <input
              className={inputCls + ' w-96'}
              placeholder="目标"
              value={eGoal}
              onChange={(e) => setEGoal(e.target.value)}
            />
            <Button size="sm" onClick={saveTopic}>
              保存
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setEditing(false)}>
              取消
            </Button>
          </div>
        ) : (
          <>
            <h1 className="text-lg font-semibold">{full.name}</h1>
            <StatusBadge status={full.status} />
            <select
              className="rounded border border-border bg-transparent px-1.5 py-0.5 text-xs"
              value={full.status}
              onChange={(e) => setTopicStatus(e.target.value)}
              aria-label="领域状态"
            >
              <option value="active">进行中</option>
              <option value="paused">暂停</option>
              <option value="done">已归档</option>
            </select>
            <button
              className="text-xs text-muted-foreground hover:text-foreground"
              onClick={() => {
                setEName(full.name)
                setEGoal(full.goal)
                setEditing(true)
              }}
            >
              编辑
            </button>
            <div className="ml-auto flex items-center gap-3">
              <div className="text-right">
                <div className="text-lg font-semibold leading-none">{pct}%</div>
                <div className="text-xs text-muted-foreground">
                  {full.progress.learned}/{full.progress.total} 节点
                </div>
              </div>
              <ProgressRing pct={pct} size={44} />
            </div>
          </>
        )}
      </div>
      {full.goal && !editing && <div className="text-sm text-muted-foreground">目标：{full.goal}</div>}

      {err && <ErrorBox msg={err} />}

      {/* 统计行 */}
      <Card className="flex flex-wrap items-center gap-x-6 gap-y-1 px-4 py-2.5 text-sm">
        <span className="text-muted-foreground">
          已学 <span className="font-medium text-foreground">{full.progress.learned}</span>
        </span>
        <span className="text-muted-foreground">
          进行中 <span className="font-medium text-foreground">{full.in_progress.length}</span>
        </span>
        <span className={dueForTrack.length > 0 ? 'text-amber-600 dark:text-amber-400' : 'text-muted-foreground'}>
          待复习 {dueForTrack.length}
        </span>
        <span className="text-muted-foreground">
          🔥 本领域连续 <span className="font-medium text-foreground">{streak}</span> 天
        </span>
        <span className="ml-auto text-xs text-muted-foreground">
          复习阶梯 {LADDER.join(' → ')} 天
        </span>
      </Card>

      {/* 主区：画布 + 侧栏，各自独立滚动 */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,24rem)] lg:h-[calc(100dvh-13.5rem)]">
        {/* 路线图画布 */}
        <div className="min-w-0 space-y-1 lg:h-full lg:overflow-y-auto lg:pr-1">
          {items.length === 0 ? (
            <Empty text="路线图还是空的——加第一个知识点" />
          ) : (
            items.map((it, i) => <NodeCard key={it.id} it={it} index={i} />)
          )}
          <div className="pl-7 pt-2">
            <div className="flex items-center gap-2">
              <input
                className={inputCls + ' w-72'}
                placeholder="加知识点（如：生命周期标注）"
                value={newItem}
                onChange={(e) => setNewItem(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && addItem()}
              />
              <Button size="sm" variant="outline" disabled={!newItem.trim()} onClick={addItem}>
                ＋ 加节点
              </Button>
            </div>
          </div>
        </div>

        {/* 领域侧栏 */}
        <div className="min-w-0 space-y-3 lg:self-start lg:max-h-full lg:overflow-y-auto lg:overflow-x-hidden lg:pl-1">
          {/* 本领域复习队列 */}
          <Card className="p-4">
            <div className="mb-2 text-xs font-medium text-muted-foreground">
              📅 本领域复习队列
              {dueForTrack.length > 0 && (
                <span className="ml-1.5 rounded bg-amber-500/15 px-1.5 py-0.5 text-amber-600 dark:text-amber-400">
                  {dueForTrack.length}
                </span>
              )}
            </div>
            {dueForTrack.length === 0 ? (
              <div className="text-xs text-muted-foreground">没有到期的复习——记得的东西都在阶梯上。</div>
            ) : (
              <div className="space-y-2">
                {dueForTrack.map((it) => (
                  <div key={it.id} className="flex items-center gap-2 text-sm">
                    <span className="min-w-0 flex-1 truncate">{it.name}</span>
                    <Button size="sm" variant="outline" onClick={() => markRemembered(it)}>
                      记得
                    </Button>
                    <Button size="sm" variant="outline" onClick={() => markForgot(it)}>
                      忘了
                    </Button>
                  </div>
                ))}
              </div>
            )}
          </Card>

          {/* 下一步队列 */}
          <Card className="p-4">
            <div className="mb-2 text-xs font-medium text-muted-foreground">▶ 下一步</div>
            {full.next_up.length === 0 ? (
              <div className="text-xs text-muted-foreground">队列空——都在学或学完了。</div>
            ) : (
              <div className="space-y-2">
                {full.next_up.map((it) => (
                  <div key={it.id} className="flex items-center gap-2 text-sm">
                    <span className="min-w-0 flex-1 truncate">{it.name}</span>
                    <Button size="sm" variant="outline" onClick={() => patchItem(it.id, { status: 'learning' })}>
                      开始
                    </Button>
                  </div>
                ))}
              </div>
            )}
          </Card>

          {/* 快速日志 + 时间线 */}
          <Card className="p-4">
            <div className="mb-2 text-xs font-medium text-muted-foreground">✎ 学习日志</div>
            <div className="flex items-center gap-2">
              <input
                className={inputCls + ' w-full'}
                placeholder="今天学到哪了…（回车记一笔）"
                value={journalNote}
                onChange={(e) => setJournalNote(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && addJournal()}
              />
            </div>
            {journal.length === 0 ? (
              <div className="mt-3 text-xs text-muted-foreground">还没有日志。</div>
            ) : (
              <ol className="mt-3 space-y-2 border-l border-border pl-3">
                {journal.map((j) => (
                  <li key={j.id} className="relative text-xs">
                    <span className="absolute top-1.5 -left-[17px] h-1.5 w-1.5 rounded-full bg-muted-foreground/60" />
                    <div>{j.note}</div>
                    <div className="text-muted-foreground">
                      {relTime(j.created_at)} · {new Date(j.created_at).toLocaleString()}
                    </div>
                  </li>
                ))}
              </ol>
            )}
          </Card>
        </div>
      </div>
    </div>
  )
}

/** 挂 wiki 小表单（内联，回车提交）。 */
function WikiAddForm({ onAdd }: { onAdd: (slug: string) => void }) {
  const [v, setV] = useState('')
  const submit = () => {
    if (!v.trim()) return
    onAdd(v.trim())
    setV('')
  }
  return (
    <input
      className="w-32 rounded border border-border bg-transparent px-1.5 py-0.5 text-xs placeholder:text-muted-foreground"
      placeholder="+ 挂 wiki slug"
      value={v}
      onChange={(e) => setV(e.target.value)}
      onKeyDown={(e) => e.key === 'Enter' && submit()}
      onBlur={submit}
    />
  )
}

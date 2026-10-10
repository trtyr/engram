/**
 * 学习驾驶舱 · 总览（2026-10-06 重设计，参考 roadmap.sh / FSRS / GitHub 热力图）：
 * 所有领域的仪表盘——进度环、全局活动热力图、连击、跨领域今日到期复习。
 * 点领域卡 → /study/:id 领域工作台（独立路由，那才是主界面）。
 */
import { useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { api } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner, StatusBadge } from '@/components/ui-bits'
import { fmtTime } from '@/lib/ui'

// ---------- 类型（与领域工作台共享） ----------
export interface StudyItem {
  id: string
  track_id: string
  name: string
  status: string // not_started | learning | learned
  position: number
  wiki_slugs: string[]
  doc_ids: string[]
  learned_at: string | null
  needs_review: boolean
  review_due_at: string | null
  updated_at: string
}

export interface StudyJournalRow {
  id: string
  track_id: string
  note: string
  created_at: string
}

export interface TopicFull {
  id: string
  name: string
  goal: string
  status: string
  updated_at: string
  items: StudyItem[]
  progress: { total: number; learned: number }
  next_up: StudyItem[]
  in_progress: StudyItem[]
  recent_journal: StudyJournalRow[]
}

export interface TopicBrief {
  id: string
  name: string
  goal: string
  status: string
  updated_at: string
}

// ---------- Leitner 复习阶梯（FSRS 的极简替身：记得升档，忘了回 1 天） ----------
export const LADDER = [1, 3, 7, 14, 30, 60]

export function currentIntervalDays(dueAt: string | null, updatedAt: string): number {
  if (!dueAt) return 0
  const d = (new Date(dueAt).getTime() - new Date(updatedAt).getTime()) / 86400_000
  return Math.max(1, Math.round(d))
}

/** 「记得」→ 阶梯升一档（封顶 60 天） */
export function nextLadderDays(dueAt: string | null, updatedAt: string): number {
  const cur = currentIntervalDays(dueAt, updatedAt)
  const idx = LADDER.findIndex((x) => x >= cur)
  return LADDER[Math.min((idx < 0 ? LADDER.length - 1 : idx) + 1, LADDER.length - 1)]
}

export function dueIn(days: number): string {
  return new Date(Date.now() + days * 86400_000).toISOString()
}

// ---------- 进度环 ----------
export function ProgressRing({ pct, size = 52 }: { pct: number; size?: number }) {
  const r = (size - 6) / 2
  const c = 2 * Math.PI * r
  return (
    <svg width={size} height={size} className="-rotate-90" aria-label={`进度 ${pct}%`}>
      <circle cx={size / 2} cy={size / 2} r={r} fill="none" strokeWidth={5} className="stroke-muted" />
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        fill="none"
        strokeWidth={5}
        strokeLinecap="round"
        className="stroke-emerald-500 transition-all"
        strokeDasharray={`${(c * pct) / 100} ${c}`}
      />
    </svg>
  )
}

// ---------- 活动热力图（近 12 周）+ 连击 ----------
export const DAY = 86400_000

/// P019-M3：热力图日期源——journal 成功用其记录，失败（null）回退该领域已学节点 learned_at；
/// 空值一律过滤（旧实现 learned_at ?? '' 产生空串 → dayKey('') → RangeError 整页崩溃）。
export function heatmapDatesFrom(
  js: ({ journal: StudyJournalRow[] } | null)[],
  fs: (TopicFull | null)[],
): string[] {
  return js.flatMap((j, i) =>
    j
      ? j.journal.map((r) => r.created_at)
      : (fs[i]?.items ?? []).map((it) => it.learned_at ?? '').filter(Boolean),
  )
}

export function dayKey(ts: number | string): string {
  return new Date(ts).toISOString().slice(0, 10)
}

export function buildHeatmap(dates: string[]): { cells: { key: string; count: number }[]; streak: number } {
  const counts = new Map<string, number>()
  for (const d of dates) {
    const k = dayKey(d)
    counts.set(k, (counts.get(k) ?? 0) + 1)
  }
  const today = new Date()
  today.setHours(12, 0, 0, 0) // 正午锚定，免夏令时偏移
  const cells: { key: string; count: number }[] = []
  for (let i = 83; i >= 0; i--) {
    const t = today.getTime() - i * DAY
    cells.push({ key: dayKey(t), count: counts.get(dayKey(t)) ?? 0 })
  }
  // 连击：从今天往回数；今天还没学则从昨天起算（连击不断）
  let streak = 0
  let start = counts.has(dayKey(today.getTime())) ? 0 : 1
  for (let i = start; i < 400; i++) {
    const k = dayKey(today.getTime() - i * DAY)
    if ((counts.get(k) ?? 0) > 0) streak++
    else break
  }
  return { cells, streak }
}

export function Heatmap({ cells }: { cells: { key: string; count: number }[] }) {
  const weeks: { key: string; count: number }[][] = []
  for (let i = 0; i < cells.length; i += 7) weeks.push(cells.slice(i, i + 7))
  const cls = (n: number) =>
    n === 0 ? 'bg-muted' : n === 1 ? 'bg-emerald-500/30' : n <= 3 ? 'bg-emerald-500/55' : 'bg-emerald-500'
  return (
    <div className="flex gap-[3px]" aria-label="近 12 周学习活动热力图">
      {weeks.map((w, wi) => (
        <div key={wi} className="flex flex-col gap-[3px]">
          {w.map((d) => (
            <div
              key={d.key}
              title={`${d.key} · ${d.count} 次`}
              className={`h-[10px] w-[10px] rounded-[2px] ${cls(d.count)}`}
            />
          ))}
        </div>
      ))}
    </div>
  )
}

// ---------- 总览页 ----------
export default function Study() {
  const nav = useNavigate()
  const [topics, setTopics] = useState<TopicBrief[] | null>(null)
  const [fulls, setFulls] = useState<TopicFull[]>([])
  const [due, setDue] = useState<StudyItem[]>([])
  const [journalDates, setJournalDates] = useState<string[]>([])
  const [err, setErr] = useState('')

  const load = () =>
    api
      .get<{ topics: TopicBrief[] }>('/study/topics')
      .then(async (v) => {
        setTopics(v.topics)
        // 每领域全量（进度环/已学日期）+ 日志（热力图数据源）——领域数少，N 个请求可接受
        const [fs, js] = await Promise.all([
          Promise.all(v.topics.map((t) => api.get<TopicFull>(`/study/topics/${t.id}`).catch(() => null))),
          Promise.all(
            v.topics.map((t) =>
              api
                .get<{ journal: StudyJournalRow[] }>(`/study/topics/${t.id}/journal`)
                .catch(() => null),
            ),
          ),
        ])
        setFulls(fs.filter((x): x is TopicFull => x !== null))
        // P019-M3：热力图日期源——journal 失败（null）回退到该领域已学节点的 learned_at，
        // 空值一律过滤（旧实现 learned_at ?? '' 产生空串，buildHeatmap→dayKey('')→
        // new Date('').toISOString() 抛 RangeError 整页崩溃；且 fs 未过滤，索引本就对齐）。
        setJournalDates(heatmapDatesFrom(js, fs))
      })
      .catch((e) => setErr(String(e)))

  useEffect(() => {
    load()
    api
      .get<{ reviews: StudyItem[] }>('/study/reviews')
      .then((v) => setDue(v.reviews))
      .catch(() => {})
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const { cells, streak } = useMemo(() => buildHeatmap(journalDates), [journalDates])
  const totalLearned = fulls.reduce((a, f) => a + f.progress.learned, 0)
  const totalNodes = fulls.reduce((a, f) => a + f.progress.total, 0)
  const overall = totalNodes ? Math.round((totalLearned / totalNodes) * 100) : 0
  // 到期复习按领域分组
  const dueByTrack = useMemo(() => {
    const m = new Map<string, StudyItem[]>()
    for (const it of due) {
      const arr = m.get(it.track_id) ?? []
      arr.push(it)
      m.set(it.track_id, arr)
    }
    return m
  }, [due])

  return (
    <div className="space-y-4">
      <PageHeader title="学习驾驶舱" />

      {/* 顶条：连击 + 热力图 + 总进度 */}
      <Card className="flex flex-wrap items-center gap-x-8 gap-y-3 px-5 py-4">
        <div className="flex items-center gap-2">
          <span className="text-2xl">🔥</span>
          <div>
            <div className="text-xl font-semibold leading-none">{streak}</div>
            <div className="text-xs text-muted-foreground">连续学习天数</div>
          </div>
        </div>
        <Heatmap cells={cells} />
        <div className="ml-auto flex items-center gap-3">
          <div className="text-right">
            <div className="text-xl font-semibold leading-none">{overall}%</div>
            <div className="text-xs text-muted-foreground">
              总进度 · {totalLearned}/{totalNodes} 节点
            </div>
          </div>
          <ProgressRing pct={overall} />
        </div>
      </Card>

      {err && <ErrorBox msg={err} />}

      {/* 今日到期复习（跨领域汇总） */}
      {due.length > 0 && (
        <Card className="p-4">
          <div className="mb-2 text-sm font-medium">
            📅 今日复习
            <span className="ml-2 rounded bg-amber-500/15 px-1.5 py-0.5 text-xs text-amber-600 dark:text-amber-400">
              {due.length}
            </span>
          </div>
          <div className="flex flex-wrap gap-2">
            {[...dueByTrack.entries()].map(([tid, items]) => {
              const t = topics?.find((x) => x.id === tid)
              return (
                <button
                  key={tid}
                  className="rounded-lg border border-border px-3 py-1.5 text-left text-sm hover:bg-muted/40"
                  onClick={() => nav(`/study/${tid}`)}
                >
                  <span className="font-medium">{t?.name ?? '未知领域'}</span>
                  <span className="ml-2 text-xs text-muted-foreground">{items.length} 个到期 →</span>
                </button>
              )
            })}
          </div>
        </Card>
      )}

      {/* 领域卡片 */}
      {topics === null ? (
        <Spinner />
      ) : topics.length === 0 ? (
        <Empty text="还没有学习领域——开个题，路线图从这里长出来" />
      ) : (
        <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
          {topics.map((t) => {
            const f = fulls.find((x) => x.id === t.id)
            const pct = f && f.progress.total ? Math.round((f.progress.learned / f.progress.total) * 100) : 0
            const dueN = dueByTrack.get(t.id)?.length ?? 0
            return (
              <button
                key={t.id}
                className="rounded-lg border border-border p-4 text-left transition-colors hover:bg-muted/40"
                onClick={() => nav(`/study/${t.id}`)}
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="truncate font-medium">{t.name}</span>
                      <StatusBadge status={t.status} />
                    </div>
                    {t.goal && <div className="mt-0.5 truncate text-xs text-muted-foreground">{t.goal}</div>}
                    <div className="mt-1.5 text-xs text-muted-foreground">
                      {f ? `${f.progress.learned}/${f.progress.total} 节点` : '…'}
                      {dueN > 0 && <span className="ml-2 text-amber-600 dark:text-amber-400">{dueN} 个待复习</span>}
                    </div>
                    <div className="mt-1 text-xs text-muted-foreground">更新于 {fmtTime(t.updated_at)}</div>
                  </div>
                  <div className="relative shrink-0">
                    <ProgressRing pct={pct} />
                    <span className="absolute inset-0 flex items-center justify-center text-xs font-medium">
                      {pct}%
                    </span>
                  </div>
                </div>
              </button>
            )
          })}
        </div>
      )}
    </div>
  )
}

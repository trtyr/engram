/** Dashboard：管线主视觉（L0→L3）+ 资产统计 + 用量趋势 + 近期活动。 */
import { useEffect, useMemo, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import {
  api,
  type Atom,
  type Document,
  type Job,
  type Persona,
  type Scenario,
  type Session,
  type SkillSummaryDto,
  type UsageRow,
  type WikiPage,
} from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner, StatusBadge } from '@/components/ui-bits'
import { relTime } from '@/lib/ui'
import { useSystemStatus } from '@/lib/status'
import { cn } from '@/lib/utils'

/** 近 7 天增量口径（锚点取页面加载时刻：分桶边界无需更细粒度，且避免渲染期调用 Date.now） */
const NOW = Date.now()
const WEEK_MS = 7 * 86_400_000
const withinWeek = (iso: string) => NOW - new Date(iso).getTime() < WEEK_MS

interface Stage {
  level: string
  tag: string
  label: string
  tab: 'sessions' | 'atoms' | 'scenarios' | 'persona'
  count: number
  delta: number
}

/** 管线主视觉：产品灵魂（L0 会话 → L3 画像）占据 C 位；点击穿透 Memory 对应 tab。 */
function PipelineHero({ stages, distilling }: { stages: Stage[]; distilling: number }) {
  const nav = useNavigate()
  return (
    <div className="flex flex-col gap-px overflow-hidden rounded-lg border border-border bg-border/60 md:flex-row">
      {stages.map((s, i) => (
        <div key={s.level} className="flex flex-1 contents md:contents">
          <button
            type="button"
            onClick={() => nav(`/memory?tab=${s.tab}`)}
            className="flex-1 bg-card p-4 text-left transition-colors hover:bg-muted/40 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
            aria-label={`${s.label} ${s.count}，跳转到记忆 ${s.label}`}
          >
            <p className="font-mono text-xs text-muted-foreground">
              {s.level} · {s.tag}
            </p>
            <p className="mt-1.5 font-mono text-3xl font-medium tracking-tight tabular-nums">
              {s.count.toLocaleString()}
            </p>
            <p className="mt-1 h-4 text-xs">
              {s.delta > 0 ? (
                <span className="text-success">+{s.delta} 近7天</span>
              ) : (
                <span className="text-muted-foreground/50">—</span>
              )}
            </p>
          </button>
          {i < stages.length - 1 && (
            <div
              className="hidden w-9 items-center justify-center bg-card font-mono text-sm text-muted-foreground md:flex"
              aria-hidden="true"
            >
              <span className={cn(distilling > 0 && 'engram-pulse')}>→</span>
            </div>
          )}
        </div>
      ))}
    </div>
  )
}

/** LLM 用量 30 天日分桶柱状图：灰阶 chart token，今日柱用前景墨；原生 title 悬停明细。 */
function UsageChart({ rows }: { rows: UsageRow[] }) {
  const days = useMemo(() => {
    const byDay = new Map<string, { tokens: number; calls: number }>()
    for (const u of rows) {
      const key = u.ts.slice(0, 10)
      const e = byDay.get(key) ?? { tokens: 0, calls: 0 }
      e.tokens += u.input_tokens + u.output_tokens
      e.calls += 1
      byDay.set(key, e)
    }
    return Array.from({ length: 30 }, (_, i) => {
      const dt = new Date(NOW - (29 - i) * 86_400_000)
      const key = dt.toISOString().slice(0, 10)
      return { key, label: `${dt.getMonth() + 1}/${dt.getDate()}`, ...(byDay.get(key) ?? { tokens: 0, calls: 0 }) }
    })
  }, [rows])
  const max = Math.max(...days.map((d) => d.tokens), 1)

  return (
    <div className="p-4">
      <div className="flex h-24 w-full items-end">
        <svg viewBox="0 0 240 96" preserveAspectRatio="none" className="h-full w-full" role="img" aria-label="近 30 天 LLM token 用量柱状图">
          {days.map((d, i) => {
            const h = d.tokens === 0 ? 2 : Math.max(3, Math.round((d.tokens / max) * 92))
            const isToday = i === days.length - 1
            return (
              <rect
                key={d.key}
                x={i * 8}
                y={96 - h}
                width={6}
                height={h}
                rx={1}
                fill={d.tokens === 0 ? 'var(--border)' : isToday ? 'var(--foreground)' : 'var(--chart-2)'}
              >
                <title>{`${d.label} · ${d.tokens.toLocaleString()} tokens · ${d.calls} 次调用`}</title>
              </rect>
            )
          })}
        </svg>
      </div>
      <div className="mt-2 flex justify-between font-mono text-xs text-muted-foreground/70">
        <span>{days[0].label}</span>
        <span>峰值 {max.toLocaleString()}/日</span>
        <span>{days[days.length - 1].label}</span>
      </div>
    </div>
  )
}

/** 近期活动 feed：状态点 + mono kind + 相对时间；失败行悬停可见错误。 */
function ActivityFeed({ jobs }: { jobs: Job[] }) {
  return (
    <ul className="divide-y divide-border/60">
      {jobs.map((j) => (
        <li key={j.id} title={j.error ?? undefined}>
          <Link to="/jobs" className="flex items-center gap-2.5 px-4 py-2.5 transition-colors hover:bg-muted/40">
            <StatusBadge status={j.status} />
            <span className="truncate font-mono text-xs text-muted-foreground">{j.kind}</span>
            <span className="ml-auto shrink-0 text-xs text-muted-foreground">{relTime(j.created_at)}</span>
          </Link>
        </li>
      ))}
    </ul>
  )
}

export default function Dashboard() {
  const [err, setErr] = useState('')
  const [core, setCore] = useState<{
    atoms: Atom[]
    sessions: Session[]
    scenarios: Scenario[]
    docs: Document[]
    pages: WikiPage[]
  } | null>(null)
  const [persona, setPersona] = useState<Persona[] | null>(null)
  const [cg, setCg] = useState<{ id: string }[] | null>(null)
  const [jobs, setJobs] = useState<Job[] | null>(null)
  const [usage, setUsage] = useState<UsageRow[] | null>(null)
  const [skills, setSkills] = useState<SkillSummaryDto[] | null>(null)
  const [openTodos, setOpenTodos] = useState<number | null>(null)
  // 复用侧栏轮询源（10s，页面隐藏自动跳过）：失败徽章 + 蒸馏脉冲与全局状态一致
  const { failed, distilling } = useSystemStatus()

  useEffect(() => {
    Promise.all([
      api.get<Atom[]>('/memory/atoms?limit=500'),
      api.get<Session[]>('/memory/sessions?limit=500'),
      api.get<Scenario[]>('/memory/scenarios?limit=500'),
      api.get<Document[]>('/wiki/documents?limit=200'),
      api.get<WikiPage[]>('/wiki/pages?limit=300'),
    ])
      .then(([atoms, sessions, scenarios, docs, pages]) => setCore({ atoms, sessions, scenarios, docs, pages }))
      .catch((e) => setErr(e instanceof Error ? e.message : String(e)))
    api.get<Persona[]>('/memory/persona').then(setPersona).catch(() => setPersona([]))
    api.get<{ id: string }[]>('/codegraph/projects').then(setCg).catch(() => setCg([]))
    api.get<Job[]>('/jobs?limit=8').then(setJobs).catch(() => setJobs([]))
    api.get<UsageRow[]>('/llm/usage').then(setUsage).catch(() => setUsage([]))
    api.get<SkillSummaryDto[]>('/skills').then(setSkills).catch(() => setSkills([]))
    api
      .get<{ status: string }[]>('/todos')
      .then((t) => setOpenTodos(t.filter((x) => x.status === 'open').length))
      .catch(() => setOpenTodos(0))
  }, [])

  if (err) return <ErrorBox msg={err} />
  if (!core) return <Spinner />

  const activeAtoms = core.atoms.filter((a) => a.status === 'active')
  const wikiPages = core.pages.filter((p) => p.page_type !== 'index' && p.page_type !== 'log')
  const readyDocs = core.docs.filter((d) => d.status === 'ready')
  const processingDocs = core.docs.filter((d) => !['ready', 'failed'].includes(d.status))
  const totalTokens = (usage ?? []).reduce((s, u) => s + u.input_tokens + u.output_tokens, 0)

  const stages: Stage[] = [
    {
      level: 'L0',
      tag: '会话',
      label: '会话',
      tab: 'sessions',
      count: core.sessions.length,
      delta: core.sessions.filter((s) => withinWeek(s.created_at)).length,
    },
    {
      level: 'L1',
      tag: '原子',
      label: '原子',
      tab: 'atoms',
      count: activeAtoms.length,
      delta: activeAtoms.filter((a) => withinWeek(a.created_at)).length,
    },
    {
      level: 'L2',
      tag: '场景',
      label: '场景',
      tab: 'scenarios',
      count: core.scenarios.length,
      delta: core.scenarios.filter((s) => withinWeek(s.updated_at)).length,
    },
    { level: 'L3', tag: '画像', label: '画像', tab: 'persona', count: persona?.length ?? 0, delta: 0 },
  ]

  const stats = [
    {
      label: '文档',
      n: readyDocs.length.toLocaleString(),
      sub: processingDocs.length > 0 ? `${processingDocs.length} 处理中` : `共 ${core.docs.length}`,
    },
    { label: 'Wiki 页面', n: wikiPages.length.toLocaleString(), sub: `共 ${core.pages.length}` },
    { label: '代码图谱项目', n: (cg?.length ?? 0).toLocaleString(), sub: '已注册' },
    {
      label: '技能',
      n: (skills?.length ?? 0).toLocaleString(),
      sub: `启用 ${skills?.filter((s) => s.enabled).length ?? 0}`,
    },
    {
      label: '待办（进行中）',
      n: (openTodos ?? 0).toLocaleString(),
      sub: 'open',
    },
    { label: 'LLM tokens', n: totalTokens.toLocaleString(), sub: '近 30 天' },
  ]

  const emptyWorld =
    core.sessions.length === 0 && activeAtoms.length === 0 && core.docs.length === 0 && wikiPages.length === 0

  return (
    <div className="space-y-6">
      <PageHeader title="概览" desc="记忆资产、系统活动与用量的概览" />

      <PipelineHero stages={stages} distilling={distilling} />

      {emptyWorld && (
        <p className="text-sm text-muted-foreground">
          还没有任何记忆资产——去<a className="underline underline-offset-4" href="#/wiki">知识</a>
          上传第一份文档，或在<a className="underline underline-offset-4" href="#/memory">记忆</a>写入第一条会话。
        </p>
      )}

      <div className="grid grid-cols-2 gap-px overflow-hidden rounded-lg border border-border bg-border/50 lg:grid-cols-6">
        {stats.map((s) => (
          <div key={s.label} className="bg-card p-4">
            <p className="font-mono text-2xl font-medium tracking-tight tabular-nums">{s.n}</p>
            <p className="mt-1 text-xs text-muted-foreground">
              {s.label}
              <span className="ml-1.5 text-muted-foreground/60">{s.sub}</span>
            </p>
          </div>
        ))}
      </div>

      <div className="grid grid-cols-1 gap-6 xl:grid-cols-3">
        <Card className="overflow-hidden xl:col-span-2">
          <div className="flex items-center justify-between border-b border-border px-4 py-3">
            <h2 className="text-sm font-medium">LLM 用量趋势</h2>
            <span className="font-mono text-xs text-muted-foreground">近 30 天</span>
          </div>
          {(usage ?? []).length === 0 ? (
            <div className="p-4">
              <Empty text="暂无用量" />
            </div>
          ) : (
            <UsageChart rows={usage ?? []} />
          )}
        </Card>

        <Card className="overflow-hidden">
          <div className="flex items-center justify-between border-b border-border px-4 py-3">
            <h2 className="text-sm font-medium">近期活动</h2>
            {failed > 0 ? (
              <Link
                to="/jobs"
                className="rounded border border-destructive/30 px-1.5 py-px font-mono text-xs text-destructive transition-colors hover:bg-destructive/10"
              >
                失败 {failed}
              </Link>
            ) : (
              <span className="font-mono text-xs text-muted-foreground">任务</span>
            )}
          </div>
          {jobs === null || jobs.length === 0 ? (
            <div className="p-4">
              <Empty text="暂无任务" />
            </div>
          ) : (
            <ActivityFeed jobs={jobs} />
          )}
        </Card>
      </div>
    </div>
  )
}

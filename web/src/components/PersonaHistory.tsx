import { useEffect, useMemo, useState } from 'react'
import { api, type Persona, type Scenario } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { relTime } from '@/lib/ui'

/**
 * 画像历史抽屉（P1 三件套之一，2026-08-31）：
 * - 版本列表搬出卡片——内联展开会把同行等高卡一起拉爆（用户截图实锤）
 * - 任选两版句子级 diff（P1.2）：增=success 高亮 / 删=destructive 删除线
 * - 证据链露脸（P1.3）：evidence_refs {atoms, sessions, scenarios} 计数
 *   + 场景标题按需加载、点击跳场景 tab
 * P2：列表自身 max-h 滚动（大版本数）、行内回滚、prompt_version、头部新鲜度。
 */

/** evidence_refs 真身（DB 验证 2026-08-31）：三组 id 数组，早期版本可为 []。 */
interface EvidenceRefs {
  atoms?: string[]
  sessions?: string[]
  scenarios?: string[]
}

function parseEv(raw: unknown): EvidenceRefs {
  if (raw && typeof raw === 'object') return raw as EvidenceRefs
  return {}
}

/** 句子切分：保留分隔符，中文标点 + 换行。 */
function splitSentences(text: string): string[] {
  const parts = text.split(/([。！？!?；;\n])/)
  const out: string[] = []
  for (let i = 0; i < parts.length; i += 2) {
    const s = (parts[i] ?? '') + (parts[i + 1] ?? '')
    if (s.trim()) out.push(s.trim())
  }
  return out
}

type DiffSeg = { text: string; kind: 'same' | 'add' | 'del' }

/** 句子级 LCS diff——画像句子数 <100，O(n·m) 足够，零依赖。 */
function diffSentences(a: string, b: string): DiffSeg[] {
  const A = splitSentences(a)
  const B = splitSentences(b)
  const n = A.length
  const m = B.length
  // dp[i][j] = A[i..] 与 B[j..] 的最长公共子序列长度
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0))
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = A[i] === B[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1])
    }
  }
  const segs: DiffSeg[] = []
  const push = (text: string, kind: DiffSeg['kind']) => {
    const last = segs[segs.length - 1]
    if (last && last.kind === kind) last.text += text
    else segs.push({ text, kind })
  }
  let i = 0
  let j = 0
  while (i < n && j < m) {
    if (A[i] === B[j]) {
      push(A[i], 'same')
      i++
      j++
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      push(A[i], 'del')
      i++
    } else {
      push(B[j], 'add')
      j++
    }
  }
  while (i < n) push(A[i++], 'del')
  while (j < m) push(B[j++], 'add')
  return segs
}

interface Props {
  aspect: string
  label: string
  onClose: () => void
  onGoScenario: () => void
  onMutated: () => void
}

export function PersonaHistoryDrawer({ aspect, label, onClose, onGoScenario, onMutated }: Props) {
  const [history, setHistory] = useState<Persona[] | null>(null)
  const [pickA, setPickA] = useState<number>(0) // 最新版（列表已按 version desc）
  const [pickB, setPickB] = useState<number>(1) // 次新版
  const [evOpen, setEvOpen] = useState(false)
  const [scenarios, setScenarios] = useState<Scenario[] | null>(null)

  // 挂载即取历史（父组件 key={aspect} 保证换分面重挂载，不在这里同步 reset）
  useEffect(() => {
    let alive = true
    api.get<Persona[]>(`/memory/persona/history?aspect=${aspect}`).then((h) => {
      if (!alive) return
      setHistory(h)
    })
    return () => {
      alive = false
    }
  }, [aspect])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  const verA = history?.[pickA]
  const verB = history?.[pickB]
  // 方向：a=基线(旧)=pickB，b=对比(新)=pickA——b-only=新增(绿)、a-only=删除(红)
  const diff = useMemo(
    () => (verA && verB && verA.version !== verB.version ? diffSentences(verB.content, verA.content) : null),
    [verA, verB],
  )

  const rollback = async (toVersion: number) => {
    if (!confirm(`回滚到 v${toVersion}？将以新版本号落地当前内容（历史不可变）。`)) return
    await api.post('/memory/persona/rollback', { aspect, to_version: toVersion })
    const h = await api.get<Persona[]>(`/memory/persona/history?aspect=${aspect}`)
    setHistory(h)
    setPickA(0)
    setPickB(h.length > 1 ? 1 : 0)
    onMutated()
  }

  const loadEvidence = async (ev: EvidenceRefs) => {
    setEvOpen((v) => !v)
    if (scenarios !== null || !ev.scenarios?.length) return
    const rows = await Promise.all(
      ev.scenarios.slice(0, 20).map((id) =>
        api.get<Scenario>(`/memory/scenarios/${id}`).catch(() => null),
      ),
    )
    setScenarios(rows.filter((s): s is Scenario => s !== null))
  }

  return (
    // 抽屉：右侧滑入，遮罩点击关闭；不进卡片流——卡片高度与历史长度解耦
    <div className="fixed inset-0 z-50 flex justify-end" role="dialog" aria-label={`${label} 历史`}>
      <div className="absolute inset-0 bg-foreground/20" onClick={onClose} aria-hidden />
      <aside className="relative flex h-full w-[480px] max-w-[92vw] flex-col border-l border-border bg-background shadow-lg">
        <header className="flex items-center justify-between gap-2 border-b border-border px-4 py-3">
          <div className="min-w-0">
            <h3 className="truncate font-medium">{label} · 历史</h3>
            <p className="font-mono text-xs text-muted-foreground">
              {aspect}
              {history ? ` · ${history.length} 版` : ''}
            </p>
          </div>
          <Button variant="ghost" size="sm" onClick={onClose} aria-label="关闭历史面板">
            关闭
          </Button>
        </header>

        {!history ? (
          <div className="flex-1 p-4 font-mono text-xs text-muted-foreground">加载历史…</div>
        ) : (
          <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4">
            {/* 版本对比（P1.2）：A=基线（旧），B=对比（新）。列表 desc：序号大=新。 */}
            <section>
              <div className="mb-2 flex items-center gap-2 text-xs text-muted-foreground">
                <span>对比</span>
                <select
                  aria-label="基线版本（旧）"
                  className="rounded-md border border-border bg-background px-1.5 py-0.5 text-xs"
                  value={pickB}
                  onChange={(e) => setPickB(Number(e.target.value))}
                >
                  {history.map((v, i) => (
                    <option key={v.id} value={i}>
                      v{v.version}（基线/旧）
                    </option>
                  ))}
                </select>
                <span>→</span>
                <select
                  aria-label="对比版本（新）"
                  className="rounded-md border border-border bg-background px-1.5 py-0.5 text-xs"
                  value={pickA}
                  onChange={(e) => setPickA(Number(e.target.value))}
                >
                  {history.map((v, i) => (
                    <option key={v.id} value={i}>
                      v{v.version}（对比/新）
                    </option>
                  ))}
                </select>
              </div>
              {diff ? (
                <div className="rounded-lg border border-border p-3 text-sm leading-relaxed">
                  <p className="mb-2 text-xs text-muted-foreground">
                    <span className="text-success">绿=新增</span> ·{' '}
                    <span className="text-destructive">红=删除</span> · 灰=未变
                  </p>
                  {diff.map((s, i) => (
                    <span
                      key={i}
                      className={cn(
                        s.kind === 'same' && 'text-muted-foreground',
                        s.kind === 'add' && 'bg-success/10 text-success',
                        s.kind === 'del' && 'bg-destructive/10 text-destructive line-through',
                      )}
                    >
                      {s.text}
                    </span>
                  ))}
                </div>
              ) : (
                <p className="text-sm text-muted-foreground">只有一个版本，无从对比。</p>
              )}
            </section>

            {/* 证据链（P1.3）：以「对比/新」选中版为准 */}
            {verA && (
              <section>
                <button
                  type="button"
                  onClick={() => loadEvidence(parseEv(verA.evidence_refs))}
                  className="flex w-full items-center gap-2 text-left"
                >
                  <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                    证据链 · v{verA.version}
                  </span>
                  <span className="font-mono text-xs text-muted-foreground">
                    {(() => {
                      const ev = parseEv(verA.evidence_refs)
                      return `${ev.atoms?.length ?? 0} 原子 · ${ev.sessions?.length ?? 0} 会话 · ${ev.scenarios?.length ?? 0} 场景`
                    })()}
                  </span>
                  <span className="ml-auto text-xs text-muted-foreground">{evOpen ? '收起' : '展开'}</span>
                </button>
                {evOpen && (
                  <div className="mt-2 space-y-1 rounded-lg border border-border p-3">
                    {(() => {
                      const ev = parseEv(verA.evidence_refs)
                      const total = (ev.atoms?.length ?? 0) + (ev.sessions?.length ?? 0) + (ev.scenarios?.length ?? 0)
                      if (total === 0)
                        return <p className="text-xs text-muted-foreground">该版本无证据引用（早期或手动写入）。</p>
                      return null
                    })()}
                    {scenarios === null ? (
                      (parseEv(verA.evidence_refs).scenarios?.length ?? 0) > 0 && (
                        <p className="font-mono text-xs text-muted-foreground">加载场景…</p>
                      )
                    ) : (
                      scenarios.map((s) => (
                        <button
                          key={s.id}
                          type="button"
                          onClick={() => {
                            onGoScenario()
                            onClose()
                          }}
                          className="block w-full truncate rounded px-1.5 py-1 text-left text-xs hover:bg-muted"
                          title="跳转到场景 tab"
                        >
                          <span className="font-mono text-muted-foreground">{relTime(s.updated_at)}</span>{' '}
                          {s.topic}
                        </button>
                      ))
                    )}
                  </div>
                )}
              </section>
            )}

            {/* 版本列表（P2.4 自滚动 + P2.5 行内回滚 + P2.6 prompt_version） */}
            <section className="flex min-h-0 flex-1 flex-col">
              <p className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                全部版本
              </p>
              <ul className="max-h-72 divide-y divide-border/60 overflow-y-auto rounded-lg border border-border">
                {history.map((v) => (
                  <li key={v.id} className="flex items-start gap-2 p-2.5">
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-1.5">
                        <span className="rounded border border-border px-1.5 py-px font-mono text-xs text-muted-foreground">
                          v{v.version}
                        </span>
                        <span className="text-xs text-muted-foreground">{relTime(v.created_at)}</span>
                        {v.prompt_version && (
                          <span
                            className="font-mono text-[10px] text-muted-foreground/60"
                            title="生成该版本的 prompt 版本"
                          >
                            prompt {v.prompt_version}
                          </span>
                        )}
                      </div>
                      <p className="mt-1 line-clamp-2 text-sm text-muted-foreground">{v.content}</p>
                    </div>
                    {pickA !== history.indexOf(v) && (
                      <Button
                        variant="ghost"
                        size="sm"
                        className="shrink-0"
                        onClick={() => rollback(v.version)}
                        title={`回滚到此版（以新版本号落地）`}
                      >
                        回滚
                      </Button>
                    )}
                  </li>
                ))}
              </ul>
            </section>
          </div>
        )}
      </aside>
    </div>
  )
}

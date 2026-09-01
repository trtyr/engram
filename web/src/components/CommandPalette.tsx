/**
 * 命令面板：Cmd/Ctrl+K 或 / 唤起的全局跨域检索（POST /search）。
 * 逻辑与 Dashboard GlobalSearch 同源（统一端点），呈现为居中覆盖层：
 * Esc 关闭、↑↓ 选择、Enter 跳转、点击命中项跳转。
 * 挂载即打开（父组件条件渲染），内部状态每次打开自然重置。
 */
import { useRef, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { api, type SearchResponse } from '@/lib/api'
import { cn } from '@/lib/utils'
import { inputCls } from '@/lib/ui'

const DOMAIN_LABEL: Record<string, string> = {
  entity: '实体',
  memory: '记忆',
  knowledge: '知识',
  wiki: 'Wiki',
}

function domainRoute(domain: string, id?: string): string {
  if (domain === 'entity') return id ? `/circle?entity=${id}` : '/circle'
  if (domain === 'memory') return '/memory?tab=atoms'
  if (domain === 'knowledge') return '/knowledge'
  if (domain === 'wiki') return '/wiki'
  return '/'
}

export function CommandPalette({ onClose }: { onClose: () => void }) {
  const nav = useNavigate()
  const [q, setQ] = useState('')
  const [hits, setHits] = useState<SearchResponse | null>(null)
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)
  const [sel, setSel] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)

  const run = async (query: string) => {
    const trimmed = query.trim()
    if (!trimmed || busy) return
    setBusy(true)
    setErr('')
    try {
      setHits(await api.post<SearchResponse>('/search', { query: trimmed, limit: 10 }))
      setSel(0)
    } catch (e) {
      setErr(e instanceof Error ? e.message : '检索失败')
    } finally {
      setBusy(false)
    }
  }

  const go = (i: number) => {
    const hit = hits?.hits[i]
    if (!hit) return
    onClose()
    nav(domainRoute(hit.domain, hit.id))
  }

  return (
    <div
      className="fixed inset-0 z-50 bg-background/70 backdrop-blur-[2px]"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
      aria-label="全局检索"
    >
      <div
        className="mx-auto mt-[12vh] w-[min(92vw,34rem)] rounded-lg border border-border bg-card"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="border-b border-border p-3">
          <input
            ref={inputRef}
            autoFocus
            className={`${inputCls} h-10`}
            placeholder="跨域检索：记忆 / 知识 / Wiki…"
            value={q}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault()
                run(q)
              } else if (e.key === 'Escape') {
                onClose()
              } else if (e.key === 'ArrowDown' && hits?.hits.length) {
                e.preventDefault()
                setSel((s) => Math.min(s + 1, hits.hits.length - 1))
              } else if (e.key === 'ArrowUp' && hits?.hits.length) {
                e.preventDefault()
                setSel((s) => Math.max(s - 1, 0))
              }
            }}
          />
          <p className="mt-2 px-1 font-mono text-[10px] text-muted-foreground/70">
            Enter 检索 · ↑↓ 选择 · Esc 关闭
          </p>
        </div>
        <div className="max-h-[50vh] overflow-auto p-2">
          {err && <p className="px-2 py-4 text-sm text-destructive">{err}</p>}
          {busy && <p className="px-2 py-4 text-sm text-muted-foreground">检索中…</p>}
          {!busy && !err && hits && hits.hits.length === 0 && (
            <p className="px-2 py-4 text-sm text-muted-foreground">无命中</p>
          )}
          {hits?.hits.map((h, i) => (
            <button
              key={`${h.domain}-${h.id}`}
              className={cn(
                'flex w-full flex-col items-start gap-1 rounded-md border border-transparent px-3 py-2 text-left transition-colors',
                i === sel ? 'border-border bg-muted' : 'hover:bg-muted/60',
              )}
              onMouseEnter={() => setSel(i)}
              onClick={() => go(i)}
            >
              <span className="flex w-full items-center gap-2">
                <span className="rounded border border-border px-1.5 py-px font-mono text-xs text-muted-foreground">
                  {DOMAIN_LABEL[h.domain] ?? h.domain}
                </span>
                <span className="truncate text-sm text-foreground">{h.title ?? h.id}</span>
                <span className="ml-auto font-mono text-xs text-muted-foreground/70">
                  {h.score.toFixed(2)}
                </span>
              </span>
              <span className="line-clamp-2 w-full text-xs text-muted-foreground">{h.snippet}</span>
            </button>
          ))}
        </div>
      </div>
    </div>
  )
}

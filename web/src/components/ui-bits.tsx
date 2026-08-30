/**
 * Engram 设计系统原语：墨白正统。
 * 分层 = 1px 发丝线（零阴影）；强调 = 墨色实心；彩色只承担语义。
 */
import type { ComponentProps, ReactNode } from 'react'
import { CircleAlert, Inbox, Loader2 } from 'lucide-react'
import { cn } from '@/lib/utils'

/** 品牌印记：三层错位方——记忆的层层留痕（L0→L2），顶层实心为「当下」。 */
export function BrandMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" fill="none" className={className} aria-hidden="true">
      <rect x="3.5" y="8.5" width="12" height="12" rx="2" stroke="currentColor" strokeWidth="1.5" opacity="0.45" />
      <rect x="7" y="5" width="12" height="12" rx="2" stroke="currentColor" strokeWidth="1.5" opacity="0.7" />
      <rect x="10.5" y="1.5" width="12" height="12" rx="2" fill="currentColor" />
    </svg>
  )
}

/** 卡片容器：发丝线分层，无阴影。 */
export function Card({ className, children, ...props }: ComponentProps<'div'>) {
  return (
    <div className={cn('rounded-lg border border-border bg-card', className)} {...props}>
      {children}
    </div>
  )
}

/** 区块标题行（卡片内的节标题）：字重层级，不靠字号。 */
export function SectionTitle({ className, children, ...props }: ComponentProps<'h2'>) {
  return (
    <h2 className={cn('px-4 py-2.5 text-sm font-semibold tracking-tight', className)} {...props}>
      {children}
    </h2>
  )
}

/** 页头：标题 + 一行副题；操作区右侧。 */
export function PageHeader({
  title,
  desc,
  children,
}: {
  title: string
  desc?: string
  children?: ReactNode
}) {
  return (
    <div className="flex items-start justify-between gap-4">
      <div>
        <h1 className="text-lg font-semibold tracking-tight">{title}</h1>
        {desc && <p className="mt-0.5 text-sm text-muted-foreground">{desc}</p>}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  )
}

/** 分段式 tab（aria-pressed 分段控件）：选中即整行反转（墨底白字），段间发丝分隔。
 * 计数/脉冲是视觉附加（aria-hidden）——button 的 accessible name 恒为裸 label。 */
export function Tabs<T extends string>({
  items,
  value,
  onChange,
}: {
  items: { value: T; label: string; count?: number; pulse?: boolean }[]
  value: T
  onChange: (v: T) => void
}) {
  return (
    <div className="inline-flex max-w-full flex-wrap items-stretch rounded-md border border-border">
      {items.map((it, i) => (
        <button
          key={it.value}
          type="button"
          aria-pressed={value === it.value}
          aria-label={it.label}
          onClick={() => onChange(it.value)}
          className={cn(
            'px-3 py-1.5 text-sm font-medium transition-colors',
            i > 0 && 'border-l border-border',
            value === it.value
              ? 'bg-foreground text-background'
              : 'text-muted-foreground hover:bg-muted hover:text-foreground',
          )}
        >
          {it.label}
          {it.count !== undefined && (
            <span aria-hidden="true" className={cn('ml-1.5 font-mono text-xs tabular-nums', value === it.value ? 'text-background/70' : 'text-muted-foreground/70')}>
              {it.count}
            </span>
          )}
          {it.pulse && (
            <span aria-hidden="true" className="ml-1.5 inline-block size-1.5 translate-y-px rounded-full bg-info engram-pulse" />
          )}
        </button>
      ))}
    </div>
  )
}

/** 状态 → 语义色映射（彩色只在此处出现）。 */
const STATUS_STYLE: Record<string, { dot: string; text: string; pulse?: boolean }> = {
  ready: { dot: 'bg-success', text: 'text-success' },
  succeeded: { dot: 'bg-success', text: 'text-success' },
  done: { dot: 'bg-success', text: 'text-success' },
  active: { dot: 'bg-success', text: 'text-success' },
  human: { dot: 'bg-success', text: 'text-success' },
  pending: { dot: 'bg-muted-foreground/60', text: 'text-muted-foreground' },
  processing: { dot: 'bg-info', text: 'text-info', pulse: true },
  parsing: { dot: 'bg-info', text: 'text-info', pulse: true },
  chunking: { dot: 'bg-info', text: 'text-info', pulse: true },
  embedding: { dot: 'bg-info', text: 'text-info', pulse: true },
  running: { dot: 'bg-info', text: 'text-info', pulse: true },
  indexing: { dot: 'bg-info', text: 'text-info', pulse: true },
  registered: { dot: 'bg-muted-foreground/60', text: 'text-muted-foreground' },
  failed: { dot: 'bg-destructive', text: 'text-destructive' },
  dead: { dot: 'bg-destructive', text: 'text-destructive' },
  error: { dot: 'bg-destructive', text: 'text-destructive' },
  superseded: { dot: 'bg-muted-foreground/60', text: 'text-muted-foreground' },
  archived: { dot: 'bg-muted-foreground/60', text: 'text-muted-foreground' },
  candidate: { dot: 'bg-info', text: 'text-info' },
  version_mismatch: { dot: 'bg-warning', text: 'text-warning' },
}

/** 状态徽章：无底色，色点 + mono 状态字——状态是数据。 */
export function StatusBadge({ status }: { status: string }) {
  const s = STATUS_STYLE[status] ?? { dot: 'bg-muted-foreground/60', text: 'text-muted-foreground' }
  return (
    <span className={cn('inline-flex items-center gap-1.5 whitespace-nowrap font-mono text-xs', s.text)}>
      <span className={cn('size-1.5 rounded-full', s.dot, s.pulse && 'engram-pulse')} />
      {status}
    </span>
  )
}

export function Empty({ text }: { text: string }) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 rounded-lg border border-dashed border-border py-12 text-center">
      <Inbox className="size-5 text-muted-foreground/50" />
      <p className="text-sm text-muted-foreground">{text}</p>
    </div>
  )
}

export function ErrorBox({ msg }: { msg: string }) {
  return (
    <div
      role="alert"
      className="flex items-start gap-2 rounded-lg border border-destructive/30 bg-destructive/5 px-3.5 py-3 text-sm text-destructive"
    >
      <CircleAlert className="mt-0.5 size-4 shrink-0" />
      <span>{msg}</span>
    </div>
  )
}

export function Spinner({ label = '加载中…' }: { label?: string }) {
  return (
    <div className="flex items-center justify-center gap-2 py-12 text-sm text-muted-foreground" aria-live="polite">
      <Loader2 className="size-4 animate-spin" />
      {label}
    </div>
  )
}

/**
 * 共享 UI 基元组件：品牌标记、卡片、页头、分段 tab、状态徽章、空态/错误态/加载态。
 * 样式常量与工具（fmtTime / tableCls / inputCls / selectCls）见 @/lib/ui。
 */
import type { ComponentProps, ReactNode } from 'react'
import { CircleAlert, Inbox, Loader2 } from 'lucide-react'
import { cn } from '@/lib/utils'

/** 品牌标记：一颗记忆节点连着它的关联（四个记忆资产各就其位）。 */
export function BrandMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 44 44" fill="none" aria-hidden="true" className={cn('size-6', className)}>
      <g stroke="var(--brand-strong)" strokeWidth="1.5" strokeLinecap="round" opacity="0.45">
        <path d="M22 20 13 12" />
        <path d="M22 20 31 12" />
        <path d="M22 20 22 34" />
      </g>
      <circle cx="22" cy="20" r="4.5" fill="var(--brand-strong)" />
      <circle cx="13" cy="12" r="2.5" fill="var(--brand-strong)" opacity="0.7" />
      <circle cx="31" cy="12" r="2.5" fill="var(--brand-strong)" opacity="0.7" />
      <circle cx="22" cy="34" r="2.5" fill="var(--brand-strong)" opacity="0.7" />
    </svg>
  )
}

/** 卡片容器：统一圆角、边框、底色。 */
export function Card({ className, children, ...props }: ComponentProps<'div'>) {
  return (
    <div className={cn('rounded-xl border border-border bg-card', className)} {...props}>
      {children}
    </div>
  )
}

/** 页头：标题 + 描述 + 右侧操作区。 */
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
        <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
        {desc && <p className="mt-1 text-sm text-muted-foreground">{desc}</p>}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  )
}

/** 分段式 tab 控件。 */
export function Tabs<T extends string>({
  items,
  value,
  onChange,
}: {
  items: { value: T; label: string }[]
  value: T
  onChange: (v: T) => void
}) {
  return (
    <div className="inline-flex max-w-full flex-wrap items-center gap-1 rounded-lg border border-border bg-muted/30 p-1">
      {items.map((it) => (
        <button
          key={it.value}
          type="button"
          onClick={() => onChange(it.value)}
          className={cn(
            'rounded-md px-3 py-1.5 text-sm font-medium transition-colors',
            value === it.value
              ? 'bg-card text-foreground shadow-sm ring-1 ring-border'
              : 'text-muted-foreground hover:text-foreground',
          )}
        >
          {it.label}
        </button>
      ))}
    </div>
  )
}

const STATUS_STYLE: Record<string, { dot: string; text: string }> = {
  ready: { dot: 'bg-green-400', text: 'text-green-400' },
  succeeded: { dot: 'bg-green-400', text: 'text-green-400' },
  active: { dot: 'bg-green-400', text: 'text-green-400' },
  pending: { dot: 'bg-yellow-400', text: 'text-yellow-400' },
  processing: { dot: 'bg-blue-400', text: 'text-blue-400' },
  parsing: { dot: 'bg-blue-400', text: 'text-blue-400' },
  chunking: { dot: 'bg-blue-400', text: 'text-blue-400' },
  embedding: { dot: 'bg-blue-400', text: 'text-blue-400' },
  running: { dot: 'bg-blue-400', text: 'text-blue-400' },
  indexing: { dot: 'bg-blue-400', text: 'text-blue-400' },
  failed: { dot: 'bg-red-400', text: 'text-red-400' },
  dead: { dot: 'bg-red-400', text: 'text-red-400' },
  error: { dot: 'bg-red-400', text: 'text-red-400' },
  superseded: { dot: 'bg-gray-400', text: 'text-gray-400' },
  archived: { dot: 'bg-gray-400', text: 'text-gray-400' },
  candidate: { dot: 'bg-purple-400', text: 'text-purple-400' },
  version_mismatch: { dot: 'bg-orange-400', text: 'text-orange-400' },
}

export function StatusBadge({ status }: { status: string }) {
  const s = STATUS_STYLE[status] ?? { dot: 'bg-gray-400', text: 'text-gray-400' }
  return (
    <span className="inline-flex items-center gap-1.5 whitespace-nowrap rounded-full border border-white/5 bg-white/[0.03] px-2 py-0.5 text-[11px] font-medium">
      <span className={`size-1.5 rounded-full ${s.dot}`} />
      <span className={s.text}>{status}</span>
    </span>
  )
}

export function Empty({ text }: { text: string }) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 rounded-xl border border-dashed border-border py-12 text-center">
      <Inbox className="size-5 text-muted-foreground/60" />
      <p className="text-sm text-muted-foreground">{text}</p>
    </div>
  )
}

export function ErrorBox({ msg }: { msg: string }) {
  return (
    <div className="flex items-start gap-2 rounded-lg border border-red-500/30 bg-red-500/10 px-3.5 py-3 text-sm text-red-400">
      <CircleAlert className="mt-0.5 size-4 shrink-0" />
      <span>{msg}</span>
    </div>
  )
}

export function Spinner({ label = '加载中…' }: { label?: string }) {
  return (
    <div className="flex items-center justify-center gap-2 py-12 text-sm text-muted-foreground">
      <Loader2 className="size-4 animate-spin" />
      {label}
    </div>
  )
}
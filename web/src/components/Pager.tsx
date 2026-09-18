/** 通用翻页条：共 N 条 · 每页条数 · 页码直跳 + 前后翻。 */
import { ChevronLeft, ChevronRight } from 'lucide-react'
import { cn } from '@/lib/utils'
import { selectCls } from '@/lib/ui'

export default function Pager({
  total,
  page,
  pageSize,
  onPage,
  onPageSize,
  hint,
}: {
  total: number
  page: number
  pageSize: number
  onPage: (p: number) => void
  onPageSize: (n: number) => void
  hint?: string
}) {
  const pages = Math.max(1, Math.ceil(total / pageSize))
  return (
    <div className="flex flex-wrap items-center justify-end gap-3 pt-1 text-xs text-muted-foreground">
      <span>共 {total} 条{hint ? `（${hint}）` : ''}</span>
      <label className="flex items-center gap-1">
        每页
        <select
          className={cn(selectCls, 'h-7 w-16 py-0 text-xs')}
          value={pageSize}
          onChange={(e) => onPageSize(Number(e.target.value))}
          aria-label="每页条数"
        >
          {[10, 20, 50, 100].map((n) => (
            <option key={n} value={n}>
              {n}
            </option>
          ))}
        </select>
        条
      </label>
      <div className="flex items-center gap-1">
        <button
          type="button"
          aria-label="上一页"
          disabled={page <= 1}
          onClick={() => onPage(page - 1)}
          className="rounded p-1 transition-colors hover:bg-muted disabled:opacity-40"
        >
          <ChevronLeft className="size-4" aria-hidden="true" />
        </button>
        {Array.from({ length: pages }, (_, i) => i + 1).map((p) => (
          <button
            key={p}
            type="button"
            aria-current={p === page ? 'page' : undefined}
            onClick={() => onPage(p)}
            className={cn(
              'min-w-7 rounded px-1.5 py-1 font-mono transition-colors',
              p === page ? 'bg-foreground text-background' : 'hover:bg-muted',
            )}
          >
            {p}
          </button>
        ))}
        <button
          type="button"
          aria-label="下一页"
          disabled={page >= pages}
          onClick={() => onPage(page + 1)}
          className="rounded p-1 transition-colors hover:bg-muted disabled:opacity-40"
        >
          <ChevronRight className="size-4" aria-hidden="true" />
        </button>
      </div>
    </div>
  )
}

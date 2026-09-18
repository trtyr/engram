/** 等高卡片正文截断器：固定最大高度 + 溢出渐隐 +「展示全文」弹窗（WikiMarkdown 渲染，markdown/mermaid 通吃）。
 *  用途：画像/场景/摘要等长文卡片网格——同一行卡片高度不再被内容撑爆。 */
import { useLayoutEffect, useRef, useState } from 'react'
import WikiMarkdown from '@/components/WikiMarkdown'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

export default function ClampText({
  content,
  title,
  meta,
  maxHeight = 168,
  className,
}: {
  content: string
  /** 展示全文弹窗的标题 */
  title: string
  /** 弹窗标题下的元信息行（版本/时间等） */
  meta?: string
  /** 正文区最大高度（px），默认 168 ≈ 7 行 */
  maxHeight?: number
  className?: string
}) {
  const boxRef = useRef<HTMLDivElement>(null)
  const [clamped, setClamped] = useState(false)
  const [open, setOpen] = useState(false)

  // 溢出检测：内容高于 max-height 才出「展示全文」；窗口缩放/内容变化重测
  useLayoutEffect(() => {
    const el = boxRef.current
    if (!el) return
    const check = () => setClamped(el.scrollHeight > el.clientHeight + 1)
    check()
    const ro = new ResizeObserver(check)
    ro.observe(el)
    return () => ro.disconnect()
  }, [content, maxHeight])

  return (
    <div className={cn('mt-2 flex-1', className)}>
      <div
        ref={boxRef}
        className="relative overflow-hidden"
        style={{ maxHeight }}
      >
        <div className="prose-invert text-sm leading-relaxed text-muted-foreground [&_p]:my-1.5">
          <WikiMarkdown content={content} />
        </div>
        {clamped && !open && (
          <div className="pointer-events-none absolute inset-x-0 bottom-0 h-10 bg-gradient-to-t from-card to-transparent" />
        )}
      </div>
      {clamped && (
        <Button variant="ghost" size="sm" className="mt-1 h-7 px-2 text-xs text-muted-foreground" onClick={() => setOpen(true)}>
          展示全文
        </Button>
      )}
      {open && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
          role="dialog"
          aria-label={`${title} 全文`}
          onClick={() => setOpen(false)}
        >
          <div
            className="flex max-h-[85vh] w-full max-w-3xl flex-col overflow-hidden rounded-lg border border-border bg-card shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex shrink-0 items-center justify-between border-b border-border px-4 py-3">
              <div>
                <h3 className="text-sm font-semibold">{title}</h3>
                {meta && <p className="mt-0.5 font-mono text-xs text-muted-foreground">{meta}</p>}
              </div>
              <Button variant="ghost" size="sm" onClick={() => setOpen(false)}>
                关闭
              </Button>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
              <div className="text-sm leading-relaxed text-foreground/90 [&_p]:my-2">
                <WikiMarkdown content={content} />
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

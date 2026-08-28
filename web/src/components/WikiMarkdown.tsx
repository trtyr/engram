/**
 * Wiki Markdown 渲染：mermaid 代码块 + [[wikilink]] 页内跳转。
 */
import { memo, useEffect, useState } from 'react'
import ReactMarkdown from 'react-markdown'
import { useNavigate, useSearchParams } from 'react-router-dom'

/** mermaid 惰性获取（模块副作用最小化，测试环境可安全 mock）。 */
async function renderMermaid(id: string, code: string): Promise<string> {
  const m = (await import('mermaid')) as unknown as {
    default?: { initialize(o: object): void; render(i: string, c: string): Promise<{ svg: string }> }
    mermaid?: { initialize(o: object): void; render(i: string, c: string): Promise<{ svg: string }> }
    initialize?(o: object): void
    render?(i: string, c: string): Promise<{ svg: string }>
  }
  const inst = m.mermaid ?? m.default ?? (m as { initialize(o: object): void; render(i: string, c: string): Promise<{ svg: string }> })
  inst.initialize({ startOnLoad: false, theme: 'dark' })
  const r = await inst.render(id, code)
  return r.svg
}

/** [[slug]] → 可点击跳转链接（点击后通知 Wiki 页加载对应页面）。 */
function WikilinkText({ text, onNavigate }: { text: string; onNavigate: (slug: string) => void }) {
  const parts = text.split(/(\[\[[^\]]+\]\])/g)
  return (
    <>
      {parts.map((p, i) => {
        const m = p.match(/^\[\[([^\]|]+)(\|[^\]]+)?\]\]$/)
        if (m) {
          const slug = m[1]
          const label = (m[2] ?? '').replace('|', '') || slug
          return (
            <button
              key={i}
              className="text-brand-strong underline-offset-2 hover:underline"
              onClick={(e) => {
                e.preventDefault()
                e.stopPropagation()
                onNavigate(slug)
              }}
            >
              {label}
            </button>
          )
        }
        return <span key={i}>{p}</span>
      })}
    </>
  )
}

function MermaidBlock({ code }: { code: string }) {
  const [id] = useState(() => `mmd-${Math.random().toString(36).slice(2)}`)
  const [svg, setSvg] = useState('')
  const [err, setErr] = useState('')
  useEffect(() => {
    let cancelled = false
    renderMermaid(id, code)
      .then((svg) => !cancelled && setSvg(svg))
      .catch(() => !cancelled && setErr('mermaid 渲染失败'))
    return () => {
      cancelled = true
    }
  }, [code, id])
  if (err) return <pre className="rounded bg-muted/50 p-2 text-xs text-red-400">{err}</pre>
  if (!svg) return <pre className="rounded bg-muted/50 p-2 text-xs">{code}</pre>
  return <div className="my-2 overflow-auto" dangerouslySetInnerHTML={{ __html: svg }} />
}

const WikiMarkdown = memo(function WikiMarkdown({
  content,
  onNavigateSlug,
}: {
  content: string
  onNavigateSlug: (slug: string) => void
}) {
  const [, setSearch] = useSearchParams()
  const nav = useNavigate()
  const goto = (slug: string) => {
    setSearch({ page: slug }, { replace: false })
    onNavigateSlug(slug)
    nav(`/wiki?page=${encodeURIComponent(slug)}`)
  }
  return (
    <article className="prose prose-sm prose-invert max-w-none">
      <ReactMarkdown
        components={{
          code({ className, children, ...props }) {
            const txt = String(children ?? '')
            if (/language-mermaid/.test(className ?? '')) return <MermaidBlock code={txt} />
            return (
              <code className={className} {...props}>
                {txt}
              </code>
            )
          },
          p({ children, ...props }) {
            return (
              <p {...props}>
                {typeof children === 'string' ? <WikilinkText text={children} onNavigate={goto} /> : children}
              </p>
            )
          },
        }}
      >
        {content}
      </ReactMarkdown>
    </article>
  )
})

export default WikiMarkdown

/**
 * Wiki Markdown 渲染（Engram 排版）：mermaid 代码块 + [[wikilink]] 页内跳转。
 * - 正文行长 70ch（可读性）；标题/表格/代码/引用走 Engram 发丝线语言
 * - mermaid 主题随当前 light/dark token 注入（theme:'base' + themeVariables），字体统一 Geist
 */
import { memo, useEffect, useState } from 'react'
import ReactMarkdown from 'react-markdown'
import { useNavigate, useSearchParams } from 'react-router-dom'
import { useThemeTick } from '@/lib/theme'

/** 当前主题（供 mermaid 注入）。 */
function isDark() {
  return document.documentElement.classList.contains('dark')
}

/** mermaid 惰性获取；主题变量从 token 取，亮暗随切。 */
async function renderMermaid(id: string, code: string): Promise<string> {
  const m = (await import('mermaid')) as unknown as {
    default?: { initialize(o: object): void; render(i: string, c: string): Promise<{ svg: string }> }
    mermaid?: { initialize(o: object): void; render(i: string, c: string): Promise<{ svg: string }> }
    initialize?(o: object): void
    render?(i: string, c: string): Promise<{ svg: string }>
  }
  const inst = m.mermaid ?? m.default ?? (m as { initialize(o: object): void; render(i: string, c: string): Promise<{ svg: string }> })
  const dark = isDark()
  const css = getComputedStyle(document.documentElement)
  const v = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback
  inst.initialize({
    startOnLoad: false,
    theme: 'base',
    fontFamily: "'Geist Variable', 'Geist Mono Variable', sans-serif",
    themeVariables: {
      background: v('--card', dark ? '#111' : '#fff'),
      primaryColor: v('--muted', dark ? '#1a1a1a' : '#f4f4f4'),
      primaryTextColor: v('--foreground', dark ? '#ededed' : '#0a0a0a'),
      primaryBorderColor: v('--border', dark ? '#262626' : '#e5e5e5'),
      lineColor: v('--muted-foreground', dark ? '#909094' : '#636365'),
      textColor: v('--foreground', dark ? '#ededed' : '#0a0a0a'),
      mainBkg: v('--card', dark ? '#111' : '#fff'),
      nodeBorder: v('--border', dark ? '#262626' : '#e5e5e5'),
      clusterBkg: v('--muted', dark ? '#1a1a1a' : '#f4f4f4'),
      edgeLabelBackground: v('--card', dark ? '#111' : '#fff'),
    },
  })
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
              className="font-medium underline underline-offset-4 hover:opacity-70"
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
  const [err, setErr] = useState(false)
  const [attempt, setAttempt] = useState(0)
  const themeTick = useThemeTick()
  useEffect(() => {
    let cancelled = false
    renderMermaid(`${id}-${attempt}-${themeTick}`, code)
      .then((s) => !cancelled && setSvg(s))
      .catch(() => !cancelled && setErr(true))
    return () => {
      cancelled = true
    }
  }, [code, id, attempt, themeTick])
  if (err) {
    return (
      <div className="my-3 flex items-center justify-between gap-3 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2.5 text-sm text-destructive">
        <span>mermaid 渲染失败</span>
        <button
          type="button"
          className="font-medium underline underline-offset-4"
          onClick={() => {
            setSvg('')
            setErr(false)
            setAttempt((a) => a + 1)
          }}
        >
          重试
        </button>
      </div>
    )
  }
  if (!svg) {
    // 渲染中：骨架占位（高度先占住，避免布局跳变）
    return <div className="my-3 h-40 animate-pulse rounded-md border border-border bg-muted/50" aria-label="mermaid 渲染中" />
  }
  return <div className="my-3 overflow-x-auto" dangerouslySetInnerHTML={{ __html: svg }} />
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
    <article className="engram-prose max-w-[70ch]">
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

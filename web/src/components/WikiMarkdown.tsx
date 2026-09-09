/**
 * Markdown 全站渲染器（Engram 排版）：全量 markdown + mermaid 图 + 代码块 + 表格/引用，
 * Wiki 场景额外支持 [[wikilink]] 页内跳转。
 * - onNavigateSlug 可选：不传（项目 docs / 技能正文 / 附属文件等非 Wiki 场景）
 *   时 wikilink 退化为纯文本样式
 * - 正文行长 70ch（可读性）；标题/表格/代码/引用走 Engram 发丝线语言
 * - mermaid 主题随当前 light/dark token 注入（theme:'base' + themeVariables），字体统一 Geist
 */
import { Fragment, cloneElement, isValidElement, memo, useEffect, useState } from 'react'
import type { ReactElement, ReactNode } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
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

/**
 * 递归把 children 树里字符串节点中的 [[wikilink]] 换成可点按钮。
 * 段落只要混入一点行内格式（`code`、**粗体**、[链接]），react-markdown 就会把
 * children 拆成「字符串+元素」数组，所以必须递归深入而不是只看顶层是否为 string。
 * code/pre 内保持字面（代码里的 [[ 不是链接）。
 */
function withWikilinks(node: ReactNode, onNavigate: (slug: string) => void): ReactNode {
  if (typeof node === 'string') {
    // 快速路径：不含 [[ 的纯文本原样返回（避免无谓的 span 包裹，保持语义父级的直接子节点）
    if (!node.includes('[[')) return node
    return <WikilinkText text={node} onNavigate={onNavigate} />
  }
  if (Array.isArray(node)) {
    return node.map((c, i) => (
      <Fragment key={i}>{withWikilinks(c, onNavigate)}</Fragment>
    ))
  }
  if (isValidElement(node)) {
    const el = node as ReactElement<{ children?: ReactNode }>
    if (el.type === 'code' || el.type === 'pre') return node
    return cloneElement(el, undefined, withWikilinks(el.props.children, onNavigate))
  }
  return node
}

/** 从 react-markdown 10 的 Hast node 提取纯文本（text 节点的 value 拼接）。 */
function hastText(node: unknown): string {
  if (!node || typeof node !== 'object') return ''
  const n = node as { value?: unknown; children?: unknown[] }
  if (typeof n.value === 'string') return n.value
  if (Array.isArray(n.children)) return n.children.map(hastText).join('')
  return ''
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
  /** Wiki 场景：点击 [[wikilink]] 跳转页面；不传则 wikilink 退化为纯文本（非 Wiki 场景） */
  onNavigateSlug?: (slug: string) => void
}) {
  const [, setSearch] = useSearchParams()
  const nav = useNavigate()
  const goto = (slug: string) => {
    if (!onNavigateSlug) return
    setSearch({ page: slug }, { replace: false })
    onNavigateSlug(slug)
    nav(`/wiki?page=${encodeURIComponent(slug)}`)
  }
  return (
    <article className="engram-prose">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          code({ className, node, ...props }) {
            const txt = hastText(node)
            if (/language-mermaid/.test(className ?? '')) return <MermaidBlock code={txt} />
            // 行内 code（非 ``` 代码块）：长 token 允许任意断行，防止把 flex 链顶出横向滚动
            const isBlock = /language-/.test(className ?? '')
            return (
              <code className={isBlock ? className : 'break-all'} {...props}>
                {txt}
              </code>
            )
          },
          p({ children, node, ...props }) {
            return <p {...props}>{withWikilinks(children, goto)}</p>
          },
          h1({ children, node, ...props }) {
            return <h1 {...props}>{withWikilinks(children, goto)}</h1>
          },
          h2({ children, node, ...props }) {
            return <h2 {...props}>{withWikilinks(children, goto)}</h2>
          },
          h3({ children, node, ...props }) {
            return <h3 {...props}>{withWikilinks(children, goto)}</h3>
          },
          h4({ children, node, ...props }) {
            return <h4 {...props}>{withWikilinks(children, goto)}</h4>
          },
          h5({ children, node, ...props }) {
            return <h5 {...props}>{withWikilinks(children, goto)}</h5>
          },
          h6({ children, node, ...props }) {
            return <h6 {...props}>{withWikilinks(children, goto)}</h6>
          },
          li({ children, node, ...props }) {
            return <li {...props}>{withWikilinks(children, goto)}</li>
          },
          blockquote({ children, node, ...props }) {
            return <blockquote {...props}>{withWikilinks(children, goto)}</blockquote>
          },
          table({ children, node, ...props }) {
            return (
              <div className="w-full overflow-x-auto [scrollbar-gutter:stable]">
                <table className="w-full" {...props}>{withWikilinks(children, goto)}</table>
              </div>
            )
          },
          td({ children, node, ...props }) {
            return <td {...props}>{withWikilinks(children, goto)}</td>
          },
          th({ children, node, ...props }) {
            return <th {...props}>{withWikilinks(children, goto)}</th>
          },
        }}
      >
        {content}
      </ReactMarkdown>
    </article>
  )
})

export default WikiMarkdown

/**
 * 图洞察面板（llm_wiki 对齐）：意外连接 / 孤立页 / 稀疏社区 / 桥节点。
 * 点击卡片高亮对应节点（联动 WikiGraph）；可忽略。
 */
import { useEffect, useState } from 'react'
import { api } from '@/lib/api'
import { Card, Empty, ErrorBox, Spinner } from '@/components/ui-bits'
import { Button } from '@/components/ui/button'

interface Insight {
  key: string
  kind: string
  title: string
  detail: string
  slugs: string[]
  search_queries: string[]
}

interface InsightsReport {
  insights: Insight[]
  communities: { id: number; top_slug: string; size: number; cohesion: number }[]
  total_pages: number
}

const KIND_LABEL: Record<string, string> = {
  surprising_connection: '意外连接',
  isolated_page: '孤立页面',
  sparse_community: '稀疏社区',
  bridge_node: '桥节点',
}

const KIND_STYLE: Record<string, { dot: string; text: string }> = {
  surprising_connection: { dot: 'bg-info', text: 'text-info' },
  isolated_page: { dot: 'bg-warning', text: 'text-warning' },
  sparse_community: { dot: 'bg-warning', text: 'text-warning' },
  bridge_node: { dot: 'bg-info', text: 'text-info' },
}

export default function InsightsPanel({
  onHighlight,
}: {
  /** 点击洞察卡片 → 高亮对应图谱节点（传 slug 集） */
  onHighlight: (slugs: string[] | null) => void
}) {
  const [report, setReport] = useState<InsightsReport | null>(null)
  const [err, setErr] = useState('')
  const [active, setActive] = useState<string | null>(null)

  const load = () => api.post<InsightsReport>('/wiki/insights').then(setReport).catch((e) => setErr(e.message))
  useEffect(() => {
    load()
  }, [])

  if (err) return <ErrorBox msg={err} />
  if (!report) return <Spinner label="洞察计算中…" />

  return (
    <div className="space-y-3" data-testid="insights-panel">
      <div className="flex items-center justify-between">
        <p className="text-sm text-muted-foreground">
          {report.total_pages} 页 · {report.insights.length} 条洞察 ·{' '}
          {report.communities.length} 个社区
        </p>
        <Button
          size="sm"
          variant="outline"
          onClick={async () => {
            await api.post('/wiki/insights/reset')
            load()
          }}
        >
          重置忽略
        </Button>
      </div>

      {report.insights.length === 0 ? (
        <Empty text="无洞察（知识库连接良好或全部已忽略）" />
      ) : (
        <div className="space-y-2">
          {report.insights.map((ins) => {
            const s = KIND_STYLE[ins.kind] ?? { dot: 'bg-gray-400', text: 'text-gray-400' }
            return (
              <Card
                key={ins.key}
                className={`cursor-pointer p-3 transition-colors ${active === ins.key ? 'border-brand/50 bg-brand/5' : 'hover:bg-muted/30'}`}
                onClick={() => {
                  if (active === ins.key) {
                    setActive(null)
                    onHighlight(null)
                  } else {
                    setActive(ins.key)
                    onHighlight(ins.slugs)
                  }
                }}
                data-testid={`insight-${ins.kind}`}
              >
                <div className="flex items-start justify-between gap-2">
                  <div>
                    <span className="inline-flex items-center gap-1.5 rounded border border-border px-1.5 py-px font-mono text-xs">
                      <span className={`size-1.5 rounded-full ${s.dot}`} />
                      <span className={s.text}>{KIND_LABEL[ins.kind] ?? ins.kind}</span>
                    </span>
                    <span className="ml-2 text-sm font-medium">{ins.title}</span>
                    <p className="mt-1 text-xs text-muted-foreground">{ins.detail}</p>
                  </div>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={async (e) => {
                      e.stopPropagation()
                      await api.post('/wiki/insights/dismiss', { key: ins.key })
                      load()
                    }}
                  >
                    忽略
                  </Button>
                </div>
              </Card>
            )
          })}
        </div>
      )}

      {report.communities.length > 0 && (
        <details className="rounded-xl border border-border bg-card p-3">
          <summary className="cursor-pointer text-sm font-medium">社区列表（Louvain）</summary>
          <div className="mt-2 space-y-1.5 text-xs text-muted-foreground">
            {report.communities.map((c) => (
              <p key={c.id}>
                #{c.id}：{c.top_slug} 等 {c.size} 页 · 凝聚度 {c.cohesion.toFixed(2)}
                {c.cohesion < 0.15 && c.size >= 3 ? ' · 稀疏' : ''}
              </p>
            ))}
          </div>
        </details>
      )}
    </div>
  )
}

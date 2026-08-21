/**
 * Review 队列（llm_wiki 对齐）：ingest 时 LLM flag 的人审项。
 * 预定义动作 + 预生成检索词；处理不阻塞 ingest。
 */
import { useEffect, useState } from 'react'
import { api } from '@/lib/api'
import { Empty, ErrorBox, Spinner } from '@/components/ui-bits'
import { Button } from '@/components/ui/button'

interface ReviewItem {
  id: string
  kind: string
  payload: { title?: string; reason?: string; suggested_slug?: string }
  action: string | null
  search_queries: string[]
  status: string
  created_at: string
}

const KIND_LABEL: Record<string, string> = {
  create_page: '建议建页',
  deep_research: '深度检索',
  skip: '建议跳过',
  flag: '需人判断',
}

const ACTIONS: Record<string, string[]> = {
  create_page: ['创建页面', '跳过'],
  deep_research: ['执行检索', '忽略'],
  skip: ['确认跳过', '保留观察'],
  flag: ['已处理', '忽略'],
}

export default function ReviewQueue() {
  const [items, setItems] = useState<ReviewItem[] | null>(null)
  const [err, setErr] = useState('')

  const load = () => api.get<ReviewItem[]>('/wiki/reviews').then(setItems).catch((e) => setErr(e.message))
  useEffect(() => {
    load()
  }, [])

  if (err) return <ErrorBox msg={err} />
  if (!items) return <Spinner label="人审队列加载…" />

  return (
    <div className="space-y-3" data-testid="review-queue">
      {items.length === 0 ? (
        <Empty text="人审队列为空（ingest 时 LLM 会标记需要人判断的项）" />
      ) : (
        items.map((it) => (
          <div key={it.id} className="rounded-lg border p-4" data-testid={`review-${it.kind}`}>
            <div className="flex items-start justify-between gap-2">
              <div>
                <span className="mr-2 rounded bg-blue-500/15 px-1.5 py-0.5 text-xs text-blue-400">
                  {KIND_LABEL[it.kind] ?? it.kind}
                </span>
                <span className="font-medium">{it.payload.title ?? '（无标题）'}</span>
                {it.payload.reason && (
                  <p className="mt-1 text-xs text-muted-foreground">{it.payload.reason}</p>
                )}
                {it.payload.suggested_slug && (
                  <p className="mt-0.5 text-xs text-muted-foreground">建议页名：{it.payload.suggested_slug}</p>
                )}
                {it.search_queries?.length > 0 && (
                  <div className="mt-1 flex flex-wrap gap-1">
                    {it.search_queries.map((q, i) => (
                      <code key={i} className="rounded bg-muted/50 px-1 text-xs">{q}</code>
                    ))}
                  </div>
                )}
              </div>
              <div className="flex shrink-0 gap-1">
                {(ACTIONS[it.kind] ?? ['已处理']).map((a) => (
                  <Button
                    key={a}
                    size="sm"
                    variant="outline"
                    data-testid={`review-action-${a}`}
                    onClick={async () => {
                      await api.post(`/wiki/reviews/${it.id}/resolve`, { action: a })
                      load()
                    }}
                  >
                    {a}
                  </Button>
                ))}
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={async () => {
                    await api.post(`/wiki/reviews/${it.id}/resolve`, { dismiss: true })
                    load()
                  }}
                >
                  dismiss
                </Button>
              </div>
            </div>
          </div>
        ))
      )}
    </div>
  )
}

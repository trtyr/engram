/** Study 学习路线图（P007）：领域 track→知识点状态机→挂 wiki。过程视图；知识本体在 wiki。 */
import { useEffect, useState } from 'react'
import { api } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner, StatusBadge } from '@/components/ui-bits'
import { fmtTime, inputCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'

interface StudyItem {
  id: string
  name: string
  status: string // not_started | learning | learned
  position: number
  wiki_slugs: string[]
  doc_ids: string[]
  learned_at: string | null
}

interface TopicFull {
  id: string
  name: string
  goal: string
  status: string
  items: StudyItem[]
  progress: { total: number; learned: number }
  next_up: StudyItem[]
  in_progress: StudyItem[]
}

interface TopicBrief {
  id: string
  name: string
  goal: string
  status: string
  updated_at: string
}

const STATUS_LABEL: Record<string, string> = {
  not_started: '待学',
  learning: '进行中',
  learned: '已学',
}

function statusChip(status: string) {
  const cls =
    status === 'learned'
      ? 'text-emerald-600 dark:text-emerald-400'
      : status === 'learning'
        ? 'text-amber-600 dark:text-amber-400'
        : 'text-muted-foreground'
  return <span className={`text-xs font-medium ${cls}`}>{STATUS_LABEL[status] ?? status}</span>
}

export default function Study() {
  const [topics, setTopics] = useState<TopicBrief[] | null>(null)
  const [err, setErr] = useState('')
  const [expanded, setExpanded] = useState<string | null>(null)
  const [full, setFull] = useState<TopicFull | null>(null)
  const [fullLoading, setFullLoading] = useState(false)
  const [newName, setNewName] = useState('')
  const [newGoal, setNewGoal] = useState('')
  const [itemName, setItemName] = useState('')

  const loadTopics = () =>
    api
      .get<{ topics: TopicBrief[] }>('/study/topics')
      .then((v) => setTopics(v.topics))
      .catch((e) => setErr(String(e)))

  useEffect(() => {
    loadTopics()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const expand = (id: string) => {
    if (expanded === id) {
      setExpanded(null)
      setFull(null)
      return
    }
    setExpanded(id)
    setFull(null)
    setFullLoading(true)
    api
      .get<TopicFull>(`/study/topics/${id}`)
      .then((v) => setFull(v))
      .catch((e) => setErr(String(e)))
      .finally(() => setFullLoading(false))
  }

  const addTopic = async () => {
    if (!newName.trim()) return
    setErr('')
    try {
      await api.post('/study/topics', { name: newName.trim(), goal: newGoal.trim() })
      setNewName('')
      setNewGoal('')
      loadTopics()
    } catch (e) {
      setErr(String(e))
    }
  }

  const addItem = async (topicId: string) => {
    if (!itemName.trim()) return
    setErr('')
    try {
      await api.post(`/study/topics/${topicId}/items`, { name: itemName.trim() })
      setItemName('')
      expand(topicId) // 重新拉全量
      if (expanded === topicId) loadTopics()
    } catch (e) {
      setErr(String(e))
    }
  }

  const setItemStatus = async (topicId: string, itemId: string, status: string) => {
    setErr('')
    try {
      await api.patch(`/study/items/${itemId}`, { status })
      expand(topicId)
    } catch (e) {
      setErr(String(e))
    }
  }

  const setTopicStatus = async (topicId: string, status: string) => {
    setErr('')
    try {
      await api.patch(`/study/topics/${topicId}`, { status })
      loadTopics()
      if (expanded === topicId) expand(topicId)
    } catch (e) {
      setErr(String(e))
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader title="学习" desc="学习路线图跟踪：领域→知识点状态机→挂 wiki 页。知识本体在 wiki，这里只管学到哪。" />

      {err && <ErrorBox msg={err} />}

      {/* 新建领域 */}
      <Card className="p-4">
        <div className="flex flex-wrap items-center gap-2">
          <input
            className={inputCls + ' max-w-xs'}
            placeholder="新领域名（如：RAG 入门）"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
          />
          <input
            className={inputCls + ' max-w-md'}
            placeholder="目标（学到什么程度算完，可选）"
            value={newGoal}
            onChange={(e) => setNewGoal(e.target.value)}
          />
          <Button size="sm" disabled={!newName.trim()} onClick={addTopic}>
            开题
          </Button>
        </div>
      </Card>

      {/* 领域列表 */}
      {topics === null ? (
        <Spinner />
      ) : topics.length === 0 ? (
        <Empty text="还没有学习领域——上面的表单开个题吧" />
      ) : (
        <div className="space-y-3">
          {topics.map((t) => {
            const isOpen = expanded === t.id
            return (
              <Card key={t.id} className="p-4 space-y-3">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <button
                    className="text-left font-medium hover:underline"
                    onClick={() => expand(t.id)}
                  >
                    {t.name}
                  </button>
                  <div className="flex items-center gap-2">
                    <StatusBadge status={t.status} />
                    {t.status === 'active' && (
                      <Button size="sm" variant="outline" onClick={() => setTopicStatus(t.id, 'paused')}>
                        暂停
                      </Button>
                    )}
                    {t.status !== 'done' && (
                      <Button size="sm" variant="outline" onClick={() => setTopicStatus(t.id, 'done')}>
                        归档
                      </Button>
                    )}
                    {t.status === 'paused' && (
                      <Button size="sm" variant="outline" onClick={() => setTopicStatus(t.id, 'active')}>
                        恢复
                      </Button>
                    )}
                  </div>
                </div>
                {t.goal && <div className="text-sm text-muted-foreground">目标：{t.goal}</div>}
                <div className="text-xs text-muted-foreground">更新于 {fmtTime(t.updated_at)}</div>

                {isOpen && (
                  <div className="border-t pt-3 space-y-3">
                    {fullLoading ? (
                      <Spinner />
                    ) : full && full.id === t.id ? (
                      <>
                        {/* 进度条 */}
                        <div className="flex items-center gap-3">
                          <div className="h-2 flex-1 rounded-full bg-muted overflow-hidden">
                            <div
                              className="h-full bg-emerald-500 transition-all"
                              style={{
                                width: full.progress.total
                                  ? `${Math.round((full.progress.learned / full.progress.total) * 100)}%`
                                  : '0%',
                              }}
                            />
                          </div>
                          <span className="text-xs text-muted-foreground">
                            {full.progress.learned}/{full.progress.total} 已学
                          </span>
                        </div>

                        {/* 知识点列表 */}
                        {full.items.length === 0 ? (
                          <Empty text="还没有知识点——下面加一个" />
                        ) : (
                          <div className="space-y-1">
                            {full.items.map((it) => (
                              <div
                                key={it.id}
                                className="flex flex-wrap items-center gap-2 rounded border px-3 py-2"
                              >
                                {statusChip(it.status)}
                                <span className={it.status === 'learned' ? 'line-through opacity-70' : ''}>
                                  {it.name}
                                </span>
                                {it.learned_at && (
                                  <span className="text-xs text-muted-foreground">
                                    {fmtTime(it.learned_at)}
                                  </span>
                                )}
                                <div className="ml-auto flex items-center gap-1">
                                  {(['not_started', 'learning', 'learned'] as const).map((s) => (
                                    <button
                                      key={s}
                                      className={`rounded px-2 py-0.5 text-xs border ${
                                        it.status === s
                                          ? 'border-foreground/60 bg-muted font-medium'
                                          : 'border-border text-muted-foreground hover:border-foreground/40'
                                      }`}
                                      onClick={() => setItemStatus(t.id, it.id, s)}
                                    >
                                      {STATUS_LABEL[s]}
                                    </button>
                                  ))}
                                  {it.wiki_slugs.map((slug) => (
                                    <a
                                      key={slug}
                                      href={`/wiki?slug=${encodeURIComponent(slug)}`}
                                      className="text-xs text-blue-600 dark:text-blue-400 hover:underline"
                                      title={`wiki: ${slug}`}
                                    >
                                      [[{slug}]]
                                    </a>
                                  ))}
                                </div>
                              </div>
                            ))}
                          </div>
                        )}

                        {/* 加知识点 */}
                        <div className="flex items-center gap-2">
                          <input
                            className={inputCls + ' max-w-xs'}
                            placeholder="新知识点名"
                            value={itemName}
                            onChange={(e) => setItemName(e.target.value)}
                          />
                          <Button size="sm" disabled={!itemName.trim()} onClick={() => addItem(t.id)}>
                            加知识点
                          </Button>
                        </div>

                        {/* 下一步提示 */}
                        {full.next_up.length > 0 && (
                          <div className="text-xs text-muted-foreground">
                            下一步：
                            {full.next_up.map((n) => n.name).join(' → ')}
                          </div>
                        )}
                      </>
                    ) : null}
                  </div>
                )}
              </Card>
            )
          })}
        </div>
      )}
    </div>
  )
}

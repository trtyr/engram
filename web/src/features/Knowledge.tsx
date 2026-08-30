/** Knowledge 域：左目录右阅读的主从版式 + 摄取/URL + 知识块检索。 */
import { useEffect, useRef, useState } from 'react'
import { Link2, Search, Upload } from 'lucide-react'
import { api, type ChunkHit, type Document } from '@/lib/api'
import {
  Card,
  Empty,
  ErrorBox,
  PageHeader,
  Spinner,
  StatusBadge,
} from '@/components/ui-bits'
import { fmtTime, relTime, inputCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

/** 服务端解析支持：pdf / docx / html / md / txt（其余二进制格式会被拒绝）。 */
const ACCEPT = '.pdf,.docx,.html,.htm,.md,.txt'

const MIME_LABEL: Record<string, string> = {
  'application/pdf': 'pdf',
  'application/vnd.openxmlformats-officedocument.wordprocessingml.document': 'docx',
  'text/html': 'html',
  'text/markdown': 'md',
  'text/plain': 'txt',
}
const mimeTag = (m: string | null) =>
  m ? MIME_LABEL[m] ?? m.replace(/^application\//, '').slice(0, 8) : '—'

export default function Knowledge() {
  const [docs, setDocs] = useState<Document[] | null>(null)
  const [err, setErr] = useState('')
  const [url, setUrl] = useState('')
  const [selected, setSelected] = useState<string | null>(null)
  const [filter, setFilter] = useState('')
  const [dragging, setDragging] = useState(false)
  const [ingesting, setIngesting] = useState(false)
  const [notice, setNotice] = useState('')
  const [q, setQ] = useState('')
  const [searching, setSearching] = useState(false)
  const [hits, setHits] = useState<ChunkHit[] | null>(null)
  const fileRef = useRef<HTMLInputElement>(null)

  const load = () => api.get<Document[]>('/knowledge/documents?limit=100').then(setDocs).catch((e) => setErr(e.message))
  useEffect(() => {
    load()
  }, [])
  // 存在处理中文档时轮询刷新（状态推进可视化；全部终态后停）
  useEffect(() => {
    if (!docs?.some((d) => ['pending', 'parsing', 'chunking', 'embedding'].includes(d.status))) return
    const t = setInterval(load, 3000)
    return () => clearInterval(t)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [docs])

  const upload = async (f: File) => {
    setIngesting(true)
    setNotice(`摄取中：${f.name}`)
    try {
      await api.upload('/knowledge/upload', f)
      setNotice(`已入列：${f.name}`)
      load()
    } catch (ex) {
      setNotice(ex instanceof Error ? `摄取失败：${ex.message}` : '摄取失败')
    } finally {
      setIngesting(false)
    }
  }

  // 目录列表（标题过滤，客户端）
  const visibleDocs = (docs ?? []).filter((d) => !filter || d.title.toLowerCase().includes(filter.toLowerCase()))
  // 有效选中：显式选中 → 失效时回退首篇（新建/删除后无空窗）
  const activeId =
    selected && visibleDocs.some((d) => d.id === selected) ? selected : (visibleDocs[0]?.id ?? null)
  const activeDoc = (docs ?? []).find((d) => d.id === activeId) ?? null

  return (
    <div className="space-y-6">
      <PageHeader title="Knowledge" desc="文档 / URL 摄取 → 分块 → 嵌入 → 混合检索" />

      <div
        className={cn(
          'rounded-lg border-2 border-dashed px-4 py-3 transition-colors',
          dragging ? 'border-foreground/60 bg-muted/40' : 'border-border hover:border-foreground/30',
          ingesting && 'border-info/50',
        )}
        data-testid="dropzone"
        onDragOver={(e) => {
          e.preventDefault()
          setDragging(true)
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={async (e) => {
          e.preventDefault()
          setDragging(false)
          const f = e.dataTransfer.files?.[0]
          if (!f) return
          await upload(f)
        }}
      >
        <div className="flex flex-wrap items-center gap-3">
          <Upload className={cn('size-4 text-muted-foreground', ingesting && 'engram-pulse text-info')} />
          <input
            ref={fileRef}
            type="file"
            accept={ACCEPT}
            className="hidden"
            onChange={async (e) => {
              const f = e.target.files?.[0]
              if (!f) return
              await upload(f)
              e.target.value = ''
            }}
          />
          <Button size="sm" disabled={ingesting} onClick={() => fileRef.current?.click()}>
            {ingesting ? '摄取中…' : '上传文件'}
          </Button>
          <form
            className="flex flex-1 gap-2"
            onSubmit={async (e) => {
              e.preventDefault()
              if (!url || ingesting) return
              setIngesting(true)
              setNotice(`摄取中：${url}`)
              try {
                await api.post('/knowledge/documents', { url })
                setNotice(`已提交：${url}`)
                setUrl('')
                load()
              } catch (ex) {
                setNotice(ex instanceof Error ? `提交失败：${ex.message}` : '提交失败')
              } finally {
                setIngesting(false)
              }
            }}
          >
            <input
              className={`${inputCls} flex-1`}
              placeholder="https://… 粘贴 URL 摄取网页"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
            />
            <Button size="sm" variant="outline" type="submit" disabled={ingesting}>
              摄取 URL
            </Button>
          </form>
        </div>
        {/* 单行状态：无动作时显示格式提示，有反馈时覆盖（省一行常驻空位） */}
        <p className="mt-2 min-h-4 text-xs">
          {notice ? (
            <span className={notice.includes('失败') ? 'text-destructive' : notice.includes('摄取中') ? 'text-info' : 'text-success'}>
              {notice}
            </span>
          ) : (
            <span className="text-muted-foreground/70">支持 pdf / docx / html / md / txt，或直接把文件拖进这个框</span>
          )}
        </p>
      </div>

      {err && <ErrorBox msg={err} />}

      <form
        className="flex gap-2"
        onSubmit={async (e) => {
          e.preventDefault()
          if (searching || !q.trim()) return
          setSearching(true)
          try {
            setHits(await api.post<ChunkHit[]>('/knowledge/search', { query: q, max_items: 10 }))
          } finally {
            setSearching(false)
          }
        }}
      >
        <input
          className={`${inputCls} flex-1`}
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder="检索知识库…（命中可直接打开对应文档）"
        />
        <Button type="submit" disabled={searching}>
          {searching ? '检索中…' : '检索'}
        </Button>
      </form>
      {hits && (
        <Card className="p-4">
          {hits.length === 0 ? (
            <Empty text="无命中知识块" />
          ) : (
            hits.map((h) => (
              <button
                key={h.chunk_id}
                type="button"
                className="block w-full border-b border-border/50 py-2.5 text-left text-sm transition-colors last:border-0 hover:bg-muted/40"
                onClick={() => setSelected(h.document_id)}
              >
                <p className="mb-1 flex flex-wrap items-center gap-2 font-mono text-xs text-muted-foreground">
                  <span className="rounded border border-border px-1.5 py-px">{h.document_title}</span>
                  <span>#{h.seq}</span>
                  {h.embed_failed && (
                    <span className="rounded bg-warning/15 px-1.5 py-px text-warning" title="向量嵌入失败，此块由全文检索降级命中">
                      FTS
                    </span>
                  )}
                  <span className="ml-auto">{h.score.toFixed(3)}</span>
                </p>
                <p className="line-clamp-2">{h.snippet}</p>
              </button>
            ))
          )}
        </Card>
      )}

      {docs === null ? (
        <Spinner />
      ) : (
        <div className="flex flex-col gap-4 lg:flex-row">
          {/* 左：文档目录 */}
          <Card className="overflow-hidden lg:w-80 lg:shrink-0 lg:self-start">
            <div className="flex items-center gap-2 border-b border-border px-3 py-2">
              <span className="font-mono text-xs text-muted-foreground">{visibleDocs.length} 篇</span>
              <div className="relative ml-auto">
                <Search className="pointer-events-none absolute left-2 top-1/2 size-3 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
                <input
                  className="w-36 rounded-md border border-border bg-card py-1 pl-7 pr-2 text-xs outline-none transition-colors placeholder:text-muted-foreground/60 focus-visible:border-foreground/40"
                  placeholder="过滤标题…"
                  value={filter}
                  onChange={(e) => setFilter(e.target.value)}
                />
              </div>
            </div>
            {visibleDocs.length === 0 ? (
              <div className="p-4">
                <Empty text={docs.length === 0 ? '暂无文档——拖拽文件到上方摄取区，或粘贴 URL' : '无匹配标题'} />
              </div>
            ) : (
              <ul className="max-h-64 divide-y divide-border/60 overflow-auto lg:max-h-[calc(100vh-16rem)]">
                {visibleDocs.map((d) => (
                  <li key={d.id}>
                    <button
                      type="button"
                      onClick={() => setSelected(d.id)}
                      aria-current={d.id === activeId ? 'true' : undefined}
                      className={cn(
                        'w-full px-3 py-2 text-left transition-colors',
                        d.id === activeId ? 'bg-foreground text-background' : 'hover:bg-muted/40',
                      )}
                    >
                      <p className="truncate text-sm font-medium">{d.title}</p>
                      <p className={cn(
                        'mt-0.5 flex items-center gap-1.5 font-mono text-xs',
                        d.id === activeId ? 'text-background/70' : 'text-muted-foreground',
                      )}>
                        {d.source_uri ? <Link2 className="size-3" aria-label="URL 摄取" /> : <span>{mimeTag(d.mime)}</span>}
                        <span className="truncate">{relTime(d.created_at)}</span>
                        {['pending', 'parsing', 'chunking', 'embedding'].includes(d.status) && (
                          <span className={cn('size-1.5 rounded-full bg-info', d.id === activeId ? '' : 'engram-pulse')} />
                        )}
                      </p>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </Card>

          {/* 右：阅读区 */}
          <div className="min-w-0 flex-1">
            {activeDoc ? (
              <DocReader
                key={activeDoc.id}
                doc={activeDoc}
                onDeleted={() => {
                  setSelected(null)
                  load()
                }}
              />
            ) : (
              <Card className="p-4">
                <Empty text="从左侧选择一篇文档开始阅读" />
              </Card>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

/** 阅读区：文档头（标题/来源/状态/操作）+ 分块连成的整篇成文。 */
function DocReader({ doc, onDeleted }: { doc: Document; onDeleted: () => void }) {
  const [rows, setRows] = useState<{ seq: number; content: string; embed_failed: boolean }[] | null>(null)
  const [busy, setBusy] = useState(false)
  const [msg, setMsg] = useState('')
  useEffect(() => {
    api.get<typeof rows>(`/knowledge/documents/${doc.id}/chunks`).then(setRows).catch(() => setRows([]))
  }, [doc.id])
  const failedCount = (rows ?? []).filter((c) => c.embed_failed).length
  return (
    <Card className="overflow-hidden">
      <div className="border-b border-border px-4 py-3">
        <div className="flex flex-wrap items-start justify-between gap-2">
          <div className="min-w-0">
            <h2 className="truncate text-base font-semibold tracking-tight">{doc.title}</h2>
            <p className="mt-1 flex flex-wrap items-center gap-2 font-mono text-xs text-muted-foreground">
              <StatusBadge status={doc.status} />
              {doc.source_uri ? (
                <span className="truncate" title={doc.source_uri}>
                  <Link2 className="mr-0.5 inline size-3" aria-label="URL 摄取" />
                  {doc.source_uri}
                </span>
              ) : (
                <span>{mimeTag(doc.mime)}</span>
              )}
              <span>{fmtTime(doc.created_at)}</span>
            </p>
          </div>
          <Button
            variant="ghost"
            size="sm"
            className="hover:bg-destructive/10 hover:text-destructive"
            onClick={async () => {
              if (!confirm(`删除文档「${doc.title}」？分块与嵌入向量将一并删除，不可恢复。`)) return
              await api.del(`/knowledge/documents/${doc.id}`)
              onDeleted()
            }}
          >
            删除
          </Button>
        </div>
        {doc.error && <p className="mt-2 text-xs text-destructive">{doc.error}</p>}
      </div>

      <div className="px-4 py-4">
        {rows === null ? (
          <Spinner />
        ) : (
          <>
            {failedCount > 0 && (
              <div className="mb-4 flex flex-wrap items-center gap-2 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2">
                <span className="text-xs text-warning">{failedCount} 个分块嵌入失败（FTS 降级）</span>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={busy}
                  onClick={async () => {
                    setBusy(true)
                    setMsg('')
                    try {
                      await api.post(`/knowledge/documents/${doc.id}/re-embed`)
                      setMsg('重嵌任务已入队')
                    } catch (ex) {
                      setMsg(ex instanceof Error ? ex.message : '重嵌失败')
                    } finally {
                      setBusy(false)
                    }
                  }}
                >
                  {busy ? '入队中…' : '重嵌缺失块'}
                </Button>
                {msg && <span className="text-xs text-muted-foreground">{msg}</span>}
              </div>
            )}
            <p className="mb-3 font-mono text-xs text-muted-foreground/70">
              共 {rows.length} 块 · 全文 {rows.reduce((s, c) => s + c.content.length, 0).toLocaleString()} 字
            </p>
            {/* 阅读视图：分块连成整篇；块的 seq/FTS 调试信息退到悬停 title */}
            <div className="max-w-[70ch] space-y-3 text-sm leading-relaxed">
              {rows.map((c) => (
                <p key={c.seq} title={`#${c.seq}${c.embed_failed ? ' · FTS 降级' : ''}`}>
                  {c.content}
                </p>
              ))}
            </div>
          </>
        )}
      </div>
    </Card>
  )
}

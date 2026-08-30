/** Knowledge 域：文档表格 + 上传/URL + 分块预览 + 检索。 */
import { Fragment, useEffect, useRef, useState } from 'react'
import { Link2, Upload } from 'lucide-react'
import { api, type ChunkHit, type Document } from '@/lib/api'
import {
  Card,
  Empty,
  ErrorBox,
  PageHeader,
  Spinner,
  StatusBadge,
} from '@/components/ui-bits'
import { fmtTime, inputCls, tableCls } from '@/lib/ui'
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
  const [openChunks, setOpenChunks] = useState<string | null>(null)
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

  return (
    <div className="space-y-6">
      <PageHeader title="Knowledge" desc="文档 / URL 摄取 → 分块 → 嵌入 → 混合检索" />

      <div
        className={cn(
          'rounded-lg border-2 border-dashed p-4 transition-colors',
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
        <p className="mt-2.5 flex items-center gap-2 text-xs text-muted-foreground/70">
          <span>支持 pdf / docx / html / md / txt，或直接把文件拖进这个框</span>
        </p>
        <p className="mt-1 min-h-4 text-xs">
          {notice && (
            <span className={notice.includes('失败') ? 'text-destructive' : notice.includes('摄取中') ? 'text-info' : 'text-success'}>
              {notice}
            </span>
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
          placeholder="检索知识库…"
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
              <div key={h.chunk_id} className="border-b border-border/50 py-2.5 text-sm last:border-0">
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
                <p>{h.snippet}</p>
              </div>
            ))
          )}
        </Card>
      )}

      {docs === null ? (
        <Spinner />
      ) : docs.length === 0 ? (
        <Empty text="暂无文档——拖拽文件到上方摄取区，或粘贴 URL 开始构建知识库" />
      ) : (
        <Card className="overflow-x-auto">
          <table className={tableCls.root}>
            <thead className={tableCls.thead}>
              <tr>
                <th className={tableCls.th}>标题</th>
                <th className={tableCls.th}>来源</th>
                <th className={tableCls.th}>状态</th>
                <th className={tableCls.th}>时间</th>
                <th className={tableCls.th}>错误</th>
                <th className={tableCls.th} />
              </tr>
            </thead>
            <tbody>
              {docs.map((d) => (
                <Fragment key={d.id}>
                  <tr className={tableCls.row}>
                    <td className={`${tableCls.td} max-w-72 truncate font-medium`} title={d.title}>
                      {d.title}
                    </td>
                    <td className={`${tableCls.tdMono}`} title={d.source_uri || mimeTag(d.mime)}>
                      {d.source_uri ? (
                        <Link2 className="size-3.5 text-muted-foreground" aria-label="URL 摄取" />
                      ) : (
                        mimeTag(d.mime)
                      )}
                    </td>
                    <td className={tableCls.td}>
                      <StatusBadge status={d.status} />
                    </td>
                    <td className={`${tableCls.td} text-muted-foreground`}>{fmtTime(d.created_at)}</td>
                    <td className={`${tableCls.td} max-w-48 truncate text-destructive`} title={d.error ?? undefined}>
                      {d.error ?? ''}
                    </td>
                    <td className={`${tableCls.td} whitespace-nowrap text-right`}>
                      <Button
                        variant="ghost"
                        size="sm"
                        className="mr-1"
                        aria-expanded={openChunks === d.id}
                        onClick={() => setOpenChunks(openChunks === d.id ? null : d.id)}
                      >
                        {openChunks === d.id ? '收起' : '分块'}
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={async () => {
                          if (!confirm(`删除文档「${d.title}」？分块与嵌入向量将一并删除，不可恢复。`)) return
                          await api.del(`/knowledge/documents/${d.id}`)
                          if (openChunks === d.id) setOpenChunks(null)
                          load()
                        }}
                      >
                        删除
                      </Button>
                    </td>
                  </tr>
                  {/* 手风琴：分块预览紧贴该行下方展开，视线不断裂 */}
                  {openChunks === d.id && (
                    <tr>
                      <td colSpan={6} className="border-b border-border p-0">
                        <ChunksPanel docId={d.id} />
                      </td>
                    </tr>
                  )}
                </Fragment>
              ))}
            </tbody>
          </table>
        </Card>
      )}
    </div>
  )
}

function ChunksPanel({ docId }: { docId: string }) {
  const [rows, setRows] = useState<{ seq: number; content: string; embed_failed: boolean }[] | null>(null)
  const [busy, setBusy] = useState(false)
  const [msg, setMsg] = useState('')
  useEffect(() => {
    api.get<typeof rows>(`/knowledge/documents/${docId}/chunks`).then(setRows).catch(() => setRows([]))
  }, [docId])
  if (!rows) return <Spinner />
  const failedCount = rows.filter((c) => c.embed_failed).length
  return (
    <div className="max-h-96 overflow-auto bg-muted/30 px-4 py-3">
      {failedCount > 0 && (
        <div className="mb-3 flex flex-wrap items-center gap-2 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2">
          <span className="text-xs text-warning">{failedCount} 个分块嵌入失败（FTS 降级）</span>
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={async () => {
              setBusy(true)
              setMsg('')
              try {
                await api.post(`/knowledge/documents/${docId}/re-embed`)
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
      {rows.map((c) => (
        <div key={c.seq} className="border-b border-border/50 py-2.5 text-sm last:border-0">
          <p className="mb-1 text-xs text-muted-foreground">#{c.seq} {c.embed_failed ? '（FTS 降级）' : ''}</p>
          <p className="line-clamp-3">{c.content}</p>
        </div>
      ))}
    </div>
  )
}

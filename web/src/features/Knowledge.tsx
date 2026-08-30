/** Knowledge 域：文档表格 + 上传/URL + 分块预览 + 检索。 */
import { useEffect, useRef, useState } from 'react'
import { Upload } from 'lucide-react'
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

export default function Knowledge() {
  const [docs, setDocs] = useState<Document[] | null>(null)
  const [err, setErr] = useState('')
  const [url, setUrl] = useState('')
  const [openChunks, setOpenChunks] = useState<string | null>(null)
  const [q, setQ] = useState('')
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
    try {
      await api.upload('/knowledge/upload', f)
      load()
    } catch (ex) {
      setErr(ex instanceof Error ? ex.message : '上传失败')
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader title="Knowledge" desc="文档 / URL 摄取 → 分块 → 嵌入 → 混合检索" />

      <div
        className="flex flex-wrap items-center gap-3 rounded-xl border-2 border-dashed border-border p-4 transition-colors hover:border-brand/50"
        data-testid="dropzone"
        onDragOver={(e) => {
          e.preventDefault()
          e.currentTarget.classList.add('border-brand')
        }}
        onDragLeave={(e) => {
          e.currentTarget.classList.remove('border-brand')
        }}
        onDrop={async (e) => {
          e.preventDefault()
          e.currentTarget.classList.remove('border-brand')
          const f = e.dataTransfer.files?.[0]
          if (!f) return
          await upload(f)
        }}
      >
        <Upload className="size-4 text-muted-foreground" />
        <input
          ref={fileRef}
          type="file"
          className="hidden"
          onChange={async (e) => {
            const f = e.target.files?.[0]
            if (!f) return
            await upload(f)
            e.target.value = ''
          }}
        />
        <Button size="sm" onClick={() => fileRef.current?.click()}>
          上传文件
        </Button>
        <form
          className="flex flex-1 gap-2"
          onSubmit={async (e) => {
            e.preventDefault()
            if (!url) return
            try {
              await api.post('/knowledge/documents', { url })
              setUrl('')
              load()
            } catch (ex) {
              setErr(ex instanceof Error ? ex.message : '提交失败')
            }
          }}
        >
          <input
            className={`${inputCls} flex-1`}
            placeholder="https://…"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
          />
          <Button size="sm" variant="outline" type="submit">
            摄取 URL
          </Button>
        </form>
      </div>

      {err && <ErrorBox msg={err} />}

      <form
        className="flex gap-2"
        onSubmit={async (e) => {
          e.preventDefault()
          setHits(await api.post<ChunkHit[]>('/knowledge/search', { query: q, max_items: 10 }))
        }}
      >
        <input
          className={`${inputCls} flex-1`}
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder="检索知识库…"
        />
        <Button type="submit">检索</Button>
      </form>
      {hits && (
        <Card className="p-4">
          {hits.length === 0 ? (
            <p className="text-sm text-muted-foreground">无命中</p>
          ) : (
            hits.map((h) => (
              <div key={h.chunk_id} className="border-b border-border/50 py-2.5 text-sm last:border-0">
                <p className="mb-1 text-xs text-muted-foreground">
                  [{h.document_title}] #{h.seq} {h.embed_failed ? '(FTS)' : ''} (
                  {h.score.toFixed(3)})
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
        <Empty text="暂无文档" />
      ) : (
        <Card className="overflow-x-auto">
          <table className={tableCls.root}>
            <thead className={tableCls.thead}>
              <tr>
                <th className={tableCls.th}>标题</th>
                <th className={tableCls.th}>状态</th>
                <th className={tableCls.th}>时间</th>
                <th className={tableCls.th}>错误</th>
                <th className={tableCls.th} />
              </tr>
            </thead>
            <tbody>
              {docs.map((d) => (
                <tr key={d.id} className={tableCls.row}>
                  <td className={`${tableCls.td} max-w-72 truncate font-medium`}>{d.title}</td>
                  <td className={tableCls.td}>
                    <StatusBadge status={d.status} />
                  </td>
                  <td className={`${tableCls.td} text-muted-foreground`}>{fmtTime(d.created_at)}</td>
                  <td className={`${tableCls.td} max-w-48 truncate text-destructive`}>{d.error ?? ''}</td>
                  <td className={`${tableCls.td} whitespace-nowrap text-right`}>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="mr-1"
                      onClick={() => setOpenChunks(openChunks === d.id ? null : d.id)}
                    >
                      分块
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={async () => {
                        await api.del(`/knowledge/documents/${d.id}`)
                        load()
                      }}
                    >
                      删除
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}

      {openChunks && <ChunksPanel docId={openChunks} />}
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
    <Card className="max-h-96 overflow-auto p-4">
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
    </Card>
  )
}

/** Knowledge 域：文档表格 + 上传/URL + 分块预览 + 检索。 */
import { useEffect, useRef, useState } from 'react'
import { api, type ChunkHit, type Document } from '@/lib/api'
import { Empty, ErrorBox, Spinner, StatusBadge, fmtTime } from '@/components/ui-bits'
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

  return (
    <div className="space-y-6">
      <h1 className="text-xl font-semibold">Knowledge</h1>

      <div className="flex flex-wrap items-center gap-2">
        <input ref={fileRef} type="file" className="hidden" onChange={async (e) => {
          const f = e.target.files?.[0]
          if (!f) return
          try {
            await api.upload('/knowledge/upload', f)
            load()
          } catch (ex) {
            setErr(ex instanceof Error ? ex.message : '上传失败')
          }
          e.target.value = ''
        }} />
        <Button size="sm" onClick={() => fileRef.current?.click()}>上传文件</Button>
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
          <input className="flex-1 rounded-md border bg-transparent px-3 py-1.5 text-sm" placeholder="https://…" value={url} onChange={(e) => setUrl(e.target.value)} />
          <Button size="sm" variant="outline" type="submit">摄取 URL</Button>
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
        <input className="flex-1 rounded-md border bg-transparent px-3 py-2 text-sm" value={q} onChange={(e) => setQ(e.target.value)} placeholder="检索知识库…" />
        <Button type="submit">检索</Button>
      </form>
      {hits && (
        <div className="rounded-lg border p-4">
          {hits.length === 0 ? (
            <p className="text-sm text-muted-foreground">无命中</p>
          ) : (
            hits.map((h) => (
              <div key={h.chunk_id} className="border-b py-2 text-sm last:border-0">
                <p className="text-xs text-muted-foreground">
                  [{h.document_title}] #{h.seq} {h.embed_failed ? '(FTS)' : ''} ({h.score.toFixed(3)})
                </p>
                <p>{h.snippet}</p>
              </div>
            ))
          )}
        </div>
      )}

      {docs === null ? (
        <Spinner />
      ) : docs.length === 0 ? (
        <Empty text="暂无文档" />
      ) : (
        <table className="w-full text-sm">
          <thead className="text-left text-muted-foreground">
            <tr className="border-b">
              <th className="py-1.5 pr-4">标题</th>
              <th className="pr-4">状态</th>
              <th className="pr-4">时间</th>
              <th className="pr-4">错误</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {docs.map((d) => (
              <tr key={d.id} className="border-b">
                <td className="max-w-72 truncate py-1.5 pr-4">{d.title}</td>
                <td className="pr-4"><StatusBadge status={d.status} /></td>
                <td className="pr-4">{fmtTime(d.created_at)}</td>
                <td className="max-w-48 truncate pr-4 text-red-400">{d.error ?? ''}</td>
                <td className="text-right">
                  <Button variant="ghost" size="sm" className="mr-1" onClick={() => setOpenChunks(openChunks === d.id ? null : d.id)}>
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
      )}

      {openChunks && <ChunksPanel docId={openChunks} />}
    </div>
  )
}

function ChunksPanel({ docId }: { docId: string }) {
  const [rows, setRows] = useState<{ seq: number; content: string; embed_failed: boolean }[] | null>(null)
  useEffect(() => {
    api.get<typeof rows>(`/knowledge/documents/${docId}/chunks`).then(setRows).catch(() => setRows([]))
  }, [docId])
  if (!rows) return <Spinner />
  return (
    <div className="max-h-96 space-y-2 overflow-auto rounded-lg border p-4">
      {rows.map((c) => (
        <div key={c.seq} className="border-b pb-2 text-sm last:border-0">
          <p className="text-xs text-muted-foreground">#{c.seq} {c.embed_failed ? '（FTS 降级）' : ''}</p>
          <p className="line-clamp-3">{c.content}</p>
        </div>
      ))}
    </div>
  )
}

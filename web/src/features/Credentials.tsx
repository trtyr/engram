/**
 * 凭据台账页（EN-234 控制台治理面，2026-09-26）：
 * 机密值的一等台账——静态加密落库、按名取用、取用留痕。
 *
 * 安全语义（与 MCP credentials 域同一 core 服务，零分叉）：
 * - 列表永不回显值；值只在显式「揭示」后返回，且每次揭示留取用痕（read_count+1、流水+1）；
 * - 同名换值清零旧取用审计（值变了旧痕作废）——页面上「换值后流水归零」可直接复现；
 * - 删除级联清流水。
 */
import { useCallback, useEffect, useState } from 'react'
import { api } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner } from '@/components/ui-bits'
import { inputCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'

export interface CredentialMetaDto {
  id: string
  name: string
  sensitive: boolean
  description: string | null
  created_by: string
  created_at: string
  updated_at: string
  last_read_at: string | null
  read_count: number
}

export interface CredentialReadRow {
  id: string
  credential_id: string
  reader: string
  read_at: string
}

export default function Credentials() {
  const [rows, setRows] = useState<CredentialMetaDto[] | null>(null)
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)
  // 流水抽屉：展开哪一行的取用历史
  const [readsFor, setReadsFor] = useState<string | null>(null)
  const [reads, setReads] = useState<CredentialReadRow[] | null>(null)
  // 揭示：只对单行、只在显式点击后
  const [revealFor, setRevealFor] = useState<string | null>(null)
  const [revealed, setRevealed] = useState('')
  // 写入表单
  const [fName, setFName] = useState('')
  const [fValue, setFValue] = useState('')
  const [fDesc, setFDesc] = useState('')
  const [notice, setNotice] = useState('')

  const load = useCallback(() => {
    api
      .get<{ items: CredentialMetaDto[] }>('/credentials')
      .then((r) => setRows(r.items))
      .catch((e) => setErr(String(e)))
  }, [])
  useEffect(() => {
    load()
  }, [load])

  async function loadReads(name: string) {
    if (readsFor === name) {
      setReadsFor(null)
      return
    }
    setReadsFor(name)
    setReads(null)
    const r = await api
      .get<{ reads: CredentialReadRow[] }>(`/credentials/${encodeURIComponent(name)}/reads`)
      .catch(() => null)
    setReads(r?.reads ?? [])
  }

  async function doReveal(name: string) {
    setBusy(true)
    setErr('')
    const r = await api
      .get<{ value: string }>(`/credentials/${encodeURIComponent(name)}/value`)
      .catch((e) => {
        setErr(String(e))
        return null
      })
    setBusy(false)
    if (r) {
      setRevealFor(name)
      setRevealed(r.value)
      load() // read_count/last_read_at 变了，刷台账
    }
  }

  async function doPut() {
    if (!fName || !fValue) return
    setBusy(true)
    setErr('')
    const r = await api
      .post<{ credential?: unknown; hint?: string }>('/credentials', {
        name: fName,
        value: fValue,
        description: fDesc || null,
      })
      .catch((e) => {
        setErr(String(e))
        return null
      })
    setBusy(false)
    if (r) {
      setNotice(`已写入「${fName}」${r.hint ?? ''}`)
      setFName('')
      setFValue('')
      setFDesc('')
      load()
    }
  }

  async function doDelete(name: string) {
    if (!confirm(`删除凭据「${name}」？取用流水一并清除，不可恢复。`)) return
    setBusy(true)
    await api.del(`/credentials/${encodeURIComponent(name)}`).catch((e) => setErr(String(e)))
    setBusy(false)
    if (readsFor === name) setReadsFor(null)
    load()
  }

  if (err && !rows) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  return (
    <div className="space-y-6">
      <PageHeader
        title="凭据台账"
        desc="机密值一等台账：静态加密落库、按名取用、取用留痕。列表永不回显值——揭示是显式动作且每次留痕。"
      />
      {err && <ErrorBox msg={err} />}
      {notice && (
        <div className="rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-300">
          {notice}
        </div>
      )}

      <Card className="p-4">
        <div className="mb-2 text-sm font-medium text-muted-foreground">写入 / 换值</div>
        <div className="flex flex-wrap items-center gap-2">
          <input
            className={inputCls + ' w-56'}
            placeholder="名称（如 newapi/api_key）"
            value={fName}
            onChange={(e) => setFName(e.target.value)}
          />
          <input
            className={inputCls + ' w-72'}
            placeholder="值（写入即加密，永不回显于列表）"
            value={fValue}
            onChange={(e) => setFValue(e.target.value)}
          />
          <input
            className={inputCls + ' w-56'}
            placeholder="说明（可选）"
            value={fDesc}
            onChange={(e) => setFDesc(e.target.value)}
          />
          <Button onClick={doPut} disabled={busy || !fName || !fValue}>
            写入
          </Button>
          <span className="text-xs text-muted-foreground">同名换值：旧取用流水清零（值变了旧痕作废）</span>
        </div>
      </Card>

      {rows.length === 0 ? (
        <Empty text="台账空——写入第一条凭据。" />
      ) : (
        <div className="space-y-2">
          {rows.map((c) => (
            <Card key={c.id} className="p-4">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div className="min-w-0">
                  <div className="font-medium">
                    <span className="font-mono">{c.name}</span>
                    {c.sensitive && (
                      <span className="ml-2 rounded bg-amber-500/15 px-1.5 py-0.5 text-xs text-amber-400">
                        敏感
                      </span>
                    )}
                  </div>
                  <div className="mt-0.5 text-xs text-muted-foreground">
                    {c.description || '（无说明）'} · 取用 {c.read_count} 次
                    {c.last_read_at ? ` · 最近 ${new Date(c.last_read_at).toLocaleString()}` : ' · 从未取用'}
                  </div>
                </div>
                <div className="flex shrink-0 gap-2">
                  <Button variant="outline" onClick={() => loadReads(c.name)}>
                    {readsFor === c.name ? '收起流水' : '取用流水'}
                  </Button>
                  <Button
                    variant="outline"
                    disabled={busy}
                    onClick={() => doReveal(c.name)}
                  >
                    揭示值
                  </Button>
                  <Button variant="destructive" disabled={busy} onClick={() => doDelete(c.name)}>
                    删除
                  </Button>
                </div>
              </div>
              {revealFor === c.name && (
                <div className="mt-3 rounded-md border border-amber-500/40 bg-amber-500/10 p-3">
                  <div className="mb-1 text-xs text-amber-400">
                    明文（本次揭示已留痕；不要粘贴进日志/文档/工单）：
                  </div>
                  <code className="break-all font-mono text-sm">{revealed}</code>
                </div>
              )}
              {readsFor === c.name && (
                <div className="mt-3 rounded-md border border-border/60 bg-muted/30 p-3 text-sm">
                  {reads === null ? (
                    <span className="text-muted-foreground">加载中…</span>
                  ) : reads.length === 0 ? (
                    <span className="text-muted-foreground">无取用记录（从未揭示/换值后已清零）。</span>
                  ) : (
                    <ul className="space-y-1">
                      {reads.map((r) => (
                        <li key={r.id} className="font-mono text-xs">
                          {new Date(r.read_at).toLocaleString()} · {r.reader}
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
              )}
            </Card>
          ))}
        </div>
      )}
    </div>
  )
}

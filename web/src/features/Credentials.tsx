/**
 * 凭据页（EN-234 治理面 · 2026-10-06 重设计——保险箱式双栏）：
 * 机密值的一等台账——静态加密落库、按名取用、取用留痕。
 *
 * 安全语义（与 MCP credentials 域同一 core 服务，零分叉）：
 * - 列表永不回显值；值只在显式「揭示/复制」后返回，且每次留取用痕（read_count+1、流水+1）；
 * - 揭示后 30 秒倒计时自动遮蔽——明文不挂在屏幕上过夜；
 * - 同名换值清零旧取用审计（值变了旧痕作废）——换值区内明说；
 * - 删除级联清流水，需输入凭据名确认——删机密的分量要对。
 *
 * 布局：工单式双栏（对齐工单/待办/资产页）——左台账列表（搜索+标签筛选+健康徽章），
 * 右详情（值保险箱 + 元信息 + 取用流水时间线 + 换值 + 危险区）。
 */
import { useCallback, useEffect, useState } from 'react'
import { api } from '@/lib/api'
import { cn } from '@/lib/utils'
import { copyText } from '@/lib/utils'
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
  /** 分组标签（按系统/环境归组） */
  tags: string[]
  /** 到期时间（过期红/临期黄） */
  expires_at: string | null
}

export interface CredentialReadRow {
  id: string
  credential_id: string
  reader: string
  read_at: string
}

/** 到期健康度：expired / expiring（30 天内）/ ok / none */
function health(c: CredentialMetaDto): 'expired' | 'expiring' | 'ok' | 'none' {
  if (!c.expires_at) return 'none'
  const t = new Date(c.expires_at).getTime()
  if (t < Date.now()) return 'expired'
  if (t - Date.now() < 30 * 86400_000) return 'expiring'
  return 'ok'
}

function HealthBadge({ c }: { c: CredentialMetaDto }) {
  const h = health(c)
  if (h === 'expired')
    return <span className="rounded bg-red-500/15 px-1.5 py-0.5 text-xs text-red-400">已过期</span>
  if (h === 'expiring')
    return <span className="rounded bg-yellow-500/15 px-1.5 py-0.5 text-xs text-yellow-500">30 天内到期</span>
  return null
}

function relTime(ts: string): string {
  const s = Math.floor((Date.now() - new Date(ts).getTime()) / 1000)
  if (s < 60) return '刚刚'
  if (s < 3600) return `${Math.floor(s / 60)} 分钟前`
  if (s < 86400) return `${Math.floor(s / 3600)} 小时前`
  return `${Math.floor(s / 86400)} 天前`
}

export default function Credentials() {
  const [rows, setRows] = useState<CredentialMetaDto[] | null>(null)
  const [err, setErr] = useState('')
  const [notice, setNotice] = useState('')
  const [busy, setBusy] = useState(false)
  // 选中详情（按名）；左列表筛选
  const [selected, setSelected] = useState<string | null>(null)
  const [q, setQ] = useState('')
  const [filterTag, setFilterTag] = useState('')
  // 揭示（只对选中项、只在显式点击后；30s 倒计时自动遮蔽）
  const [reveal, setReveal] = useState<{ name: string; value: string } | null>(null)
  const [countdown, setCountdown] = useState(0)
  // 新建弹窗
  const [showCreate, setShowCreate] = useState(false)

  const load = useCallback(() => {
    api
      .get<{ items: CredentialMetaDto[] }>('/credentials')
      .then((r) => setRows(r.items))
      .catch((e) => setErr(String(e)))
  }, [])
  useEffect(() => {
    load()
  }, [load])

  // 揭示倒计时：到点自动遮蔽
  useEffect(() => {
    if (!reveal) return
    setCountdown(30)
    const t = setInterval(() => {
      setCountdown((s) => {
        if (s <= 1) {
          clearInterval(t)
          setReveal(null)
          return 0
        }
        return s - 1
      })
    }, 1000)
    return () => clearInterval(t)
  }, [reveal])

  // 切换选中时，未遮蔽的明文立即收起
  useEffect(() => {
    setReveal(null)
  }, [selected])

  const cur = rows?.find((r) => r.name === selected) ?? null

  const filtered = (rows ?? []).filter((c) => {
    if (filterTag && !c.tags.includes(filterTag)) return false
    if (!q.trim()) return true
    const k = q.trim().toLowerCase()
    return (
      c.name.toLowerCase().includes(k) ||
      (c.description ?? '').toLowerCase().includes(k) ||
      c.tags.some((t) => t.toLowerCase().includes(k))
    )
  })

  const posture = {
    total: rows?.length ?? 0,
    expiring: (rows ?? []).filter((c) => health(c) === 'expiring').length,
    expired: (rows ?? []).filter((c) => health(c) === 'expired').length,
    neverRead: (rows ?? []).filter((c) => c.read_count === 0).length,
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
      setReveal({ name, value: r.value })
      load() // read_count/last_read_at 变了，刷台账
    }
  }

  async function doCopy(name: string) {
    setBusy(true)
    setErr('')
    const r = await api
      .get<{ value: string }>(`/credentials/${encodeURIComponent(name)}/value`)
      .catch((e) => {
        setErr(String(e))
        return null
      })
    setBusy(false)
    if (!r) return
    const ok = await copyText(r.value)
    load() // 复制也是取用，留痕
    setNotice(
      ok
        ? `已复制「${name}」的值（取用已留痕）——建议尽快粘贴，别让明文在剪贴板过夜`
        : `取值成功但剪贴板不可用（非安全上下文）——请用「揭示」手动复制`,
    )
  }

  const allTags = [...new Set((rows ?? []).flatMap((c) => c.tags))]

  if (err && !rows) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  return (
    <div className="space-y-4">
      <PageHeader title="凭据">
        <Button onClick={() => setShowCreate(true)}>＋ 新建凭据</Button>
      </PageHeader>
      {err && <ErrorBox msg={err} />}
      {notice && (
        <div className="rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-300">
          {notice}
        </div>
      )}

      {rows.length === 0 ? (
        <Empty text="台账空——点右上角「＋ 新建凭据」写入第一条机密。" />
      ) : (
        <>
          {/* 态势条：一眼健康度 */}
          <Card className="flex flex-wrap items-center gap-x-6 gap-y-1 px-4 py-2.5 text-sm">
            <span className="text-muted-foreground">
              共 <span className="font-medium text-foreground">{posture.total}</span> 条
            </span>
            <span className={posture.expiring > 0 ? 'text-yellow-500' : 'text-muted-foreground'}>
              临期 {posture.expiring}
            </span>
            <span className={posture.expired > 0 ? 'text-red-400' : 'text-muted-foreground'}>
              过期 {posture.expired}
            </span>
            <span className={posture.neverRead > 0 ? 'text-muted-foreground' : 'text-muted-foreground'}>
              从未取用 {posture.neverRead}
            </span>
          </Card>

          {/* 工单式双栏：左台账右详情，各自独立滚动 */}
          <div
            className={cn(
              'grid grid-cols-1 gap-4',
              cur ? 'lg:grid-cols-[minmax(0,1fr)_minmax(0,26rem)] lg:h-[calc(100dvh-9.5rem)]' : 'grid-cols-1',
            )}
          >
            {/* 左：列表 */}
            <div className={cn('min-w-0 space-y-2', cur && 'lg:h-full lg:overflow-y-auto lg:pr-1')}>
              <div className="flex flex-wrap items-center gap-2">
                <input
                  className={inputCls + ' w-64'}
                  placeholder="搜索名称 / 说明 / 标签…"
                  value={q}
                  onChange={(e) => setQ(e.target.value)}
                  aria-label="搜索凭据"
                />
                {filterTag && (
                  <button
                    className="rounded-full border border-primary/50 bg-primary/10 px-2 py-0.5 text-xs"
                    onClick={() => setFilterTag('')}
                  >
                    ✕ {filterTag}
                  </button>
                )}
                {allTags
                  .filter((t) => t !== filterTag)
                  .map((t) => (
                    <button
                      key={t}
                      className="rounded-full border border-border px-2 py-0.5 text-xs text-muted-foreground hover:bg-muted"
                      onClick={() => setFilterTag(t)}
                    >
                      {t}
                    </button>
                  ))}
              </div>
              {filtered.length === 0 ? (
                <Empty text={q || filterTag ? '没有匹配的凭据——换个关键词试试。' : '台账空。'} />
              ) : (
                filtered.map((c) => {
                  const sel = c.name === selected
                  return (
                    <button
                      key={c.id}
                      className={cn(
                        'block w-full rounded-lg border p-3 text-left transition-colors',
                        sel ? 'border-primary/60 bg-primary/10' : 'border-border hover:bg-muted/40',
                      )}
                      onClick={() => setSelected(sel ? null : c.name)}
                    >
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="font-mono text-sm font-medium break-all">{c.name}</span>
                        {c.sensitive && (
                          <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-xs text-amber-400">敏感</span>
                        )}
                        <HealthBadge c={c} />
                        <span className="ml-auto text-xs text-muted-foreground">取用 {c.read_count}</span>
                      </div>
                      {c.description && (
                        <div className="mt-0.5 truncate text-xs text-muted-foreground">{c.description}</div>
                      )}
                      {c.tags.length > 0 && (
                        <div className="mt-1.5 flex flex-wrap gap-1">
                          {c.tags.map((t) => (
                            <span
                              key={t}
                              className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground"
                            >
                              {t}
                            </span>
                          ))}
                        </div>
                      )}
                    </button>
                  )
                })
              )}
            </div>

            {/* 右：详情 */}
            {cur && (
              <div className="min-w-0 space-y-3 lg:self-start lg:max-h-full lg:overflow-y-auto lg:overflow-x-hidden lg:pl-1">
                <Card className="p-4">
                  <div className="flex items-start justify-between gap-2">
                    <div className="min-w-0">
                      <div className="font-mono text-sm font-medium break-all">{cur.name}</div>
                      <div className="mt-0.5 text-xs text-muted-foreground">{cur.description || '（无说明）'}</div>
                    </div>
                    <button
                      className="shrink-0 rounded p-1 text-muted-foreground hover:bg-muted"
                      onClick={() => setSelected(null)}
                      aria-label="关闭详情"
                    >
                      ✕
                    </button>
                  </div>

                  {/* 值保险箱：默认遮蔽，揭示 30s 自动遮蔽，复制留痕 */}
                  <div className="mt-3 rounded-lg border border-dashed border-border bg-muted/30 p-3">
                    <div className="mb-2 text-xs font-medium text-muted-foreground">值（默认遮蔽）</div>
                    {reveal && reveal.name === cur.name ? (
                      <>
                        <div className="mb-2 font-mono text-sm break-all">{reveal.value}</div>
                        <div className="flex items-center gap-2">
                          <Button
                            size="sm"
                            variant="outline"
                            onClick={() => setReveal(null)}
                          >
                            遮蔽
                          </Button>
                          <span className="text-xs text-yellow-500">{countdown}s 后自动遮蔽</span>
                        </div>
                      </>
                    ) : (
                      <div className="flex items-center gap-2">
                        <span className="font-mono text-sm tracking-widest text-muted-foreground select-none">
                          ••••••••••••
                        </span>
                        <div className="ml-auto flex gap-2">
                          <Button size="sm" variant="outline" disabled={busy} onClick={() => doReveal(cur.name)}>
                            揭示
                          </Button>
                          <Button size="sm" variant="outline" disabled={busy} onClick={() => doCopy(cur.name)}>
                            复制
                          </Button>
                        </div>
                      </div>
                    )}
                  </div>

                  {/* 元信息 */}
                  <dl className="mt-3 grid grid-cols-2 gap-x-4 gap-y-1.5 text-xs">
                    <dt className="text-muted-foreground">创建者</dt>
                    <dd className="font-mono">{cur.created_by}</dd>
                    <dt className="text-muted-foreground">创建</dt>
                    <dd>{new Date(cur.created_at).toLocaleString()}</dd>
                    <dt className="text-muted-foreground">更新</dt>
                    <dd>{new Date(cur.updated_at).toLocaleString()}</dd>
                    <dt className="text-muted-foreground">最近取用</dt>
                    <dd>{cur.last_read_at ? new Date(cur.last_read_at).toLocaleString() : '从未'}</dd>
                    {cur.expires_at && (
                      <>
                        <dt className="text-muted-foreground">到期</dt>
                        <dd>{new Date(cur.expires_at).toLocaleDateString()}</dd>
                      </>
                    )}
                  </dl>
                </Card>

                {/* 取用流水：选中项的完整审计（时间线） */}
                <ReadsTimeline name={cur.name} />

                {/* 换值：折叠区——换值清零旧流水，明说 */}
                <RevalueBox c={cur} onDone={() => {
                  setNotice(`已换值「${cur.name}」——旧取用流水已清零（值变了旧痕作废）`)
                  setReveal(null)
                  load()
                }} onError={setErr} />

                {/* 危险区：输名字确认 */}
                <DangerZone
                  name={cur.name}
                  onDeleted={() => {
                    setSelected(null)
                    setNotice(`已删除「${cur.name}」——取用流水一并清除`)
                    load()
                  }}
                  onError={setErr}
                />
              </div>
            )}
          </div>
        </>
      )}

      {showCreate && (
        <CreateDialog
          onClose={() => setShowCreate(false)}
          onCreated={(name, hint) => {
            setShowCreate(false)
            setNotice(`已写入「${name}」${hint ?? ''}`)
            load()
          }}
          onError={setErr}
        />
      )}
    </div>
  )
}

/** 取用流水时间线：谁·何时（相对+绝对），空态=从未取用。 */
function ReadsTimeline({ name }: { name: string }) {
  const [reads, setReads] = useState<CredentialReadRow[] | null>(null)
  const [err, setErr] = useState('')
  useEffect(() => {
    setReads(null)
    api
      .get<{ reads: CredentialReadRow[] }>(`/credentials/${encodeURIComponent(name)}/reads`)
      .then((r) => setReads(r.reads))
      .catch((e) => setErr(String(e)))
  }, [name])

  return (
    <Card className="p-4">
      <div className="mb-2 text-xs font-medium text-muted-foreground">取用流水（谁 · 何时，揭示与复制都留痕）</div>
      {err && <ErrorBox msg={err} />}
      {reads === null ? (
        <Spinner />
      ) : reads.length === 0 ? (
        <div className="text-xs text-muted-foreground">从未取用——流水在「揭示 / 复制」时生成。</div>
      ) : (
        <ol className="space-y-2 border-l border-border pl-3">
          {reads.map((r) => (
            <li key={r.id} className="relative text-xs">
              <span className="absolute top-1.5 -left-[17px] h-1.5 w-1.5 rounded-full bg-muted-foreground/60" />
              <span className="font-mono">{r.reader}</span>
              <span className="text-muted-foreground">
                {' '}
                · {relTime(r.read_at)} · {new Date(r.read_at).toLocaleString()}
              </span>
            </li>
          ))}
        </ol>
      )}
    </Card>
  )
}

/** 换值折叠区：值变了旧痕作废——进区内明说。 */
function RevalueBox({
  c,
  onDone,
  onError,
}: {
  c: CredentialMetaDto
  onDone: () => void
  onError: (m: string) => void
}) {
  const [value, setValue] = useState('')
  const [expires, setExpires] = useState(
    c.expires_at ? new Date(c.expires_at).toISOString().slice(0, 16) : '',
  )
  const [busy, setBusy] = useState(false)

  async function submit() {
    if (!value) return
    setBusy(true)
    const r = await api
      .post<{ credential?: unknown; hint?: string }>('/credentials', {
        name: c.name,
        value,
        description: c.description,
        tags: c.tags,
        expires_at: expires ? new Date(expires).toISOString() : null,
      })
      .catch((e) => {
        onError(String(e))
        return null
      })
    setBusy(false)
    if (r) {
      setValue('')
      onDone()
    }
  }

  return (
    <details className="rounded-lg border border-border">
      <summary className="cursor-pointer px-4 py-2.5 text-xs font-medium text-muted-foreground hover:bg-muted/40">
        换值（旧取用流水清零）
      </summary>
      <div className="space-y-2 border-t border-border px-4 py-3">
        <div className="text-xs text-muted-foreground">值变了，旧痕作废——流水与次数清零，重新开始记。</div>
        <input
          className={inputCls + ' w-full font-mono'}
          placeholder="新值（写入即加密）"
          value={value}
          onChange={(e) => setValue(e.target.value)}
        />
        <div className="flex items-center gap-2">
          <input
            className={inputCls + ' w-48'}
            type="datetime-local"
            value={expires}
            onChange={(e) => setExpires(e.target.value)}
            aria-label="到期时间"
          />
          <Button size="sm" disabled={busy || !value} onClick={submit}>
            写入新值
          </Button>
        </div>
      </div>
    </details>
  )
}

/** 危险区：删除需输入凭据名确认——删机密的分量要对。 */
function DangerZone({
  name,
  onDeleted,
  onError,
}: {
  name: string
  onDeleted: () => void
  onError: (m: string) => void
}) {
  const [typed, setTyped] = useState('')
  const [busy, setBusy] = useState(false)
  const match = typed.trim() === name

  async function del() {
    if (!match) return
    setBusy(true)
    await api.del(`/credentials/${encodeURIComponent(name)}`).catch((e) => onError(String(e)))
    setBusy(false)
    onDeleted()
  }

  return (
    <div className="rounded-lg border border-red-500/30 bg-red-500/5 p-4">
      <div className="mb-1.5 text-xs font-medium text-red-400">危险区</div>
      <div className="mb-2 text-xs text-muted-foreground">
        删除「{name}」并级联清除全部取用流水，不可恢复。输入凭据名确认：
      </div>
      <div className="flex items-center gap-2">
        <input
          className={inputCls + ' w-full font-mono'}
          placeholder={name}
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
        />
        <Button
          size="sm"
          variant="destructive"
          disabled={busy || !match}
          onClick={del}
        >
          删除
        </Button>
      </div>
    </div>
  )
}

/** 新建凭据弹窗：顶部常驻表单退役——写入是低频动作，不该常驻页面。 */
function CreateDialog({
  onClose,
  onCreated,
  onError,
}: {
  onClose: () => void
  onCreated: (name: string, hint?: string) => void
  onError: (m: string) => void
}) {
  const [name, setName] = useState('')
  const [value, setValue] = useState('')
  const [desc, setDesc] = useState('')
  const [tags, setTags] = useState('')
  const [expires, setExpires] = useState('')
  const [busy, setBusy] = useState(false)

  async function submit() {
    if (!name || !value) return
    setBusy(true)
    const r = await api
      .post<{ credential?: unknown; hint?: string }>('/credentials', {
        name,
        value,
        description: desc || null,
        tags: tags
          .split(',')
          .map((s) => s.trim())
          .filter(Boolean),
        expires_at: expires ? new Date(expires).toISOString() : null,
      })
      .catch((e) => {
        onError(String(e))
        return null
      })
    setBusy(false)
    if (r) onCreated(name, r.hint)
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      onClick={onClose}
      role="presentation"
    >
      <Card
        className="w-full max-w-md space-y-3 p-4"
        onClick={(e: React.MouseEvent) => e.stopPropagation()}
      >
        <div className="text-sm font-medium">新建凭据</div>
        <input
          className={inputCls + ' w-full font-mono'}
          placeholder="名称（如 newapi/api_key）"
          value={name}
          onChange={(e) => setName(e.target.value)}
          autoFocus
        />
        <input
          className={inputCls + ' w-full font-mono'}
          placeholder="值（写入即加密，永不回显于列表）"
          value={value}
          onChange={(e) => setValue(e.target.value)}
        />
        <input
          className={inputCls + ' w-full'}
          placeholder="说明（可选）"
          value={desc}
          onChange={(e) => setDesc(e.target.value)}
        />
        <input
          className={inputCls + ' w-full'}
          placeholder="标签（逗号分隔，如 prod,newapi）"
          value={tags}
          onChange={(e) => setTags(e.target.value)}
        />
        <input
          className={inputCls + ' w-48'}
          type="datetime-local"
          value={expires}
          onChange={(e) => setExpires(e.target.value)}
          aria-label="到期时间（可选）"
        />
        <div className="flex items-center justify-between gap-2">
          <span className="text-xs text-muted-foreground">同名换值：旧取用流水清零（值变了旧痕作废）</span>
          <div className="flex shrink-0 gap-2">
            <Button variant="outline" onClick={onClose}>
              取消
            </Button>
            <Button disabled={busy || !name || !value} onClick={submit}>
              写入
            </Button>
          </div>
        </div>
      </Card>
    </div>
  )
}

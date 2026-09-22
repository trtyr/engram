/**
 * 资产台账页（2026-09-21 新增）：我拥有的、可以被操作的东西——主机 / 云实例 / 域名 / 账号 / 设备。
 *
 * 语义（《项目与资产模型 · README》§2.7）：资产是**档案**（它是什么），身份唯一、无「收尾」；
 * 项目通过位置登记**引用**它（不在项目里重抄身份）。所以本页右栏的「被哪些项目用到」就是那层引用。
 */
import { useCallback, useEffect, useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import { api, type AssetDetailDto, type AssetDto, type AssetKindDto } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner } from '@/components/ui-bits'
import { inputCls, selectCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'

/** 类型色点（标签文字由 `/assets/types` 提供——值域单一事实源在后端 ASSET_KINDS 常量）。 */
const KIND_DOT: Record<string, string> = {
  host: '#3b82f6', // 主机
  cloud: '#8b5cf6', // 云实例
  domain: '#f59e0b', // 域名
  account: '#10b981', // 账号
  device: '#06b6d4', // 设备
  other: '#6b7280', // 其他
}

export default function Assets() {
  const [rows, setRows] = useState<AssetDto[] | null>(null)
  const [kinds, setKinds] = useState<AssetKindDto[]>([])
  const [filter, setFilter] = useState('')
  const [q, setQ] = useState('')
  const [selId, setSelId] = useState<string | null>(null)
  const [detail, setDetail] = useState<AssetDetailDto | null>(null)
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)

  // 建档表单
  const [kind, setKind] = useState('host')
  const [name, setName] = useState('')
  const [aliases, setAliases] = useState('')
  const [ip, setIp] = useState('')
  const [os, setOs] = useState('')
  const [note, setNote] = useState('')

  // 编辑表单（右栏内联）
  const [editing, setEditing] = useState(false)
  const [eKind, setEKind] = useState('host')
  const [eName, setEName] = useState('')
  const [eAliases, setEAliases] = useState('')
  const [eIp, setEIp] = useState('')
  const [eOs, setEOs] = useState('')
  const [eNote, setENote] = useState('')

  const load = useCallback(() => {
    const parts: string[] = []
    if (filter) parts.push(`kind=${encodeURIComponent(filter)}`)
    if (q.trim()) parts.push(`q=${encodeURIComponent(q.trim())}`)
    const qs = parts.length ? `?${parts.join('&')}` : ''
    return api
      .get<AssetDto[]>(`/assets${qs}`)
      .then((r) => {
        setRows(r)
        setErr('')
        return r
      })
      .catch((e) => {
        setErr(e instanceof Error ? e.message : '加载失败')
        return []
      })
  }, [filter, q])

  useEffect(() => {
    api.get<AssetKindDto[]>('/assets/types').then(setKinds).catch(() => {})
  }, [])

  useEffect(() => {
    load()
  }, [load])

  // 选中项的详情（含「被哪些项目用到」反查）
  const openDetail = useCallback((id: string) => {
    setSelId(id)
    setEditing(false)
    api
      .get<AssetDetailDto>(`/assets/${id}`)
      .then(setDetail)
      .catch((e) => setErr(e instanceof Error ? e.message : '读取详情失败'))
  }, [])

  // 首屏自动选中第一条
  useEffect(() => {
    if (!selId && rows && rows.length > 0) openDetail(rows[0].id)
  }, [rows, selId, openDetail])

  const kindLabel = useMemo(
    () => (k: string) => kinds.find((x) => x.kind === k)?.label ?? k,
    [kinds],
  )

  async function doCreate() {
    if (!name.trim()) return
    setBusy(true)
    try {
      await api.post('/assets', {
        kind,
        name: name.trim(),
        aliases: aliases.split(',').map((s) => s.trim()).filter(Boolean),
        ip: ip.trim(),
        os: os.trim(),
        note: note.trim(),
      })
      setName('')
      setAliases('')
      setIp('')
      setOs('')
      setNote('')
      await load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '建档失败')
    } finally {
      setBusy(false)
    }
  }

  async function doSave() {
    if (!detail || !eName.trim()) return
    setBusy(true)
    try {
      await api.put(`/assets/${detail.id}`, {
        kind: eKind,
        name: eName.trim(),
        aliases: eAliases.split(',').map((s) => s.trim()).filter(Boolean),
        ip: eIp.trim(),
        os: eOs.trim(),
        note: eNote.trim(),
      })
      setEditing(false)
      await load()
      openDetail(detail.id)
    } catch (e) {
      setErr(e instanceof Error ? e.message : '保存失败')
    } finally {
      setBusy(false)
    }
  }

  async function doDelete(id: string) {
    setBusy(true)
    try {
      await api.del(`/assets/${id}`)
      if (selId === id) {
        setSelId(null)
        setDetail(null)
      }
      await load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '删除失败')
    } finally {
      setBusy(false)
    }
  }

  if (!rows) return <Spinner />

  return (
    <div className="space-y-5">
      <PageHeader
        title="资产"
        desc="台账：我拥有的、可以被操作的东西（主机 / 云实例 / 域名 / 账号 / 设备）。身份唯一——项目只**引用**它，不重抄一遍。"
      >
        <div className="flex items-center gap-2">
          <input
            className={`${inputCls} w-40`}
            placeholder="搜名称 / 别名 / IP"
            value={q}
            onChange={(e) => setQ(e.target.value)}
          />
          <select className={selectCls} value={filter} onChange={(e) => setFilter(e.target.value)}>
            <option value="">全部类型</option>
            {kinds.map((k) => (
              <option key={k.kind} value={k.kind}>
                {k.label}
              </option>
            ))}
          </select>
        </div>
      </PageHeader>

      <Card className="p-3">
        <form
          className="flex flex-wrap items-center gap-2"
          onSubmit={(e) => {
            e.preventDefault()
            doCreate()
          }}
        >
          <select className={selectCls} value={kind} onChange={(e) => setKind(e.target.value)}>
            {kinds.map((k) => (
              <option key={k.kind} value={k.kind}>
                {k.label}
              </option>
            ))}
          </select>
          <input
            className={`${inputCls} w-48`}
            placeholder="台账名（如 MacBook Air M1）"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
          <input
            className={`${inputCls} w-56`}
            placeholder="别名（逗号分隔，如 trtyr-mac, tencent-beijing）"
            value={aliases}
            onChange={(e) => setAliases(e.target.value)}
          />
          <input
            className={`${inputCls} w-40`}
            placeholder="IP（可选）"
            value={ip}
            onChange={(e) => setIp(e.target.value)}
          />
          <input
            className={`${inputCls} w-32`}
            placeholder="系统（可选）"
            value={os}
            onChange={(e) => setOs(e.target.value)}
          />
          <input
            className={`${inputCls} flex-1`}
            placeholder="备注（规格 / 位置 / 用途线索，可选）"
            value={note}
            onChange={(e) => setNote(e.target.value)}
          />
          <Button size="sm" type="submit" disabled={busy || !name.trim()}>
            建档
          </Button>
        </form>
      </Card>

      {err && <ErrorBox msg={err} />}

      {rows.length === 0 ? (
        <Empty text="台账还是空的——上面建第一台资产，或先跑 MCP assets add" />
      ) : (
        <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.15fr)]">
          {/* 左：台账列表 */}
          <div className="space-y-2">
            {rows.map((a) => (
              <button
                key={a.id}
                type="button"
                onClick={() => openDetail(a.id)}
                className={`w-full rounded-lg border px-3 py-2 text-left transition-colors ${
                  selId === a.id
                    ? 'border-foreground/30 bg-muted/40'
                    : 'border-border bg-card hover:border-foreground/20'
                }`}
              >
                <div className="flex items-center gap-2">
                  <i
                    className="size-2 shrink-0 rounded-full"
                    style={{ background: KIND_DOT[a.kind] ?? '#6b7280' }}
                    aria-hidden="true"
                  />
                  <span className="truncate text-sm font-medium">{a.name}</span>
                  <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">
                    {kindLabel(a.kind)}
                  </span>
                  {a.ip && (
                    <span className="ml-auto shrink-0 font-mono text-[11px] text-muted-foreground">
                      {a.ip}
                    </span>
                  )}
                </div>
                {a.aliases.length > 0 && (
                  <div className="mt-1 truncate text-[11px] text-muted-foreground">
                    别名：{a.aliases.join('、')}
                  </div>
                )}
              </button>
            ))}
          </div>

          {/* 右：详情 + 被谁引用 */}
          <div>
            {!detail ? (
              <Empty text="选左边一条看详情与引用" />
            ) : (
              <Card className="space-y-3 p-4">
                {!editing ? (
                  <>
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0">
                        <div className="flex items-center gap-2">
                          <i
                            className="size-2 shrink-0 rounded-full"
                            style={{ background: KIND_DOT[detail.kind] ?? '#6b7280' }}
                            aria-hidden="true"
                          />
                          <h3 className="truncate font-medium">{detail.name}</h3>
                          <span className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">
                            {kindLabel(detail.kind)}
                          </span>
                        </div>
                        <div className="mt-1 space-y-0.5 text-xs text-muted-foreground">
                          {detail.ip && (
                            <div>
                              IP：<span className="font-mono">{detail.ip}</span>
                            </div>
                          )}
                          {detail.os && <div>系统：{detail.os}</div>}
                          {detail.aliases.length > 0 && <div>别名：{detail.aliases.join('、')}</div>}
                          {detail.note && <div>备注：{detail.note}</div>}
                        </div>
                      </div>
                      <div className="flex shrink-0 gap-1">
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() => {
                            setEditing(true)
                            setEKind(detail.kind)
                            setEName(detail.name)
                            setEAliases(detail.aliases.join(', '))
                            setEIp(detail.ip)
                            setEOs(detail.os)
                            setENote(detail.note)
                          }}
                        >
                          编辑
                        </Button>
                        <Button
                          size="sm"
                          variant="ghost"
                          disabled={busy}
                          onClick={() => doDelete(detail.id)}
                        >
                          删除
                        </Button>
                      </div>
                    </div>

                    <div className="border-t border-border/60 pt-3">
                      <div className="mb-2 text-xs font-medium text-muted-foreground">
                        被哪些项目用到（{detail.used_by.length}）
                      </div>
                      {detail.used_by.length === 0 ? (
                        <p className="text-xs text-muted-foreground">
                          还没有项目引用它——在项目详情页登记位置时选这条资产即可。
                        </p>
                      ) : (
                        <ul className="space-y-1.5">
                          {detail.used_by.map((u) => (
                            <li key={u.location_id} className="text-xs">
                              <Link
                                to={`/projects/${u.project_id}`}
                                className="font-medium text-info hover:underline"
                              >
                                {u.project_name}
                              </Link>
                              <span className="text-muted-foreground">
                                {' '}
                                · {u.host}
                                {u.path && ` · ${u.path}`}
                                {u.purpose && ` · ${u.purpose}`}
                              </span>
                            </li>
                          ))}
                        </ul>
                      )}
                    </div>
                  </>
                ) : (
                  <div className="space-y-2">
                    <div className="flex gap-2">
                      <select className={selectCls} value={eKind} onChange={(e) => setEKind(e.target.value)}>
                        {kinds.map((k) => (
                          <option key={k.kind} value={k.kind}>
                            {k.label}
                          </option>
                        ))}
                      </select>
                      <input
                        className={`${inputCls} flex-1`}
                        value={eName}
                        onChange={(e) => setEName(e.target.value)}
                      />
                    </div>
                    <input
                      className={`${inputCls} w-full`}
                      placeholder="别名（逗号分隔）"
                      value={eAliases}
                      onChange={(e) => setEAliases(e.target.value)}
                    />
                    <div className="flex gap-2">
                      <input
                        className={`${inputCls} w-44`}
                        placeholder="IP"
                        value={eIp}
                        onChange={(e) => setEIp(e.target.value)}
                      />
                      <input
                        className={`${inputCls} flex-1`}
                        placeholder="系统"
                        value={eOs}
                        onChange={(e) => setEOs(e.target.value)}
                      />
                    </div>
                    <input
                      className={`${inputCls} w-full`}
                      placeholder="备注"
                      value={eNote}
                      onChange={(e) => setENote(e.target.value)}
                    />
                    <div className="flex gap-2">
                      <Button size="sm" disabled={busy} onClick={doSave}>
                        保存
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => setEditing(false)}>
                        取消
                      </Button>
                    </div>
                  </div>
                )}
              </Card>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

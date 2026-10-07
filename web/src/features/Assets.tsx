/**
 * 资产台账页（2026-09-21 新增）：我拥有的、可以被操作的东西——主机 / 云实例 / 域名 / 账号 / 设备。
 *
 * 语义（《项目与资产模型 · README》§2.7）：资产是**档案**（它是什么），身份唯一、无「收尾」；
 * 项目通过位置登记**引用**它（不在项目里重抄身份）。所以本页右栏的「被哪些项目用到」就是那层引用。
 */
import { useCallback, useEffect, useMemo, useState } from 'react'
import { cn } from '@/lib/utils'
import { Link } from 'react-router-dom'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { api, type AssetDetailDto, type AssetDto, type AssetKindDto, type AssetRevisionDto } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner } from '@/components/ui-bits'
import { inputCls, selectCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import { Box, Cloud, Globe, IdCard, Server, Smartphone } from 'lucide-react'

/** 类型色点（标签文字由 `/assets/types` 提供——值域单一事实源在后端 ASSET_KINDS 常量）。 */
const KIND_DOT: Record<string, string> = {
  host: '#3b82f6', // 主机
  cloud: '#8b5cf6', // 云实例
  domain: '#f59e0b', // 域名
  account: '#10b981', // 账号
  device: '#06b6d4', // 设备
  other: '#6b7280', // 其他
}

/** 类型图标（与 KIND_DOT 同源着色）。 */
const KIND_ICON: Record<string, typeof Server> = {
  host: Server,
  cloud: Cloud,
  domain: Globe,
  account: IdCard,
  device: Smartphone,
  other: Box,
}

/** 类型图标瓦片：列表行的视觉锚点——色底色字，一眼认类型。 */
function AssetTile({ kind, size = 'md' }: { kind: string; size?: 'md' | 'lg' }) {
  const color = KIND_DOT[kind] ?? '#6b7280'
  const Icon = KIND_ICON[kind] ?? Box
  return (
    <span
      className={cn(
        'flex shrink-0 items-center justify-center rounded-lg',
        size === 'md' ? 'size-9' : 'size-11',
      )}
      style={{ background: `${color}1f`, color }}
    >
      <Icon className={size === 'md' ? 'size-4' : 'size-5'} aria-hidden="true" />
    </span>
  )
}

export default function Assets() {
  const [rows, setRows] = useState<AssetDto[] | null>(null)
  const [kinds, setKinds] = useState<AssetKindDto[]>([])
  const [filter, setFilter] = useState('')
  const [q, setQ] = useState(() => new URLSearchParams(window.location.search).get('q') ?? '')
  const [selId, setSelId] = useState<string | null>(null)
  const [detail, setDetail] = useState<AssetDetailDto | null>(null)
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)

  // 编辑表单（右栏内联）
  const [editing, setEditing] = useState(false)
  // 运行手册（0062）：查看/编辑/修订史
  const [rbEditing, setRbEditing] = useState(false)
  const [rbMd, setRbMd] = useState('')
  const [rbVersions, setRbVersions] = useState<AssetRevisionDto[] | null>(null)
  const [rbShowVersions, setRbShowVersions] = useState(false)
  // fields 运维字段编辑态（键值对；保存整体替换）
  const [eFields, setEFields] = useState<[string, string][]>([])
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
        fields: Object.fromEntries(eFields.filter(([k]) => k.trim())),
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

  // 运行手册：修订史开关（懒加载）
  async function loadVersions(id: string) {
    if (rbShowVersions) {
      setRbShowVersions(false)
      return
    }
    setRbShowVersions(true)
    setRbVersions(null)
    const r = await api
      .get<{ versions: AssetRevisionDto[] }>(`/assets/${id}/runbook/versions`)
      .catch(() => null)
    setRbVersions(r?.versions ?? [])
  }

  // 运行手册：保存（旧文自动入修订史）
  async function doSaveRunbook() {
    if (!detail) return
    setBusy(true)
    try {
      await api.put(`/assets/${detail.id}/runbook`, { md: rbMd, editor: 'console' })
      setRbEditing(false)
      await openDetail(detail.id)
    } catch (e) {
      setErr(e instanceof Error ? e.message : '手册保存失败')
    } finally {
      setBusy(false)
    }
  }

  // 运行手册：回滚到某修订（回滚前正文也会入史）
  async function doRestoreRunbook(id: string, versionId: string) {
    setBusy(true)
    try {
      await api.post(`/assets/${id}/runbook/restore`, { version_id: versionId, editor: 'console' })
      await openDetail(id)
      setRbVersions(null)
      setRbShowVersions(false)
    } catch (e) {
      setErr(e instanceof Error ? e.message : '回滚失败')
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
      <PageHeader title="资产" desc={`${rows.length} 项资产`}>
        <div className="flex items-center gap-2">
          <input
            className={`${inputCls} w-64`}
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

      {err && <ErrorBox msg={err} />}

      {rows.length === 0 ? (
        <Empty text="台账还是空的——让 AI 通过 MCP assets add 建第一台资产" />
      ) : (
        <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.15fr)] lg:h-[calc(100dvh-7.5rem)]">
          {/* 左：台账列表（区域滚动，页面框架不动） */}
          <div className="min-h-0 space-y-2 lg:h-full lg:overflow-y-auto lg:pr-1">
            {rows.map((a) => (
              <button
                key={a.id}
                type="button"
                onClick={() => openDetail(a.id)}
                className={cn(
                  'flex w-full items-center gap-3 rounded-lg border px-3 py-2.5 text-left transition-colors',
                  selId === a.id
                    ? 'border-primary/45 bg-primary/5 shadow-[inset_2px_0_0_0] shadow-primary/50'
                    : 'border-border hover:border-foreground/20 hover:bg-muted/40',
                )}
              >
                <AssetTile kind={a.kind} />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="truncate text-sm font-medium">{a.name}</span>
                    <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">
                      {kindLabel(a.kind)}
                    </span>
                  </div>
                  <div className="mt-0.5 flex items-center gap-2 text-xs text-muted-foreground">
                    {a.ip ? (
                      <span className="truncate font-mono">{a.ip}</span>
                    ) : (
                      <span className="truncate">{a.aliases.length > 0 ? a.aliases.join('、') : '—'}</span>
                    )}
                  </div>
                </div>
              </button>
            ))}
          </div>

          {/* 右：详情 + 被谁引用（区域滚动） */}
          <div className="min-h-0 lg:h-full lg:overflow-y-auto lg:overflow-x-hidden lg:pl-1">
            {!detail ? (
              <Empty text="选左边一条看详情与引用" />
            ) : (
              <Card className="space-y-3 p-4">
                {!editing ? (
                  <>
                    <div className="flex items-start justify-between gap-3">
                      <div className="flex min-w-0 items-start gap-3">
                        <AssetTile kind={detail.kind} size="lg" />
                        <div className="min-w-0">
                          <div className="flex items-center gap-2">
                            <h3 className="truncate font-medium">{detail.name}</h3>
                            <span className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">
                              {kindLabel(detail.kind)}
                            </span>
                          </div>
                          <dl className="mt-1.5 grid grid-cols-[3.5rem_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
                            {detail.ip && (
                              <>
                                <dt className="text-muted-foreground">IP</dt>
                                <dd className="truncate font-mono">{detail.ip}</dd>
                              </>
                            )}
                            {detail.os && (
                              <>
                                <dt className="text-muted-foreground">系统</dt>
                                <dd className="truncate">{detail.os}</dd>
                              </>
                            )}
                            {detail.aliases.length > 0 && (
                              <>
                                <dt className="text-muted-foreground">别名</dt>
                                <dd className="truncate">{detail.aliases.join('、')}</dd>
                              </>
                            )}
                            {detail.note && (
                              <>
                                <dt className="text-muted-foreground">备注</dt>
                                <dd className="truncate">{detail.note}</dd>
                              </>
                            )}
                            {Object.entries(
                              (detail.fields ?? {}) as Record<string, unknown>,
                            ).map(([k, v]) => (
                              <span key={k} className="contents">
                                <dt className="text-muted-foreground">{k}</dt>
                                <dd className="truncate font-mono">{v == null ? '' : String(v)}</dd>
                              </span>
                            ))}
                          </dl>
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
                            setEFields(
                              Object.entries(
                                (detail.fields ?? {}) as Record<string, unknown>,
                              ).map(([k, v]) => [k, v == null ? '' : String(v)]),
                            )
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
                      <div className="mb-2 flex items-center justify-between">
                        <div className="text-xs font-medium text-muted-foreground">
                          运行手册（Markdown · 主动记录）
                        </div>
                        <div className="flex gap-1">
                          {rbEditing ? (
                            <>
                              <Button size="sm" disabled={busy} onClick={doSaveRunbook}>
                                保存手册
                              </Button>
                              <Button size="sm" variant="ghost" onClick={() => setRbEditing(false)}>
                                取消
                              </Button>
                            </>
                          ) : (
                            <>
                              <Button
                                size="sm"
                                variant="ghost"
                                onClick={() => {
                                  setRbMd(detail.runbook_md)
                                  setRbEditing(true)
                                }}
                              >
                                编辑手册
                              </Button>
                              <Button
                                size="sm"
                                variant="ghost"
                                onClick={() => void loadVersions(detail.id)}
                              >
                                {rbShowVersions ? '收起版本' : '版本史'}
                              </Button>
                            </>
                          )}
                        </div>
                      </div>
                      {rbEditing ? (
                        <textarea
                          className={`${inputCls} h-64 w-full font-mono text-xs`}
                          value={rbMd}
                          onChange={(e) => setRbMd(e.target.value)}
                          placeholder="# 主机手册（Markdown）——硬件 / 网络 / 服务 / 端口 / 变更 / 踩坑"
                        />
                      ) : detail.runbook_md ? (
                        <div className="md-body max-w-none text-sm">
                          <ReactMarkdown remarkPlugins={[remarkGfm]}>
                            {detail.runbook_md}
                          </ReactMarkdown>
                        </div>
                      ) : (
                        <p className="text-xs text-muted-foreground">
                          还没有手册——点「编辑手册」开始记录（硬件/网络/服务/端口/变更/踩坑）。
                        </p>
                      )}
                      {rbShowVersions && (
                        <div className="mt-2 rounded-md border border-border/60 bg-muted/30 p-2 text-xs">
                          {rbVersions === null ? (
                            <span className="text-muted-foreground">加载中…</span>
                          ) : rbVersions.length === 0 ? (
                            <span className="text-muted-foreground">
                              暂无修订——首次改动手册后这里会出现旧文。
                            </span>
                          ) : (
                            <ul className="space-y-1">
                              {rbVersions.map((v) => (
                                <li key={v.id} className="space-y-0.5">
                                  <div className="flex items-center justify-between gap-2">
                                    <span className="truncate font-mono">
                                      {new Date(v.created_at).toLocaleString()} · {v.edited_by}
                                    </span>
                                    <Button
                                      size="sm"
                                      variant="ghost"
                                      disabled={busy}
                                      onClick={() => void doRestoreRunbook(detail.id, v.id)}
                                    >
                                      滚到这版
                                    </Button>
                                  </div>
                                  <div className="truncate font-mono text-[11px] text-muted-foreground/80">
                                    旧文：{v.old_runbook_md.split('\n')[0] || '（空）'}
                                  </div>
                                </li>
                              ))}
                            </ul>
                          )}
                        </div>
                      )}
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
                    <div className="space-y-1.5 rounded-md border border-border/60 p-2">
                      <div className="flex items-center justify-between">
                        <span className="text-xs font-medium text-muted-foreground">
                          运维字段（自由 kv：主机名/规格/用途/到期日…）
                        </span>
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() => setEFields((f) => [...f, ['', '']])}
                        >
                          + 字段
                        </Button>
                      </div>
                      {eFields.map(([k, v], i) => (
                        <div key={i} className="flex gap-1.5">
                          <input
                            className={`${inputCls} w-36`}
                            placeholder="字段名"
                            value={k}
                            onChange={(e) =>
                              setEFields((f) =>
                                f.map(([fk, fv], j) => (j === i ? [e.target.value, fv] : [fk, fv])),
                              )
                            }
                          />
                          <input
                            className={`${inputCls} flex-1`}
                            placeholder="值"
                            value={v}
                            onChange={(e) =>
                              setEFields((f) =>
                                f.map(([fk, fv], j) => (j === i ? [fk, e.target.value] : [fk, fv])),
                              )
                            }
                          />
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => setEFields((f) => f.filter((_, j) => j !== i))}
                          >
                            删
                          </Button>
                        </div>
                      ))}
                    </div>
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

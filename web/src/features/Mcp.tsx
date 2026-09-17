/**
 * MCP 管理页：Engram MCP 服务的管理台。
 * 布局：顶部状态条（状态灯 + 端点 + 总开关一行收口）→ 域 Tabs（一个域一个域地看）
 * → 选中域的工具列表（名称可点击展开详情：完整描述 + 参数 Schema；拨杆 + 语义徽标）。
 * 管理语义：总开关关闭 = /mcp 整体 503；工具停用 = 对 AI 隐身 + 调用拒。
 * 描述与 AI 实际收到的 tools/list 同源（含动态资产清单段——如技能库当前有什么）。
 */
import { useEffect, useMemo, useState } from 'react'
import { Check, ChevronDown, ChevronRight, Copy } from 'lucide-react'
import { api, type McpInfo, type McpToolInfo } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Card, ErrorBox, PageHeader, Spinner, Tabs } from '@/components/ui-bits'
import { cn, copyText } from '@/lib/utils'

/** 拨杆开关（墨白：选中即墨底白钮）。 */
function Switch({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean
  onChange: (next: boolean) => void
  label: string
  disabled?: boolean
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        'relative h-[18px] w-8 shrink-0 rounded-full border transition-colors',
        checked ? 'border-foreground bg-foreground' : 'border-border bg-muted',
      )}
    >
      <span
        aria-hidden="true"
        className={cn(
          'absolute left-[2px] top-[2px] size-3 rounded-full transition-transform',
          checked ? 'translate-x-[14px] bg-background' : 'bg-muted-foreground/50',
        )}
      />
    </button>
  )
}

/** 复制按钮：成功短暂打勾，失败明确提示（HTTP 部署下 Clipboard API 缺席，走 execCommand 回退）。 */
function CopyBtn({ text, label }: { text: string; label: string }) {
  const [state, setState] = useState<'idle' | 'done' | 'failed'>('idle')
  const flash = (s: 'done' | 'failed') => {
    setState(s)
    setTimeout(() => setState('idle'), 1500)
  }
  return (
    <Button
      size="sm"
      variant="outline"
      className={state === 'failed' ? 'border-destructive/40 text-destructive' : undefined}
      onClick={async () => {
        flash((await copyText(text)) ? 'done' : 'failed')
      }}
    >
      {state === 'done' ? (
        <Check className="size-3.5" aria-hidden="true" />
      ) : (
        <Copy className="size-3.5" aria-hidden="true" />
      )}
      {state === 'done' ? '已复制' : state === 'failed' ? '复制失败' : label}
    </Button>
  )
}

/** 工具语义徽标（数据驱动文案与颜色，彩色只留语义）。 */
function ToolBadge({ t }: { t: McpToolInfo }) {
  const kind = t.destructive ? '破坏性' : t.read_only ? '只读' : '写入'
  return (
    <span
      className={cn(
        'rounded border px-1 font-mono text-[10px] leading-4',
        t.destructive
          ? 'border-destructive/40 text-destructive'
          : t.read_only
            ? 'border-border text-muted-foreground'
            : 'border-info/40 text-info',
      )}
    >
      {kind}
    </span>
  )
}

/** 参数 Schema 属性视图：逐参数 name/类型/必填/说明；再折叠一份原始 JSON。 */
function ParamList({ schema }: { schema: Record<string, unknown> }) {
  const props = (schema?.properties ?? {}) as Record<
    string,
    { type?: string; description?: string; enum?: string[] }
  >
  const required = (schema.required as string[]) ?? []
  const names = Object.keys(props)
  if (names.length === 0) {
    return <p className="text-xs text-muted-foreground">无参数</p>
  }
  return (
    <div className="space-y-2">
      <ul role="list" className="space-y-2">
        {names.map((n) => {
          const meta = props[n] ?? {}
          return (
            <li key={n} className="rounded border border-border/60 bg-background px-2 py-1.5">
              <p className="flex flex-wrap items-baseline gap-x-2">
                <code className="font-mono text-xs font-medium">{n}</code>
                <span className="font-mono text-[10px] text-muted-foreground">
                  {meta.enum ? meta.enum.join(' | ') : (meta.type ?? 'any')}
                </span>
                {required.includes(n) && (
                  <span className="rounded bg-destructive/10 px-1 font-mono text-[10px] leading-4 text-destructive">
                    必填
                  </span>
                )}
              </p>
              {meta.description && (
                <p className="mt-0.5 text-xs leading-4 text-muted-foreground">{meta.description}</p>
              )}
            </li>
          )
        })}
      </ul>
      <details className="text-xs text-muted-foreground">
        <summary className="cursor-pointer select-none hover:text-foreground">原始 JSON Schema</summary>
        <pre className="mt-1 max-h-64 overflow-auto rounded bg-muted/60 p-2 font-mono text-[10px] leading-4">
          {JSON.stringify(schema, null, 2)}
        </pre>
      </details>
    </div>
  )
}

/** 工具详情（展开行）：完整描述（= AI tools/list 收到的内容）+ 域内操作（单操作开关）+ 调用信封。 */
function ToolDetail({
  t,
  busy,
  onToggleAction,
}: {
  t: McpToolInfo
  busy: boolean
  onToggleAction: (key: string, next: boolean) => void
}) {
  const [openAction, setOpenAction] = useState<string | null>(null)
  return (
    <div className="space-y-3 border-t border-border/60 bg-muted/20 px-3 py-3">
      <div>
        <h4 className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">工具描述</h4>
        <p className="mt-1 whitespace-pre-wrap text-xs leading-5">{t.description || '（无描述）'}</p>
      </div>
      {t.actions.length > 0 && (
        <div>
          <h4 className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
            域内操作（{t.actions.length}）——AI 调用形态 {'{'}
            <code className="font-mono">"action": "…"</code>
            {'}'}
          </h4>
          <ul role="list" className="mt-1 space-y-1.5">
            {t.actions.map((a) => {
              const key = `${t.name}.${a.action}`
              const open = openAction === key
              return (
                <li key={key} className="rounded border border-border/60 bg-background">
                  <div className="flex items-center gap-2 px-2 py-1.5">
                    <button
                      type="button"
                      aria-expanded={open}
                      className="flex min-w-0 flex-1 items-baseline gap-2 text-left"
                      onClick={() => setOpenAction(open ? null : key)}
                    >
                      <code
                        className={cn(
                          'shrink-0 font-mono text-xs font-medium',
                          a.disabled && 'text-muted-foreground/60 line-through',
                        )}
                      >
                        {a.action}
                      </code>
                      {a.destructive && (
                        <span className="shrink-0 rounded border border-destructive/40 px-1 font-mono text-[10px] leading-4 text-destructive">
                          破坏性
                        </span>
                      )}
                      <span className="min-w-0 truncate text-xs text-muted-foreground">{a.summary}</span>
                    </button>
                    <span className="shrink-0">
                      <Switch
                        checked={!a.disabled}
                        disabled={busy}
                        label={`${a.disabled ? '启用' : '停用'} ${key}`}
                        onChange={(next) => onToggleAction(key, next)}
                      />
                    </span>
                  </div>
                  {open && (
                    <div className="border-t border-border/60 px-2 py-2">
                      <ParamList schema={a.parameters} />
                    </div>
                  )}
                </li>
              )
            })}
          </ul>
        </div>
      )}
      {t.actions.length === 0 && t.name === 'search_all' && (
        /* search_all 不是域：单个跨域工具，无 action 目录——说明卡替代操作区，
           避免点开一片空白被误读成「空的/坏了」（工具级开关在本行上方） */
        <div className="rounded-md border border-border/60 bg-muted/30 p-3 text-xs leading-5 text-muted-foreground">
          <p className="font-medium text-foreground">跨域工具——无域内操作目录</p>
          <p className="mt-1">
            它不是域，是单个全局检索工具：一次查询并发 memory / wiki / skills / todos / projects
            五域，各回 top-k 摘要——AI 不确定信息在哪个域时的兜底入口。命中后 AI 再用对应域工具精确取用。
          </p>
          <p className="mt-1">整个工具只有一个开关（上方工具行右侧）；没有 action 级开关，所以这里没有计数与操作列表。</p>
        </div>
      )}
      <div>
        <h4 className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">调用信封参数</h4>
        <div className="mt-1">
          <ParamList schema={t.parameters} />
        </div>
      </div>
    </div>
  )
}

/** 域 key → 中文标签（与侧栏资产域命名同源）。 */
const DOMAIN_LABELS: Record<string, string> = {
  memory: '用户记忆',
  wiki: 'Wiki',
  codegraph: '代码图谱',
  projects: '项目',
  skills: '技能',
  todos: '待办',
  jobs: '异步任务',
  search: '跨域检索', // 后端把 search_all 的 domain 字段写作 "search"（settings/mcp tools[].domain）
}

export default function Mcp() {
  const [info, setInfo] = useState<McpInfo | null>(null)
  const [err, setErr] = useState('')
  const [domain, setDomain] = useState<string>('')
  const [busy, setBusy] = useState(false)
  const [openTool, setOpenTool] = useState<string | null>(null)

  const load = () => {
    api
      .get<McpInfo>('/settings/mcp')
      .then((next) => {
        setInfo(next)
        setErr('')
      })
      .catch((e) => setErr(e.message))
  }
  useEffect(load, [])

  const putConfig = async (body: { enabled?: boolean; disabled_tools?: string[] }) => {
    setBusy(true)
    try {
      setInfo(await api.put<McpInfo>('/settings/mcp', body))
      setErr('')
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  // 按域分组（后端同源 domain 字段；首次出现顺序即展示顺序）
  const domains = useMemo(() => {
    const map = new Map<string, McpToolInfo[]>()
    for (const t of info?.tools ?? []) {
      const list = map.get(t.domain) ?? []
      list.push(t)
      map.set(t.domain, list)
    }
    return [...map.entries()]
  }, [info])

  const currentDomain = domain || domains[0]?.[0] || ''
  const domainTools = domains.find(([d]) => d === currentDomain)?.[1] ?? []
  const domainDisabled = domainTools.filter((t) => info?.disabled_tools.includes(t.name)).length
  // 操作级统计（渐进式发现：域工具下挂 actions）
  const domainActionTotal = domainTools.reduce((n, t) => n + t.actions.length, 0)
  const domainActionDisabled = domainTools.reduce(
    (n, t) =>
      n + t.actions.filter((a) => info?.disabled_tools.includes(`${t.name}.${a.action}`)).length,
    0,
  )
  const mcpUrl = info ? `${window.location.origin}${info.endpoint}` : null

  return (
    <div className="space-y-4">
      <PageHeader title="MCP" desc="AI 接入管理——一个域一个域地收口：能连什么、能调什么" />
      {err && <ErrorBox msg={err} />}

      {info === null && !err ? (
        <Spinner />
      ) : info ? (
        <>
          {/* 状态条：一行收口——状态灯 / 端点 / 协议 / 版本 / 总开关 */}
          <Card
            className={cn(
              'flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3',
              !info.enabled && 'border-destructive/40',
            )}
          >
            <span
              aria-hidden="true"
              className={cn(
                'size-2 shrink-0 rounded-full',
                info.enabled ? 'bg-success engram-pulse' : 'bg-destructive',
              )}
            />
            <p className="text-sm font-semibold">{info.enabled ? '运行中' : '已关闭'}</p>
            <code className="min-w-0 truncate font-mono text-xs text-muted-foreground">{mcpUrl}</code>
            <CopyBtn text={mcpUrl ?? ''} label="复制" />
            <span className="font-mono text-xs text-muted-foreground">
              {info.protocol_version} · v{info.server_version}
            </span>
            <div className="ml-auto">
              <Button
                size="sm"
                variant={info.enabled ? 'destructive' : 'outline'}
                disabled={busy}
                onClick={() => putConfig({ enabled: !info.enabled })}
              >
                {info.enabled ? '关闭服务' : '开启服务'}
              </Button>
            </div>
          </Card>

          {/* 域 Tabs + 选中域的工具列表（点击行展开详情） */}
          <Card className="overflow-hidden">
            {domains.length > 0 && (
              <div className="border-b border-border px-3 py-2">
                <Tabs
                  items={domains.map(([d, tools]) => ({
                    value: d,
                    label: DOMAIN_LABELS[d] ?? d,
                    // search 不是域，是单个跨域工具（search_all）——无 action 目录，不显示计数（显示 0 会被误读成「空的/坏了」）
                    count:
                      d === 'search'
                        ? undefined
                        : tools.reduce((n, t) => n + t.actions.length, 0),
                  }))}
                  value={currentDomain}
                  onChange={setDomain}
                />
              </div>
            )}
            <div className="flex items-center justify-between px-3 py-2">
              <h3 className="text-sm font-semibold">{DOMAIN_LABELS[currentDomain] ?? currentDomain}域工具</h3>
              <p className="font-mono text-xs text-muted-foreground" aria-live="polite">
                操作启用 {domainActionTotal - domainActionDisabled}/{domainActionTotal}
                {domainActionDisabled > 0 && ` · 停用 ${domainActionDisabled}`}
                {domainDisabled > 0 && ' · 整域停用'}
              </p>
            </div>
            <ul role="list">
              {domainTools.map((t) => {
                const disabled = info.disabled_tools.includes(t.name)
                const open = openTool === t.name
                return (
                  <li key={t.name} className="border-b border-border/60 last:border-b-0">
                    <div
                      className={cn(
                        'flex items-center gap-3 px-3 py-2 transition-colors hover:bg-muted/40',
                        open && 'bg-muted/40',
                      )}
                    >
                      <button
                        type="button"
                        aria-expanded={open}
                        className="flex min-w-0 flex-1 items-center gap-2 text-left"
                        onClick={() => setOpenTool(open ? null : t.name)}
                      >
                        {open ? (
                          <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                        ) : (
                          <ChevronRight className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                        )}
                        <code
                          className={cn(
                            'truncate text-xs font-medium',
                            disabled && 'text-muted-foreground/60 line-through',
                          )}
                        >
                          {t.name}
                        </code>
                        <ToolBadge t={t} />
                      </button>
                      <span className="ml-auto shrink-0">
                        <Switch
                          checked={!disabled}
                          disabled={busy}
                          label={`${disabled ? '启用' : '停用'} ${t.name}`}
                          onChange={(next) =>
                            putConfig({
                              disabled_tools: next
                                ? info.disabled_tools.filter((d) => d !== t.name)
                                : [...info.disabled_tools, t.name],
                            })
                          }
                        />
                      </span>
                    </div>
                    {open && (
                      <ToolDetail
                        t={t}
                        busy={busy}
                        onToggleAction={(key, next) =>
                          putConfig({
                            disabled_tools: next
                              ? info.disabled_tools.filter((d) => d !== key)
                              : [...info.disabled_tools, key],
                          })
                        }
                      />
                    )}
                  </li>
                )
              })}
            </ul>
            <p className="border-t border-border bg-muted/30 px-3 py-2 text-[11px] leading-4 text-muted-foreground">
              域工具开关 = 整域对 AI 隐身；展开后可对域内单个操作拨杆（停用操作从 AI 的操作目录与
              help 手册隐身，调用被拒）。点击操作行展开参数 Schema；总开关关闭时整个 /mcp 拒绝连接。
            </p>
          </Card>
        </>
      ) : null}
    </div>
  )
}

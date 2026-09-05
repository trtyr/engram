/**
 * MCP 管理页：Engram MCP 服务的管理台。
 * 布局：顶部状态条（状态灯 + 端点 + 总开关一行收口）→ 双栏：左工具开关列表（密集行 +
 * 拨杆开关，说明收 tooltip），右接入卡（密钥选择 + 客户端 tab + 配置复制）。
 * 管理语义：总开关关闭 = /mcp 整体 503；工具停用 = 对 AI 隐身 + 调用拒。
 */
import { useEffect, useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import { Check, Copy } from 'lucide-react'
import { api, type ApiKey, type McpInfo, type McpToolInfo } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Card, ErrorBox, PageHeader, Spinner, Tabs } from '@/components/ui-bits'
import { cn } from '@/lib/utils'
import { selectCls } from '@/lib/ui'

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

/** 复制按钮：点击后短暂打勾。 */
function CopyBtn({ text, label }: { text: string; label: string }) {
  const [done, setDone] = useState(false)
  return (
    <Button
      size="sm"
      variant="outline"
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(text)
        } catch {
          return
        }
        setDone(true)
        setTimeout(() => setDone(false), 1500)
      }}
    >
      {done ? <Check className="size-3.5" aria-hidden="true" /> : <Copy className="size-3.5" aria-hidden="true" />}
      {done ? '已复制' : label}
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

type ClientId = 'claude-code' | 'cursor' | 'claude-desktop'

const CLIENTS: { value: ClientId; label: string; hint: string }[] = [
  { value: 'claude-code', label: 'Claude Code', hint: '终端运行' },
  { value: 'cursor', label: 'Cursor', hint: 'mcp.json' },
  { value: 'claude-desktop', label: 'Claude Desktop', hint: 'claude_desktop_config.json' },
]

export default function Mcp() {
  const [info, setInfo] = useState<McpInfo | null>(null)
  const [err, setErr] = useState('')
  const [keys, setKeys] = useState<ApiKey[] | null>(null)
  const [selectedKeyId, setSelectedKeyId] = useState<string>('')
  const [client, setClient] = useState<ClientId>('claude-code')
  const [busy, setBusy] = useState(false)

  const load = () => {
    api
      .get<McpInfo>('/settings/mcp')
      .then(setInfo)
      .catch((e) => setErr(e.message))
    api
      .get<ApiKey[]>('/settings/api-keys')
      .then((rows) => setKeys(rows.filter((k) => !k.revoked_at)))
      .catch(() => setKeys([]))
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

  const mcpKeys = keys?.filter((k) => k.scopes.includes('memory')) ?? []
  const selectedKey = mcpKeys.find((k) => k.id === selectedKeyId) ?? mcpKeys[0] ?? null
  const mcpUrl = info ? `${window.location.origin}${info.endpoint}` : null

  const configText = useMemo(() => {
    if (!mcpUrl || !info?.enabled) return null
    switch (client) {
      case 'claude-code':
        return `claude mcp add --transport http engram ${mcpUrl} --header "Authorization: Bearer <KEY>"`
      case 'cursor':
        return JSON.stringify(
          { mcpServers: { engram: { url: mcpUrl, headers: { Authorization: 'Bearer <KEY>' } } } },
          null,
          2,
        )
      case 'claude-desktop':
        return JSON.stringify(
          { mcpServers: { engram: { type: 'http', url: mcpUrl, headers: { Authorization: 'Bearer <KEY>' } } } },
          null,
          2,
        )
    }
  }, [client, mcpUrl, info?.enabled])

  const enabledCount = info ? info.tools.length - info.disabled_tools.length : 0

  return (
    <div className="space-y-4">
      <PageHeader title="MCP" desc="AI 接入管理——能连、能调什么，都在这里收口" />
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

          <div className="grid gap-4 lg:grid-cols-5">
            {/* 左：工具开关列表 */}
            <Card className="overflow-hidden lg:col-span-3">
              <div className="flex items-center justify-between border-b border-border px-3 py-2">
                <h3 className="text-sm font-semibold">工具面</h3>
                <p className="font-mono text-xs text-muted-foreground" aria-live="polite">
                  启用 {enabledCount}/{info.tools.length}
                  {info.disabled_tools.length > 0 && ` · 停用 ${info.disabled_tools.length}`}
                </p>
              </div>
              <ul role="list">
                {info.tools.map((t) => {
                  const disabled = info.disabled_tools.includes(t.name)
                  return (
                    <li
                      key={t.name}
                      className="flex items-center gap-3 border-b border-border/60 px-3 py-2 transition-colors last:border-b-0 hover:bg-muted/40"
                      title={t.description}
                    >
                      <code className={cn('text-xs font-medium', disabled && 'text-muted-foreground/60 line-through')}>
                        {t.name}
                      </code>
                      <ToolBadge t={t} />
                      <span className="ml-auto">
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
                    </li>
                  )
                })}
              </ul>
              <p className="border-t border-border bg-muted/30 px-3 py-2 text-[11px] leading-4 text-muted-foreground">
                停用 = 对 AI 隐身（tools/list 不出现）且调用被拒；悬浮工具名看完整说明。
              </p>
            </Card>

            {/* 右：接入卡（密钥选择 + 客户端 tab + 配置） */}
            <div className="space-y-4 lg:col-span-2">
              <Card className="p-3">
                <div className="flex items-center justify-between gap-2">
                  <h3 className="text-sm font-semibold">接入</h3>
                  <span className="font-mono text-xs text-muted-foreground">
                    memory 密钥 {mcpKeys.length}
                  </span>
                </div>
                {mcpKeys.length === 0 ? (
                  <p className="mt-2 text-xs leading-5 text-muted-foreground">
                    没有 memory scope 的 key——
                    <Link to="/settings" className="underline underline-offset-2 hover:text-foreground">
                      设置 → API 密钥
                    </Link>
                    签发一把再回来。
                  </p>
                ) : (
                  <>
                    <select
                      aria-label="使用密钥"
                      className={cn(selectCls, 'mt-2 w-full text-xs')}
                      value={selectedKey?.id ?? mcpKeys[0].id}
                      onChange={(e) => setSelectedKeyId(e.target.value)}
                    >
                      {mcpKeys.map((k) => (
                        <option key={k.id} value={k.id}>
                          {k.name}（{k.key_prefix}…）
                        </option>
                      ))}
                    </select>
                    <Tabs
                      items={CLIENTS.map((c) => ({ value: c.value, label: c.label }))}
                      value={client}
                      onChange={setClient}
                    />
                    <p className="mt-2 text-[11px] text-muted-foreground">
                      {CLIENTS.find((c) => c.value === client)!.hint}
                    </p>
                    {configText && (
                      <div className="relative mt-2">
                        <pre className="overflow-x-auto rounded bg-muted/50 p-2 pr-16 font-mono text-[11px] leading-4 break-all whitespace-pre-wrap">
                          {configText}
                        </pre>
                        <div className="absolute top-1.5 right-1.5">
                          <CopyBtn text={configText} label="复制" />
                        </div>
                      </div>
                    )}
                    <p className="mt-2 text-[11px] text-muted-foreground">
                      &lt;KEY&gt; 换成密钥明文；
                      <Link to="/settings" className="underline underline-offset-2 hover:text-foreground">
                        签发新 key
                      </Link>
                    </p>
                  </>
                )}
              </Card>
            </div>
          </div>
        </>
      ) : null}
    </div>
  )
}

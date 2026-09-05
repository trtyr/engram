/**
 * MCP 管理页：Engram MCP 服务的管理台。
 * 布局：顶部状态条（状态灯 + 端点 + 总开关一行收口）→ 域 Tabs（一个域一个域地看，
 * 用户记忆 + Wiki 已接入，CodeGraph/项目未来接入即新增 tab）→ 选中域的工具开关列表
 * （拨杆 + 语义徽标，说明收 tooltip）。
 * 管理语义：总开关关闭 = /mcp 整体 503；工具停用 = 对 AI 隐身 + 调用拒。
 */
import { useEffect, useMemo, useState } from 'react'
import { Check, Copy } from 'lucide-react'
import { api, type McpInfo, type McpToolInfo } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Card, ErrorBox, PageHeader, Spinner, Tabs } from '@/components/ui-bits'
import { cn } from '@/lib/utils'

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

/** 域 key → 中文标签（与侧栏资产域命名同源）。 */
const DOMAIN_LABELS: Record<string, string> = {
  memory: '用户记忆',
  wiki: 'Wiki',
  codegraph: '代码图谱',
  project: '项目',
}

export default function Mcp() {
  const [info, setInfo] = useState<McpInfo | null>(null)
  const [err, setErr] = useState('')
  const [domain, setDomain] = useState<string>('')
  const [busy, setBusy] = useState(false)

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

          {/* 域 Tabs + 选中域的工具开关列表 */}
          <Card className="overflow-hidden">
            {domains.length > 0 && (
              <div className="border-b border-border px-3 py-2">
                <Tabs
                  items={domains.map(([d, tools]) => ({
                    value: d,
                    label: DOMAIN_LABELS[d] ?? d,
                    count: tools.length,
                  }))}
                  value={currentDomain}
                  onChange={setDomain}
                />
              </div>
            )}
            <div className="flex items-center justify-between px-3 py-2">
              <h3 className="text-sm font-semibold">{DOMAIN_LABELS[currentDomain] ?? currentDomain}域工具</h3>
              <p className="font-mono text-xs text-muted-foreground" aria-live="polite">
                启用 {domainTools.length - domainDisabled}/{domainTools.length}
                {domainDisabled > 0 && ` · 停用 ${domainDisabled}`}
              </p>
            </div>
            <ul role="list">
              {domainTools.map((t) => {
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
              停用 = 对 AI 隐身（tools/list 不出现）且调用被拒；悬浮工具名看完整说明。总开关关闭时整个 /mcp 拒绝连接。
            </p>
          </Card>
        </>
      ) : null}
    </div>
  )
}

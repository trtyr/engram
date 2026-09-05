/**
 * MCP 管理页：管理 Engram 的 MCP 服务面。
 * 三件事：服务总开关（关闭 = /mcp 整体 503）、工具粒度开关（停用 = 对 AI 隐身 + 调用拒绝）、
 * 连接配置一键复制（密钥在「设置 → API 密钥」签发，这里只选不建）。
 * 未来 Wiki / CodeGraph 等域接入 MCP 时，工具管理卡按域分组扩展。
 */
import { useEffect, useMemo, useState } from 'react'
import { Check, Copy } from 'lucide-react'
import { api, type ApiKey, type McpInfo } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Card, ErrorBox, PageHeader, Spinner } from '@/components/ui-bits'
import { selectCls } from '@/lib/ui'

/** 可复制的连接配置：每种客户端一段文案（<KEY> 占位符由用户粘贴 key 明文）。 */
function useConnectionConfigs(url: string | null, hasKey: boolean) {
  return useMemo(() => {
    if (!url || !hasKey) return []
    return [
      {
        id: 'claude-code',
        label: 'Claude Code',
        hint: '终端运行（claude mcp add）',
        text: `claude mcp add --transport http engram ${url} --header "Authorization: Bearer <KEY>"`,
      },
      {
        id: 'cursor',
        label: 'Cursor',
        hint: '全局 / 项目 mcp.json',
        text: JSON.stringify(
          { mcpServers: { engram: { url, headers: { Authorization: 'Bearer <KEY>' } } } },
          null,
          2,
        ),
      },
      {
        id: 'claude-desktop',
        label: 'Claude Desktop',
        hint: 'claude_desktop_config.json',
        text: JSON.stringify(
          {
            mcpServers: {
              engram: { type: 'http', url, headers: { Authorization: 'Bearer <KEY>' } },
            },
          },
          null,
          2,
        ),
      },
    ]
  }, [url, hasKey])
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

export default function Mcp() {
  const [info, setInfo] = useState<McpInfo | null>(null)
  const [err, setErr] = useState('')
  const [keys, setKeys] = useState<ApiKey[] | null>(null)
  const [selectedKeyId, setSelectedKeyId] = useState<string>('')
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

  /** 更新配置（开关粒度由调用方给全量或增量字段），成功后以响应刷新本地状态。 */
  const putConfig = async (body: { enabled?: boolean; disabled_tools?: string[] }) => {
    setBusy(true)
    try {
      const next = await api.put<McpInfo>('/settings/mcp', body)
      setInfo(next)
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
  const configs = useConnectionConfigs(mcpUrl, !!selectedKey)

  return (
    <div className="space-y-6">
      <PageHeader
        title="MCP"
        desc="管理 Engram 的 MCP 服务——控制 AI 客户端能接入什么、能调用哪些工具"
      />
      {err && <ErrorBox msg={err} />}

      {info === null && !err ? (
        <Spinner />
      ) : info ? (
        <>
          {/* 服务总开关 */}
          <Card className={info.enabled ? 'p-4' : 'border-destructive/40 p-4'}>
            <div className="flex flex-wrap items-center gap-x-6 gap-y-2">
              <div className="min-w-0">
                <p className="text-sm font-semibold">MCP 服务{info.enabled ? '运行中' : '已关闭'}</p>
                <p className="mt-0.5 text-xs text-muted-foreground">
                  关闭后 /mcp 整体拒绝（503）——已签发的 key 也无法连接
                </p>
              </div>
              <div className="ml-auto flex items-center gap-2">
                <Button
                  size="sm"
                  variant={info.enabled ? 'destructive' : 'outline'}
                  disabled={busy}
                  onClick={() => putConfig({ enabled: !info.enabled })}
                >
                  {info.enabled ? '关闭服务' : '开启服务'}
                </Button>
              </div>
            </div>
            <div className="mt-3 flex flex-wrap items-center gap-x-6 gap-y-2 border-t border-border pt-3">
              <div className="min-w-0">
                <p className="text-xs text-muted-foreground">端点</p>
                <code className="mt-0.5 block truncate font-mono text-sm">{mcpUrl}</code>
              </div>
              <div>
                <p className="text-xs text-muted-foreground">协议版本</p>
                <p className="mt-0.5 font-mono text-sm">{info.protocol_version}</p>
              </div>
              <div>
                <p className="text-xs text-muted-foreground">服务端</p>
                <p className="mt-0.5 font-mono text-sm">
                  {info.server_name} v{info.server_version}
                </p>
              </div>
              <div className="ml-auto">
                <CopyBtn text={mcpUrl ?? ''} label="复制端点" />
              </div>
            </div>
          </Card>

          {/* 工具粒度开关 */}
          <Card className="overflow-hidden">
            <div className="border-b border-border px-4 py-3">
              <h3 className="text-sm font-semibold">工具管理（{info.tools.length}）</h3>
              <p className="mt-0.5 text-xs text-muted-foreground">
                停用的工具对 AI 隐身（tools/list 不出现）且调用被拒——语义等同下线，不影响配置
              </p>
            </div>
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead className="border-b border-border text-left">
                  <tr>
                    <th className="px-3 py-2 text-xs font-medium text-muted-foreground">工具</th>
                    <th className="px-3 py-2 text-xs font-medium text-muted-foreground">说明</th>
                    <th className="w-24 px-3 py-2 text-xs font-medium text-muted-foreground">语义</th>
                    <th className="w-24 px-3 py-2 text-xs font-medium text-muted-foreground">状态</th>
                    <th className="w-20 px-3 py-2" />
                  </tr>
                </thead>
                <tbody>
                  {info.tools.map((t) => {
                    const disabled = info.disabled_tools.includes(t.name)
                    return (
                      <tr key={t.name} className="border-b border-border/60 transition-colors last:border-b-0 hover:bg-muted/40">
                        <td className="px-3 py-2 font-mono text-xs align-top">{t.name}</td>
                        <td className="px-3 py-2 text-xs leading-5 text-muted-foreground align-top">
                          {t.description.split('\n').filter(Boolean).slice(0, 2).join(' ')}
                        </td>
                        <td className="px-3 py-2 text-xs text-muted-foreground align-top">
                          {t.destructive ? '破坏性' : t.read_only ? '只读' : '写入'}
                        </td>
                        <td className="px-3 py-2 align-top">
                          <span className={disabled ? 'text-xs text-muted-foreground' : 'text-xs text-success'}>
                            {disabled ? '停用' : '启用'}
                          </span>
                        </td>
                        <td className="px-3 py-2 text-right align-top">
                          <Button
                            size="sm"
                            variant={disabled ? 'outline' : 'ghost'}
                            disabled={busy}
                            onClick={() =>
                              putConfig({
                                disabled_tools: disabled
                                  ? info.disabled_tools.filter((d) => d !== t.name)
                                  : [...info.disabled_tools, t.name],
                              })
                            }
                          >
                            {disabled ? '启用' : '停用'}
                          </Button>
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            </div>
          </Card>

          {/* 连接配置（密钥在设置页签发，这里只选） */}
          <Card className="p-4">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <h3 className="text-sm font-semibold">连接配置</h3>
              <p className="text-xs text-muted-foreground">
                密钥在「设置 → API 密钥」签发（勾 memory scope），配置中的 &lt;KEY&gt; 粘贴时换成 key 明文
              </p>
            </div>
            {mcpKeys.length === 0 ? (
              <p className="mt-2 text-xs text-muted-foreground">
                还没有 memory scope 的 key——去「设置 → API 密钥」签发一把（勾选 memory scope），再回这里复制配置。
              </p>
            ) : (
              <>
                <div className="mt-2 flex flex-wrap items-center gap-2">
                  <label htmlFor="mcp-key-select" className="text-sm font-medium">
                    使用密钥
                  </label>
                  <select
                    id="mcp-key-select"
                    className={`${selectCls} w-56`}
                    value={selectedKey?.id ?? ''}
                    onChange={(e) => setSelectedKeyId(e.target.value)}
                  >
                    {mcpKeys.map((k) => (
                      <option key={k.id} value={k.id}>
                        {k.name}（{k.key_prefix}… {k.scopes.join('/')}）
                      </option>
                    ))}
                  </select>
                </div>
                <div className="mt-3 grid gap-3 md:grid-cols-3">
                  {configs.map((c) => (
                    <div key={c.id} className="rounded-md border border-border p-3">
                      <div className="flex items-center justify-between gap-2">
                        <div>
                          <p className="text-sm font-medium">{c.label}</p>
                          <p className="text-xs text-muted-foreground">{c.hint}</p>
                        </div>
                        <CopyBtn text={c.text} label="复制" />
                      </div>
                      <pre className="mt-2 overflow-x-auto rounded bg-muted/50 p-2 font-mono text-[11px] leading-4 whitespace-pre-wrap break-all">
                        {c.text}
                      </pre>
                    </div>
                  ))}
                </div>
              </>
            )}
          </Card>
        </>
      ) : null}
    </div>
  )
}

/**
 * MCP 管理页：把 Engram 接入 AI 客户端（Claude Code / Cursor / Claude Desktop 等）。
 * 三件事：端点与协议信息（与后端 MCP 层同源）、连接配置一键复制、MCP 密钥签发（scope 选择）。
 * 未来 Wiki / CodeGraph 等域接入 MCP 时，本页扩展为按域分区的统一 MCP 管理界面。
 */
import { useEffect, useMemo, useState } from 'react'
import { Check, Copy } from 'lucide-react'
import { api, type ApiKey, type McpInfo } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Card, Empty, ErrorBox, PageHeader, Spinner } from '@/components/ui-bits'
import { fmtTime, inputCls, selectCls, tableCls } from '@/lib/ui'

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
  const [name, setName] = useState('')
  const [withErase, setWithErase] = useState(false)
  const [newKey, setNewKey] = useState('')

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

  const mcpKeys = keys?.filter((k) => k.scopes.includes('memory')) ?? []
  const selectedKey = mcpKeys.find((k) => k.id === selectedKeyId) ?? mcpKeys[0] ?? null
  const mcpUrl = info ? `${window.location.origin}${info.endpoint}` : null
  const configs = useConnectionConfigs(mcpUrl, !!selectedKey)

  return (
    <div className="space-y-6">
      <PageHeader
        title="MCP"
        desc="Model Context Protocol——把 Engram 的用户记忆接入 AI 客户端，让 AI 自己来读写你的记忆"
      />
      {err && <ErrorBox msg={err} />}

      {info === null && !err ? (
        <Spinner />
      ) : info ? (
        <>
          {/* 端点信息 */}
          <Card className="p-4">
            <div className="flex flex-wrap items-center gap-x-6 gap-y-2">
              <div className="min-w-0">
                <p className="text-xs text-muted-foreground">MCP 端点</p>
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
            <p className="mt-3 border-t border-border pt-3 text-xs leading-5 text-muted-foreground">
              鉴权用 <code className="font-mono">amk_</code> API key（memory scope），每个请求独立认证——
              在下方签发专用 key，吊销即刻失权。
            </p>
          </Card>

          {/* 连接配置 */}
          <Card className="p-4">
            <h3 className="text-sm font-semibold">连接配置</h3>
            {mcpKeys.length === 0 ? (
              <p className="mt-2 text-xs text-muted-foreground">
                还没有 memory scope 的 key——先在下方「MCP 密钥」签发一把，再回这里复制配置。
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
                  <span className="text-xs text-muted-foreground">配置中的 &lt;KEY&gt; 粘贴时换成 key 明文</span>
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

          {/* 工具清单（与 MCP 层同源：GET /settings/mcp） */}
          <Card className="overflow-hidden">
            <div className="border-b border-border px-4 py-3">
              <h3 className="text-sm font-semibold">工具清单（{info.tools.length}）</h3>
              <p className="mt-0.5 text-xs text-muted-foreground">
                AI 客户端在 initialize 时收到的 instructions：摘要见下方引用
              </p>
            </div>
            <div className="border-b border-border bg-muted/30 px-4 py-3">
              <p className="text-xs leading-5 text-muted-foreground">
                {info.instructions.split('\n').slice(0, 3).join(' ')}
              </p>
            </div>
            <div className="overflow-x-auto">
              <table className={tableCls.root}>
                <thead className={tableCls.thead}>
                  <tr>
                    <th className={tableCls.th}>工具</th>
                    <th className={tableCls.th}>说明</th>
                    <th className={`${tableCls.th} w-24`}>语义</th>
                  </tr>
                </thead>
                <tbody>
                  {info.tools.map((t) => (
                    <tr key={t.name} className={tableCls.row}>
                      <td className={`${tableCls.td} font-mono text-xs`}>{t.name}</td>
                      <td className={`${tableCls.td} text-xs leading-5 text-muted-foreground`}>
                        {t.description.split('\n').filter(Boolean).slice(0, 2).join(' ')}
                      </td>
                      <td className={tableCls.td}>
                        <span className="text-xs text-muted-foreground">
                          {t.destructive ? '破坏性' : t.read_only ? '只读' : '写入'}
                        </span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </Card>
        </>
      ) : null}

      {/* MCP 密钥 */}
      <Card className="p-4">
        <h3 className="text-sm font-semibold">签发 MCP 密钥</h3>
        <p className="mt-0.5 text-xs text-muted-foreground">
          memory scope 让 AI 读写记忆；追加 erase 才能物理删除（不可逆，默认不勾）
        </p>
        <form
          className="mt-3 flex flex-wrap items-center gap-3"
          onSubmit={async (e) => {
            e.preventDefault()
            const scopes = withErase ? ['memory', 'erase'] : ['memory']
            const r = await api.post<{ key: string }>('/settings/api-keys', { name, scopes })
            setNewKey(r.key)
            setName('')
            setWithErase(false)
            load()
          }}
        >
          <label htmlFor="mcp-key-name" className="text-sm font-medium">
            名称
          </label>
          <input
            id="mcp-key-name"
            className={`${inputCls} w-48`}
            placeholder="claude-code"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
          <label className="flex items-center gap-1.5 text-sm">
            <input
              type="checkbox"
              className="accent-current"
              checked={withErase}
              onChange={(e) => setWithErase(e.target.checked)}
            />
            含 erase scope（可物理删除会话）
          </label>
          <Button size="sm" type="submit">
            签发
          </Button>
        </form>
      </Card>
      {newKey && (
        <Card className="border-success/30 bg-success/10 p-4">
          <p className="text-xs text-muted-foreground">
            新 key（只显示这一次，粘贴到上方连接配置的 &lt;KEY&gt; 处）：
          </p>
          <div className="mt-1.5 flex flex-wrap items-center gap-2">
            <code className="block break-all font-mono text-sm">{newKey}</code>
            <CopyBtn text={newKey} label="复制" />
          </div>
        </Card>
      )}
      {keys === null ? (
        <Spinner />
      ) : mcpKeys.length === 0 ? (
        <Empty text="无 MCP 密钥（memory scope）" />
      ) : (
        <Card className="overflow-hidden">
          <div className="overflow-x-auto">
            <table className={tableCls.root}>
              <thead className={tableCls.thead}>
                <tr>
                  <th className={tableCls.th}>名称</th>
                  <th className={tableCls.th}>前缀</th>
                  <th className={tableCls.th}>scopes</th>
                  <th className={tableCls.th}>创建</th>
                  <th className={tableCls.th}>最近使用</th>
                  <th className={tableCls.th} />
                </tr>
              </thead>
              <tbody>
                {mcpKeys.map((k) => (
                  <tr key={k.id} className={tableCls.row}>
                    <td className={`${tableCls.td} font-medium`}>{k.name}</td>
                    <td className={`${tableCls.td} font-mono`}>{k.key_prefix}…</td>
                    <td className={`${tableCls.td} font-mono text-xs text-muted-foreground`}>
                      {k.scopes.join(', ')}
                    </td>
                    <td className={`${tableCls.td} text-muted-foreground`}>{fmtTime(k.created_at)}</td>
                    <td className={`${tableCls.td} text-muted-foreground`}>
                      {k.last_used_at ? fmtTime(k.last_used_at) : '—'}
                    </td>
                    <td className={`${tableCls.td} text-right`}>
                      <Button
                        variant="destructive"
                        size="sm"
                        onClick={async () => {
                          if (!confirm(`删除 key「${k.name}」？使用它的 AI 将立即失权。`)) return
                          await api.post(`/settings/api-keys/${k.id}/revoke`)
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
          </div>
        </Card>
      )}
    </div>
  )
}

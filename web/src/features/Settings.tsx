/** Settings 域：LLM providers / 路由 / API keys。 */
import { useEffect, useState } from 'react'
import { api, type ApiKey, type Provider } from '@/lib/api'
import { Empty, ErrorBox, Spinner, fmtTime } from '@/components/ui-bits'
import { Button } from '@/components/ui/button'

type Tab = 'providers' | 'routing' | 'keys'

export default function Settings() {
  const [tab, setTab] = useState<Tab>('providers')
  return (
    <div className="space-y-6">
      <h1 className="text-xl font-semibold">Settings</h1>
      <div className="flex gap-2">
        {(['providers', 'routing', 'keys'] as Tab[]).map((t) => (
          <Button key={t} variant={tab === t ? 'default' : 'outline'} size="sm" onClick={() => setTab(t)}>
            {t}
          </Button>
        ))}
      </div>
      {tab === 'providers' && <Providers />}
      {tab === 'routing' && <Routing />}
      {tab === 'keys' && <Keys />}
    </div>
  )
}

function Providers() {
  const [rows, setRows] = useState<Provider[] | null>(null)
  const [err, setErr] = useState('')
  const [form, setForm] = useState({ name: '', base_url: '', api_key: '', chat_model: '', embed_model: '' })
  const [testMsg, setTestMsg] = useState<Record<string, string>>({})
  const load = () => api.get<Provider[]>('/settings/llm/providers').then(setRows).catch((e) => setErr(e.message))
  useEffect(() => {
    load()
  }, [])
  return (
    <div className="space-y-4">
      <form
        className="grid gap-2 rounded-lg border p-4 md:grid-cols-2"
        onSubmit={async (e) => {
          e.preventDefault()
          const models = []
          if (form.chat_model) models.push({ id: form.chat_model, capabilities: ['chat'] })
          if (form.embed_model) models.push({ id: form.embed_model, capabilities: ['embedding'] })
          try {
            await api.post('/settings/llm/providers', { ...form, models, is_default: rows?.length === 0 })
            setForm({ name: '', base_url: '', api_key: '', chat_model: '', embed_model: '' })
            load()
          } catch (ex) {
            setErr(ex instanceof Error ? ex.message : '注册失败')
          }
        }}
      >
        <input className="rounded-md border bg-transparent px-3 py-1.5 text-sm" placeholder="名称" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} />
        <input className="rounded-md border bg-transparent px-3 py-1.5 text-sm" placeholder="Base URL（OpenAI 兼容）" value={form.base_url} onChange={(e) => setForm({ ...form, base_url: e.target.value })} />
        <input className="rounded-md border bg-transparent px-3 py-1.5 text-sm" type="password" placeholder="API Key（加密存储）" value={form.api_key} onChange={(e) => setForm({ ...form, api_key: e.target.value })} />
        <input className="rounded-md border bg-transparent px-3 py-1.5 text-sm" placeholder="chat 模型 ID" value={form.chat_model} onChange={(e) => setForm({ ...form, chat_model: e.target.value })} />
        <input className="rounded-md border bg-transparent px-3 py-1.5 text-sm" placeholder="embedding 模型 ID" value={form.embed_model} onChange={(e) => setForm({ ...form, embed_model: e.target.value })} />
        <Button size="sm" type="submit">注册 Provider</Button>
      </form>
      {err && <ErrorBox msg={err} />}
      {rows === null ? (
        <Spinner />
      ) : rows.length === 0 ? (
        <Empty text="未配置 provider" />
      ) : (
        rows.map((p) => (
          <div key={p.id} className="flex items-center justify-between rounded-lg border p-4">
            <div>
              <p className="font-medium">
                {p.name} {p.is_default && <span className="ml-1 text-xs text-green-400">默认</span>}
              </p>
              <p className="text-xs text-muted-foreground">
                {p.base_url} · {p.models.map((m) => m.id).join(', ')}
              </p>
              {testMsg[p.id] && <p className="text-xs text-blue-400">{testMsg[p.id]}</p>}
            </div>
            <Button
              size="sm"
              variant="outline"
              onClick={async () => {
                try {
                  const r = await api.post<{ ok: boolean; message: string }>(`/settings/llm/providers/${p.id}/test`)
                  setTestMsg({ ...testMsg, [p.id]: r.message })
                } catch (ex) {
                  setTestMsg({ ...testMsg, [p.id]: ex instanceof Error ? ex.message : '测试失败' })
                }
              }}
            >
              测试连通
            </Button>
          </div>
        ))
      )}
    </div>
  )
}

function Routing() {
  const [text, setText] = useState('')
  const [msg, setMsg] = useState('')
  useEffect(() => {
    api.get<Record<string, unknown>>('/settings/llm/routing').then((r) => setText(JSON.stringify(r, null, 2))).catch(() => {})
  }, [])
  return (
    <div className="space-y-2">
      <p className="text-sm text-muted-foreground">
        purpose → provider/model 回退链（extract/arbitrate/organize/consolidate/persona/wiki_analysis/wiki_generation）
      </p>
      <textarea className="h-72 w-full rounded-md border bg-transparent px-3 py-2 font-mono text-sm" value={text} onChange={(e) => setText(e.target.value)} />
      <div className="flex items-center gap-2">
        <Button
          size="sm"
          onClick={async () => {
            try {
              await api.put('/settings/llm/routing', JSON.parse(text))
              setMsg('已保存')
            } catch (ex) {
              setMsg(ex instanceof Error ? ex.message : '保存失败（JSON 非法?）')
            }
          }}
        >
          保存
        </Button>
        {msg && <p className="text-xs text-muted-foreground">{msg}</p>}
      </div>
    </div>
  )
}

function Keys() {
  const [rows, setRows] = useState<ApiKey[] | null>(null)
  const [newKey, setNewKey] = useState('')
  const [name, setName] = useState('')
  const load = () => api.get<ApiKey[]>('/settings/api-keys').then(setRows).catch(() => {})
  useEffect(() => {
    load()
  }, [])
  return (
    <div className="space-y-4">
      <form
        className="flex gap-2"
        onSubmit={async (e) => {
          e.preventDefault()
          const r = await api.post<{ key: string }>('/settings/api-keys', { name })
          setNewKey(r.key)
          setName('')
          load()
        }}
      >
        <input className="w-48 rounded-md border bg-transparent px-3 py-1.5 text-sm" placeholder="名称" value={name} onChange={(e) => setName(e.target.value)} />
        <Button size="sm" variant="outline" type="submit">签发</Button>
      </form>
      {newKey && (
        <div className="rounded-lg border border-green-500/30 bg-green-500/10 p-4 text-sm">
          <p className="text-xs text-muted-foreground">新 key（只显示这一次，给 AI 客户端用）：</p>
          <code className="mt-1 block break-all font-mono">{newKey}</code>
        </div>
      )}
      {rows === null ? (
        <Spinner />
      ) : rows.length === 0 ? (
        <Empty text="无 API key" />
      ) : (
        <table className="w-full text-sm">
          <thead className="text-left text-muted-foreground">
            <tr className="border-b">
              <th className="py-1.5 pr-4">名称</th>
              <th className="pr-4">前缀</th>
              <th className="pr-4">创建</th>
              <th className="pr-4">最近使用</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rows.map((k) => (
              <tr key={k.id} className="border-b">
                <td className="py-1.5 pr-4">{k.name}</td>
                <td className="pr-4 font-mono">{k.key_prefix}…</td>
                <td className="pr-4">{fmtTime(k.created_at)}</td>
                <td className="pr-4">{k.last_used_at ? fmtTime(k.last_used_at) : '—'}</td>
                <td className="text-right">
                  {k.revoked_at ? (
                    <span className="text-xs text-red-400">已吊销</span>
                  ) : (
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={async () => {
                        await api.post(`/settings/api-keys/${k.id}/revoke`)
                        load()
                      }}
                    >
                      吊销
                    </Button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  )
}

/** Settings 域：LLM providers / 路由 / API keys。 */
import { useEffect, useState } from 'react'
import { api, type ApiKey, type Provider } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner, Tabs } from '@/components/ui-bits'
import { fmtTime, inputCls, tableCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'

type Tab = 'providers' | 'routing' | 'keys'

const TABS: { value: Tab; label: string }[] = [
  { value: 'providers', label: 'Providers' },
  { value: 'routing', label: '路由' },
  { value: 'keys', label: 'API Keys' },
]

export default function Settings() {
  const [tab, setTab] = useState<Tab>('providers')
  return (
    <div className="space-y-6">
      <PageHeader title="Settings" desc="LLM 供应商、模型路由与 API 密钥" />
      <Tabs items={TABS} value={tab} onChange={setTab} />
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
  const [editingId, setEditingId] = useState<string | null>(null)
  const load = () => api.get<Provider[]>('/settings/llm/providers').then(setRows).catch((e) => setErr(e.message))
  useEffect(() => {
    load()
  }, [])
  return (
    <div className="space-y-4">
      <Card className="p-4">
        <form
          className="grid gap-3 md:grid-cols-2"
          onSubmit={async (e) => {
            e.preventDefault()
            const models = []
            if (form.chat_model) models.push({ id: form.chat_model, capabilities: ['chat'] })
            if (form.embed_model) models.push({ id: form.embed_model, capabilities: ['embedding'] })
            try {
              if (editingId) {
                // name 不可改；key 留空表示不更新
                const body: Record<string, unknown> = { base_url: form.base_url, models }
                if (form.api_key) body.api_key = form.api_key
                await api.put(`/settings/llm/providers/${editingId}`, body)
              } else {
                await api.post('/settings/llm/providers', { ...form, models, is_default: rows?.length === 0 })
              }
              setForm({ name: '', base_url: '', api_key: '', chat_model: '', embed_model: '' })
              setEditingId(null)
              load()
            } catch (ex) {
              setErr(ex instanceof Error ? ex.message : '保存失败')
            }
          }}
        >
          <input
            className={inputCls}
            placeholder="名称"
            value={form.name}
            disabled={!!editingId}
            onChange={(e) => setForm({ ...form, name: e.target.value })}
          />
          <input
            className={inputCls}
            placeholder="Base URL（OpenAI 兼容）"
            value={form.base_url}
            onChange={(e) => setForm({ ...form, base_url: e.target.value })}
          />
          <input
            className={inputCls}
            type="password"
            placeholder="API Key（加密存储）"
            value={form.api_key}
            onChange={(e) => setForm({ ...form, api_key: e.target.value })}
          />
          <input
            className={inputCls}
            placeholder="chat 模型 ID"
            value={form.chat_model}
            onChange={(e) => setForm({ ...form, chat_model: e.target.value })}
          />
          <input
            className={inputCls}
            placeholder="embedding 模型 ID"
            value={form.embed_model}
            onChange={(e) => setForm({ ...form, embed_model: e.target.value })}
          />
          <div className="flex items-center gap-2 md:col-span-2">
            <Button size="sm" type="submit">
              {editingId ? '保存修改' : '注册 Provider'}
            </Button>
            {editingId && (
              <Button
                size="sm"
                variant="outline"
                type="button"
                onClick={() => {
                  setEditingId(null)
                  setForm({ name: '', base_url: '', api_key: '', chat_model: '', embed_model: '' })
                }}
              >
                取消
              </Button>
            )}
          </div>
        </form>
      </Card>
      {err && <ErrorBox msg={err} />}
      {rows === null ? (
        <Spinner />
      ) : rows.length === 0 ? (
        <Empty text="未配置 provider" />
      ) : (
        rows.map((p) => (
          <Card key={p.id} className="flex items-center justify-between p-4">
            <div>
              <p className="font-medium">
                {p.name}{' '}
                {p.is_default && <span className="ml-1 text-xs text-green-400">默认</span>}
              </p>
              <p className="mt-0.5 text-xs text-muted-foreground">
                {p.base_url} · {p.models.map((m) => m.id).join(', ')}
              </p>
              {testMsg[p.id] && <p className="mt-1 text-xs text-brand-strong">{testMsg[p.id]}</p>}
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <Button
                size="sm"
                variant="outline"
                onClick={async () => {
                  try {
                    const r = await api.post<{ ok: boolean; message: string }>(
                      `/settings/llm/providers/${p.id}/test`,
                    )
                    setTestMsg({ ...testMsg, [p.id]: r.message })
                  } catch (ex) {
                    setTestMsg({ ...testMsg, [p.id]: ex instanceof Error ? ex.message : '测试失败' })
                  }
                }}
              >
                测试连通
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() => {
                  const chat = p.models.find((m) => m.capabilities.includes('chat'))?.id ?? ''
                  const embed = p.models.find((m) => m.capabilities.includes('embedding'))?.id ?? ''
                  setEditingId(p.id)
                  setForm({ name: p.name, base_url: p.base_url, api_key: '', chat_model: chat, embed_model: embed })
                  setErr('')
                }}
              >
                编辑
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={async () => {
                  if (!confirm(`删除 provider「${p.name}」？`)) return
                  try {
                    await api.del(`/settings/llm/providers/${p.id}`)
                    load()
                  } catch (ex) {
                    setErr(ex instanceof Error ? ex.message : '删除失败')
                  }
                }}
              >
                删除
              </Button>
            </div>
          </Card>
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
    <div className="space-y-3">
      <p className="text-sm text-muted-foreground">
        purpose → provider/model 回退链（extract/arbitrate/organize/consolidate/persona/wiki_analysis/wiki_generation）
      </p>
      <textarea
        className={`${inputCls} h-72 w-full font-mono`}
        value={text}
        onChange={(e) => setText(e.target.value)}
      />
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
        <input
          className={`${inputCls} w-48`}
          placeholder="名称"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <Button size="sm" variant="outline" type="submit">
          签发
        </Button>
      </form>
      {newKey && (
        <Card className="border-green-500/30 bg-green-500/10 p-4">
          <p className="text-xs text-muted-foreground">新 key（只显示这一次，给 AI 客户端用）：</p>
          <code className="mt-1.5 block break-all font-mono text-sm">{newKey}</code>
        </Card>
      )}
      {rows === null ? (
        <Spinner />
      ) : rows.length === 0 ? (
        <Empty text="无 API key" />
      ) : (
        <Card className="overflow-hidden">
          <table className={tableCls.root}>
            <thead className={tableCls.thead}>
              <tr>
                <th className={tableCls.th}>名称</th>
                <th className={tableCls.th}>前缀</th>
                <th className={tableCls.th}>创建</th>
                <th className={tableCls.th}>最近使用</th>
                <th className={tableCls.th} />
              </tr>
            </thead>
            <tbody>
              {rows.map((k) => (
                <tr key={k.id} className={tableCls.row}>
                  <td className={`${tableCls.td} font-medium`}>{k.name}</td>
                  <td className={`${tableCls.td} font-mono`}>{k.key_prefix}…</td>
                  <td className={`${tableCls.td} text-muted-foreground`}>{fmtTime(k.created_at)}</td>
                  <td className={`${tableCls.td} text-muted-foreground`}>
                    {k.last_used_at ? fmtTime(k.last_used_at) : '—'}
                  </td>
                  <td className={`${tableCls.td} text-right`}>
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
        </Card>
      )}
    </div>
  )
}

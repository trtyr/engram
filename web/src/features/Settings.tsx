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

      <div className="mt-10 space-y-3">
        <h2 className="text-sm font-semibold text-destructive">危险区域</h2>
        <ReencryptPane />
      </div>
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
          <div className="space-y-1.5">
            <label htmlFor="prov-name" className="block text-xs font-medium">名称</label>
            <input
              id="prov-name"
              className={`${inputCls} w-full`}
              placeholder="SiliconFlow"
              value={form.name}
              disabled={!!editingId}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
            />
          </div>
          <div className="space-y-1.5">
            <label htmlFor="prov-url" className="block text-xs font-medium">Base URL（OpenAI 兼容）</label>
            <input
              id="prov-url"
              className={`${inputCls} w-full`}
              placeholder="https://api.example.com/v1"
              value={form.base_url}
              onChange={(e) => setForm({ ...form, base_url: e.target.value })}
            />
          </div>
          <div className="space-y-1.5">
            <label htmlFor="prov-key" className="block text-xs font-medium">
              API Key<span className="ml-1 font-normal text-muted-foreground">（AES-GCM 加密落库{editingId ? '；留空不更新' : ''}）</span>
            </label>
            <input
              id="prov-key"
              className={`${inputCls} w-full`}
              type="password"
              placeholder="sk-…"
              value={form.api_key}
              onChange={(e) => setForm({ ...form, api_key: e.target.value })}
            />
          </div>
          <div className="space-y-1.5">
            <label htmlFor="prov-chat" className="block text-xs font-medium">chat 模型 ID</label>
            <input
              id="prov-chat"
              className={`${inputCls} w-full font-mono`}
              placeholder="deepseek-ai/DeepSeek-V4"
              value={form.chat_model}
              onChange={(e) => setForm({ ...form, chat_model: e.target.value })}
            />
          </div>
          <div className="space-y-1.5">
            <label htmlFor="prov-embed" className="block text-xs font-medium">embedding 模型 ID</label>
            <input
              id="prov-embed"
              className={`${inputCls} w-full font-mono`}
              placeholder="Qwen/Qwen3-Embedding-8B"
              value={form.embed_model}
              onChange={(e) => setForm({ ...form, embed_model: e.target.value })}
            />
          </div>
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
                {p.is_default && <span className="ml-1 text-xs text-success">默认</span>}
              </p>
              <p className="mt-0.5 text-xs text-muted-foreground">
                {p.base_url} · {p.models.map((m) => m.id).join(', ')}
              </p>
              {testMsg[p.id] && <p className="mt-1 text-xs text-foreground">{testMsg[p.id]}</p>}
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
                variant="destructive"
                onClick={async () => {
                  if (!confirm(`删除 provider「${p.name}」？该操作不可撤销。`)) return
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
        <label htmlFor="key-name" className="text-sm font-medium">名称</label>
        <input
          id="key-name"
          className={`${inputCls} w-48`}
          placeholder="pi-agent"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <Button size="sm" variant="outline" type="submit">
          签发
        </Button>
      </form>
      {newKey && (
        <Card className="border-success/30 bg-success/10 p-4">
          <p className="text-xs text-muted-foreground">新 key（只显示这一次，给 AI 客户端用）：</p>
          <code className="mt-1.5 block break-all font-mono text-sm">{newKey}</code>
        </Card>
      )}
      {rows === null ? (
        <Spinner />
      ) : rows.length === 0 ? (
        <Empty text="无 API key" />
      ) : (
        <Card className="overflow-x-auto">
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
                      <span className="text-xs text-destructive">已吊销</span>
                    ) : (
                      <Button
                        variant="destructive"
                        size="sm"
                        onClick={async () => {
                          if (!confirm(`吊销 key「${k.name}」？使用它的 AI 将立即失权。`)) return
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

/** 主密钥重加密：更换 AGENT_MEMORY_MASTER_KEY 后，用旧密钥重加密所有 provider 密钥。 */
function ReencryptPane() {
  const [oldKey, setOldKey] = useState('')
  const [busy, setBusy] = useState(false)
  const [msg, setMsg] = useState('')
  return (
    <Card className="space-y-3 border-destructive/30 p-4">
      <p className="text-sm text-muted-foreground">
        更换主密钥（AGENT_MEMORY_MASTER_KEY）后，用旧主密钥重加密所有 provider 密钥。填错旧密钥会使全部 provider 不可用。
      </p>
      <div className="space-y-1.5">
        <label htmlFor="reencrypt-old" className="block text-xs font-medium">旧主密钥（64 hex）</label>
        <input
          id="reencrypt-old"
          className={`${inputCls} w-full font-mono`}
          type="password"
          placeholder="openssl rand -hex 32 的旧值"
          value={oldKey}
          onChange={(e) => setOldKey(e.target.value)}
        />
      </div>
      <div className="flex items-center gap-2">
        <Button
          size="sm"
          variant="destructive"
          disabled={busy || !oldKey.trim()}
          onClick={async () => {
            setBusy(true)
            setMsg('')
            try {
              const r = await api.post<{ re_encrypted: number }>('/settings/llm/providers/re-encrypt', {
                old_master_key: oldKey.trim(),
              })
              setMsg(`已重加密 ${r.re_encrypted} 个 provider`)
            } catch (ex) {
              setMsg(ex instanceof Error ? ex.message : '重加密失败')
            } finally {
              setBusy(false)
            }
          }}
        >
          {busy ? '重加密中…' : '执行重加密'}
        </Button>
        {msg && <p className="text-xs text-muted-foreground">{msg}</p>}
      </div>
    </Card>
  )
}

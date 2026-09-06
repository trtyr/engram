/** Settings 域：LLM providers / 路由 / API keys / 节律。 */
import { useEffect, useState } from 'react'
import { api, type AdminSessionDto, type ApiKey, type Job, type Provider } from '@/lib/api'
import { Card, Checkbox, Empty, ErrorBox, PageHeader, Spinner, StatusBadge, Tabs } from '@/components/ui-bits'
import { fmtTime, inputCls, selectCls, relTime, tableCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'

type Tab = 'providers' | 'routing' | 'keys' | 'rhythm' | 'danger' | 'migrate' | 'account'

const TABS: { value: Tab; label: string }[] = [
  { value: 'routing', label: 'AI 功能' },
  { value: 'providers', label: '供应商' },
  { value: 'keys', label: 'API 密钥' },
  { value: 'rhythm', label: '节律' },
  { value: 'danger', label: '危险操作' },
  { value: 'migrate', label: '数据迁移' },
  { value: 'account', label: '账号与会话' },
]

const PURPOSES: { key: string; label: string; desc: string }[] = [
  { key: 'extract', label: '抽取', desc: '把对话提炼成一条条记忆（最频繁，用便宜的模型）' },
  { key: 'arbitrate', label: '仲裁', desc: '判断两条记忆是否重复或矛盾（用便宜的模型）' },
  { key: 'embed', label: '嵌入', desc: '把文字转成向量，供语义搜索（用 embedding 模型）' },
  { key: 'organize', label: '组织', desc: '把零散记忆聚成一个个话题场景（中等模型）' },
  { key: 'consolidate', label: '整理', desc: '定期更新你的画像和人物档案（中等模型）' },
  { key: 'wiki_analysis', label: 'Wiki 分析', desc: '分析你喂进去的文档，提取结构' },
  { key: 'persona', label: '画像', desc: '沉淀对你的长期了解（用最强的模型）' },
  { key: 'wiki_generation', label: 'Wiki 生成', desc: '把素材写成成篇的 Wiki 页面（用最强的模型）' },
]

export default function Settings() {
  const [tab, setTab] = useState<Tab>('routing')
  return (
    <div className="space-y-6">
      <PageHeader title="设置" desc="LLM 供应商、模型路由、API 密钥与记忆节律" />
      <Tabs items={TABS} value={tab} onChange={setTab} />
      {tab === 'routing' && <Routing />}
      {tab === 'providers' && <Providers />}
      {tab === 'keys' && <Keys />}
      {tab === 'rhythm' && <RhythmPane />}
      {tab === 'danger' && <DangerZone />}
      {tab === 'migrate' && <MigratePane />}
      {tab === 'account' && <AccountPane />}
    </div>
  )
}

/** 账号与会话（单用户管理）：改用户名/密码 + 活跃会话列表（吊销/吊销其他）。 */
function AccountPane() {
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState('')
  const [okMsg, setOkMsg] = useState('')
  const [username, setUsername] = useState<string | null>(null)
  const [sessions, setSessions] = useState<AdminSessionDto[] | null>(null)

  const [curPw, setCurPw] = useState('')
  const [newUsername, setNewUsername] = useState('')
  const [newPw, setNewPw] = useState('')
  const [newPw2, setNewPw2] = useState('')

  const loadSessions = () =>
    api
      .get<AdminSessionDto[]>('/auth/sessions')
      .then(setSessions)
      .catch((e) => setErr(e.message))

  useEffect(() => {
    api
      .get<{ username: string }>('/auth/username')
      .then((r) => setUsername(r.username))
      .catch(() => {})
    loadSessions()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  async function doChange() {
    if (!curPw) {
      setErr('请输入当前密码')
      return
    }
    if (!newUsername.trim() && !newPw) {
      setErr('新用户名与新密码至少填一项')
      return
    }
    if (newPw && newPw.length < 8) {
      setErr('新密码至少 8 位')
      return
    }
    if (newPw && newPw !== newPw2) {
      setErr('两次输入的新密码不一致')
      return
    }
    setBusy(true)
    try {
      const r = await api.put<{ username: string; revoked_sessions: number }>('/auth/account', {
        current_password: curPw,
        new_username: newUsername.trim() || undefined,
        new_password: newPw || undefined,
      })
      setOkMsg(
        `已更新${newUsername.trim() ? `（用户名：${r.username}）` : ''}` +
          (r.revoked_sessions > 0 ? `，吊销其他会话 ${r.revoked_sessions} 个` : ''),
      )
      setUsername(r.username)
      setCurPw('')
      setNewUsername('')
      setNewPw('')
      setNewPw2('')
      setErr('')
      loadSessions()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '修改失败')
    } finally {
      setBusy(false)
    }
  }

  async function doRevoke(id: string) {
    if (!confirm(`吊销会话 ${id}？该设备下次请求需重新登录。`)) return
    setBusy(true)
    try {
      await api.del(`/auth/sessions/${id}`)
      setErr('')
      loadSessions()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '吊销失败')
    } finally {
      setBusy(false)
    }
  }

  async function doRevokeOthers() {
    if (!confirm('吊销除当前设备外的全部会话？')) return
    setBusy(true)
    try {
      const r = await api.post<{ revoked: number }>('/auth/sessions/revoke-others')
      setOkMsg(`已吊销其他会话 ${r.revoked} 个`)
      setErr('')
      loadSessions()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '吊销失败')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="space-y-4">
      {err && <ErrorBox msg={err} />}
      {okMsg && (
        <Card className="border-success/30 bg-success/10 p-3 text-sm text-success">{okMsg}</Card>
      )}

      <Card className="p-4">
        <h3 className="text-sm font-semibold">修改账号</h3>
        <p className="mt-1 text-xs text-muted-foreground">
          当前用户名：{username ?? '…'} · 单用户（唯一管理员）。修改需验证当前密码；
          改密码后其他设备的会话自动吊销。
        </p>
        <div className="mt-3 space-y-3">
          <div>
            <label className="mb-1.5 block text-xs font-medium text-muted-foreground">当前密码</label>
            <input
              className={`${inputCls} w-64`}
              type="password"
              autoComplete="current-password"
              aria-label="当前密码"
              value={curPw}
              onChange={(e) => setCurPw(e.target.value)}
            />
          </div>
          <div className="flex flex-wrap gap-3">
            <div>
              <label className="mb-1.5 block text-xs font-medium text-muted-foreground">新用户名（可选）</label>
              <input
                className={`${inputCls} w-64`}
                aria-label="新用户名"
                placeholder={username ?? ''}
                value={newUsername}
                onChange={(e) => setNewUsername(e.target.value)}
              />
            </div>
            <div>
              <label className="mb-1.5 block text-xs font-medium text-muted-foreground">新密码（可选，≥8 位）</label>
              <input
                className={`${inputCls} w-64`}
                type="password"
                autoComplete="new-password"
                aria-label="新密码"
                value={newPw}
                onChange={(e) => setNewPw(e.target.value)}
              />
            </div>
            <div>
              <label className="mb-1.5 block text-xs font-medium text-muted-foreground">确认新密码</label>
              <input
                className={`${inputCls} w-64`}
                type="password"
                autoComplete="new-password"
                aria-label="确认新密码"
                value={newPw2}
                onChange={(e) => setNewPw2(e.target.value)}
              />
            </div>
          </div>
          <Button size="sm" disabled={busy} onClick={doChange}>
            保存修改
          </Button>
        </div>
      </Card>

      <Card className="overflow-hidden">
        <div className="flex items-center justify-between border-b border-border px-4 py-3">
          <h3 className="text-sm font-semibold">活跃会话</h3>
          <Button size="sm" variant="outline" disabled={busy} onClick={doRevokeOthers}>
            吊销其他设备
          </Button>
        </div>
        {sessions === null ? (
          <Spinner />
        ) : sessions.length === 0 ? (
          <p className="px-4 py-4 text-xs text-muted-foreground">无活跃会话</p>
        ) : (
          <ul role="list" className="divide-y divide-border/60">
            {sessions.map((s) => (
              <li key={s.id} className="flex flex-wrap items-center gap-x-3 gap-y-1 px-4 py-2.5 text-xs">
                <code className="font-mono text-muted-foreground">{s.id}…</code>
                {s.current && (
                  <span className="rounded bg-success/15 px-1.5 py-0.5 text-[10px] text-success">当前设备</span>
                )}
                <span className="text-muted-foreground">
                  创建 {new Date(s.created_at).toLocaleString()}
                </span>
                <span className="text-muted-foreground">
                  最近使用 {s.last_used_at ? new Date(s.last_used_at).toLocaleString() : '—'}
                </span>
                <span className="text-muted-foreground">过期 {new Date(s.expires_at).toLocaleString()}</span>
                {!s.current && (
                  <Button
                    size="sm"
                    variant="ghost"
                    className="ml-auto"
                    disabled={busy}
                    onClick={() => doRevoke(s.id)}
                  >
                    吊销
                  </Button>
                )}
              </li>
            ))}
          </ul>
        )}
        <p className="border-t border-border bg-muted/30 px-4 py-2 text-[11px] text-muted-foreground">
          会话 7 天有效；吊销后该设备下次请求需重新登录。修改密码会自动吊销其他设备。
        </p>
      </Card>
    </div>
  )
}

/** 数据迁移（admin）：全系统导出 / 导入（冲突跳过，分域报告）/ 远程拉取（A→B）。 */
function MigratePane() {
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState('')
  const [report, setReport] = useState<Record<string, unknown> | null>(null)
  const [sourceUrl, setSourceUrl] = useState('')
  const [sourcePassword, setSourcePassword] = useState('')

  const saveBlob = (blob: Blob, filename: string) => {
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = filename
    a.click()
    URL.revokeObjectURL(url)
  }

  async function doExport() {
    setBusy(true)
    try {
      const blob = await api.download('/migrate/export')
      saveBlob(blob, `engram-transfer-${new Date().toISOString().slice(0, 10)}.json`)
      setErr('')
    } catch (e) {
      setErr(e instanceof Error ? e.message : '导出失败')
    } finally {
      setBusy(false)
    }
  }

  async function doImport(file: File) {
    setBusy(true)
    try {
      const text = await file.text()
      const data = JSON.parse(text)
      const r = await api.post<Record<string, unknown>>('/migrate/import', data)
      setReport(r)
      setErr('')
    } catch (e) {
      setErr(e instanceof Error ? e.message : '导入失败（确认是 /migrate/export 的迁移包）')
    } finally {
      setBusy(false)
    }
  }

  async function doPull() {
    if (!sourceUrl.trim() || !sourcePassword) return
    setBusy(true)
    try {
      const r = await api.post<Record<string, unknown>>('/migrate/pull', {
        source_url: sourceUrl.trim(),
        source_admin_password: sourcePassword,
      })
      setReport((r.imported as Record<string, unknown>) ?? r)
      setSourcePassword('')
      setErr('')
    } catch (e) {
      setErr(e instanceof Error ? e.message : '拉取失败')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="space-y-4">
      {err && <ErrorBox msg={err} />}
      {report && (
        <Card className="p-4">
          <h3 className="text-sm font-semibold">导入报告</h3>
          <pre className="mt-2 max-h-72 overflow-auto rounded bg-muted/40 p-3 font-mono text-xs leading-5">
            {JSON.stringify(report, null, 2)}
          </pre>
          <p className="mt-2 text-xs text-muted-foreground">
            已存在的记录按跳过处理（合并语义，可重复执行）；embedding 未迁移——
            用「AI 功能」页重配 embed 后 POST /memory/reembed 补齐。
          </p>
        </Card>
      )}

      <Card className="p-4">
        <h3 className="text-sm font-semibold">① 全系统导出</h3>
        <p className="mt-1 text-xs text-muted-foreground">
          下载迁移包（JSON）：用户记忆五表 + 实体关系 + 技能（含附属文件）+ Wiki 页面 +
          项目记忆。向量不迁移（导入端重建）。
        </p>
        <div className="mt-2">
          <Button size="sm" disabled={busy} onClick={doExport}>
            导出迁移包
          </Button>
        </div>
      </Card>

      <Card className="p-4">
        <h3 className="text-sm font-semibold">② 导入迁移包</h3>
        <p className="mt-1 text-xs text-muted-foreground">
          选择迁移包 JSON：已存在的记录跳过（合并语义，可重复执行）；新记录全量进。
        </p>
        <div className="mt-2">
          <input
            type="file"
            accept=".json,application/json"
            aria-label="迁移包文件"
            className="text-xs"
            disabled={busy}
            onChange={(e) => {
              const f = e.target.files?.[0]
              if (f) doImport(f)
              e.target.value = ''
            }}
          />
        </div>
      </Card>

      <Card className="p-4">
        <h3 className="text-sm font-semibold">③ 远程拉取（A → 本机）</h3>
        <p className="mt-1 text-xs text-muted-foreground">
          填 A 机地址与 A 机管理员密码：本机登录 A 拉取迁移包并落地。
          密码仅本次请求使用，不落库。A 机需为可访问的 engram 实例。
        </p>
        <div className="mt-2 flex flex-wrap gap-2">
          <input
            className={`${inputCls} w-72`}
            placeholder="http://a-host:8080"
            aria-label="源机地址"
            value={sourceUrl}
            onChange={(e) => setSourceUrl(e.target.value)}
          />
          <input
            className={`${inputCls} w-48`}
            type="password"
            placeholder="A 机管理员密码"
            aria-label="源机管理员密码"
            value={sourcePassword}
            onChange={(e) => setSourcePassword(e.target.value)}
          />
          <Button
            size="sm"
            disabled={busy || !sourceUrl.trim() || !sourcePassword}
            onClick={doPull}
          >
            拉取并导入
          </Button>
        </div>
      </Card>
    </div>
  )
}

function Providers() {
  const [rows, setRows] = useState<Provider[] | null>(null)
  const [err, setErr] = useState('')
  const [form, setForm] = useState({ name: '', base_url: '', api_key: '', model_id: '', capability: 'chat' })
  const [testMsg, setTestMsg] = useState<Record<string, string>>({})
  const [modelOptions, setModelOptions] = useState<string[]>([])
  const [fetchingModels, setFetchingModels] = useState(false)
  const [modelErr, setModelErr] = useState('')
  const [editingId, setEditingId] = useState<string | null>(null)
  const [showForm, setShowForm] = useState(false)
  const load = () => api.get<Provider[]>('/settings/llm/providers').then(setRows).catch((e) => setErr(e.message))
  const closeForm = () => {
    setShowForm(false)
    setEditingId(null)
    setForm({ name: '', base_url: '', api_key: '', model_id: '', capability: 'chat' })
  }
  useEffect(() => {
    load()
  }, [])
  return (
    <div className="space-y-4">
      {showForm ? (
      <Card className="p-4">
        <div className="mb-3 flex items-center justify-between">
          <h3 className="text-sm font-semibold">{editingId ? '编辑供应商' : '注册供应商'}</h3>
          <button type="button" onClick={closeForm} className="text-xs text-muted-foreground transition-colors hover:text-foreground">收起</button>
        </div>
        <form
          className="grid gap-3 md:grid-cols-2"
          onSubmit={async (e) => {
            e.preventDefault()
            try {
              if (editingId) {
                // name 不可改；key 留空表示不更新
                const body: Record<string, unknown> = { base_url: form.base_url, model_id: form.model_id, capability: form.capability }
                if (form.api_key) body.api_key = form.api_key
                await api.put(`/settings/llm/providers/${editingId}`, body)
              } else {
                await api.post('/settings/llm/providers', { ...form, is_default: rows?.length === 0 })
              }
              setForm({ name: '', base_url: '', api_key: '', model_id: '', capability: 'chat' })
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
            <div className="flex items-center justify-between">
              <label htmlFor="prov-model" className="block text-xs font-medium">模型 ID</label>
              <button
                type="button"
                className="text-xs text-muted-foreground underline underline-offset-4 transition-colors hover:text-foreground disabled:opacity-50"
                disabled={fetchingModels || !form.base_url.trim()}
                title="从供应商拉取可用模型列表（OpenAI 兼容 GET /models）；拉取失败可手动输入"
                onClick={async () => {
                  setFetchingModels(true)
                  setModelErr('')
                  try {
                    const r = await api.post<{ models: string[] }>('/settings/llm/providers/models', {
                      base_url: form.base_url,
                      api_key: form.api_key.trim() || undefined,
                      provider_id: editingId ?? undefined,
                    })
                    setModelOptions(r.models)
                  } catch (ex) {
                    setModelErr(ex instanceof Error ? ex.message : '拉取模型列表失败——请手动输入')
                  } finally {
                    setFetchingModels(false)
                  }
                }}
              >
                {fetchingModels ? '获取中…' : '自动获取模型'}
              </button>
            </div>
            <input
              id="prov-model"
              className={`${inputCls} w-full font-mono`}
              placeholder="MiniMax-M3（可手动输入；填好地址和 Key 后可自动获取）"
              value={form.model_id}
              onChange={(e) => setForm({ ...form, model_id: e.target.value })}
            />
            {modelOptions.length > 0 && (
              <select
                className={`${selectCls} w-full`}
                aria-label="从获取的模型列表选择"
                value=""
                onChange={(e) => {
                  if (e.target.value) setForm({ ...form, model_id: e.target.value })
                }}
              >
                <option value="">— 从获取到的 {modelOptions.length} 个模型中选择 —</option>
                {modelOptions.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            )}
            {modelErr && <p className="text-xs text-destructive">{modelErr}</p>}
          </div>
          <div className="space-y-1.5">
            <label htmlFor="prov-cap" className="block text-xs font-medium">类型</label>
            <select
              id="prov-cap"
              className={`${selectCls} w-full`}
              value={form.capability}
              onChange={(e) => setForm({ ...form, capability: e.target.value })}
            >
              <option value="chat">对话模型（生成文字）</option>
              <option value="embedding">向量模型（文字转向量）</option>
            </select>
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
                  closeForm()
                }}
              >
                取消
              </Button>
            )}
          </div>
        </form>
      </Card>
      ) : (
        <Button
          onClick={() => {
            setEditingId(null)
            setForm({ name: '', base_url: '', api_key: '', model_id: '', capability: 'chat' })
            setShowForm(true)
          }}
        >
          {rows?.length ? '注册新供应商' : '注册供应商'}
        </Button>
      )}
      {err && <ErrorBox msg={err} />}
      {rows === null ? (
        <Spinner />
      ) : rows.length === 0 ? (
        <Empty text="未配置 provider" />
      ) : (
        rows.map((p) => (
          <Card key={p.id} className="p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div className="min-w-0">
                <p className="font-medium">
                  {p.name}{' '}
                  <span className="ml-1 rounded border border-border px-1.5 py-0.5 align-middle text-xs text-muted-foreground">
                    {p.capability === 'embedding' ? '向量' : '对话'}
                  </span>
                  {p.is_default && <span className="ml-1 text-xs text-success">默认</span>}
                </p>
                <p className="mt-0.5 font-mono text-sm">{p.model_id}</p>
                <p className="mt-1 text-xs text-muted-foreground">{p.base_url}</p>
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
                    setEditingId(p.id)
                    setForm({ name: p.name, base_url: p.base_url, api_key: '', model_id: p.model_id, capability: p.capability })
                    setErr('')
                    setShowForm(true)
                  }}
                >
                  编辑
                </Button>
                <Button
                  size="sm"
                  variant="destructive"
                  onClick={async () => {
                    if (!confirm(`删除「${p.name}」？该操作不可撤销。`)) return
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
            </div>
            {testMsg[p.id] && <p className="mt-2 text-xs text-foreground">{testMsg[p.id]}</p>}
          </Card>
        ))
      )}
    </div>
  )
}

function Routing() {
  const [routing, setRouting] = useState<Record<string, Array<{ provider: string; model: string }>> | null>(null)
  const [providers, setProviders] = useState<Provider[]>([])
  const [msg, setMsg] = useState('')
  const [busy, setBusy] = useState(false)
  const load = () => {
    api
      .get<Record<string, Array<{ provider: string; model: string }>>>('/settings/llm/routing')
      .then(setRouting)
      .catch(() => setRouting({}))
    api.get<Provider[]>('/settings/llm/providers').then(setProviders).catch(() => setProviders([]))
  }
  useEffect(() => {
    load()
  }, [])

  const capFor = (key: string) => (key === 'embed' ? 'embedding' : 'chat')
  const defaultFor = (key: string) => providers.find((p) => p.capability === capFor(key) && p.is_default)
  const currentFor = (key: string) => routing?.[key]?.[0]?.provider ?? ''

  const save = async (key: string, providerName: string) => {
    setBusy(true)
    setMsg('')
    try {
      const next: Record<string, Array<{ provider: string; model: string }>> = { ...(routing ?? {}) }
      if (providerName === '') {
        delete next[key]
      } else {
        const p = providers.find((x) => x.name === providerName)
        if (p) next[key] = [{ provider: p.name, model: p.model_id }]
      }
      await api.put('/settings/llm/routing', next)
      setRouting(next)
      const label = PURPOSES.find((x) => x.key === key)?.label ?? key
      setMsg(providerName === '' ? `「${label}」已切回默认` : `「${label}」已配 ${providerName}`)
    } catch (ex) {
      setMsg(ex instanceof Error ? ex.message : '保存失败')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        系统里共有 <span className="font-medium text-foreground">{PURPOSES.length}</span> 个 AI 功能，逐个给它们选 API；没选的走默认供应商。
      </p>

      <Card className="divide-y divide-border">
        {PURPOSES.map((p) => {
          const matches = providers.filter((x) => x.capability === capFor(p.key))
          const cur = currentFor(p.key)
          const dft = defaultFor(p.key)
          return (
            <div key={p.key} className="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
              <div className="min-w-0 flex-1">
                <p className="font-medium">
                  {p.label} <span className="font-mono text-xs text-muted-foreground">{p.key}</span>
                </p>
                <p className="mt-0.5 text-xs text-muted-foreground">{p.desc}</p>
              </div>
              {matches.length === 0 ? (
                <span className="text-xs text-muted-foreground">无匹配供应商（去「供应商」注册）</span>
              ) : (
                <select
                  className={`${selectCls} w-56`}
                  value={cur}
                  onChange={(e) => save(p.key, e.target.value)}
                  disabled={busy}
                  aria-label={`${p.label} 配 API`}
                >
                  <option value="">用默认{dft ? `（${dft.name}）` : '（未设默认）'}</option>
                  {matches.map((prov) => (
                    <option key={prov.name} value={prov.name}>
                      {prov.name}（{prov.model_id}）
                    </option>
                  ))}
                </select>
              )}
            </div>
          )
        })}
      </Card>

      {msg && <p className="text-xs text-muted-foreground">{msg}</p>}
    </div>
  )
}

/** scope 中文标签（与后端 SCOPES 九项对齐）。 */
const SCOPE_LABELS: Record<string, string> = {
  memory: '记忆',
  wiki: 'Wiki',
  codegraph: '代码图谱',
  project: '项目',
  skills: '技能',
  todos: '待办',
  llm: 'LLM 网关',
  erase: '擦除（不可逆删除）',
  cron: '节律心跳',
}

function Keys() {
  const [rows, setRows] = useState<ApiKey[] | null>(null)
  const [newKey, setNewKey] = useState('')
  const [name, setName] = useState('')
  const [scopes, setScopes] = useState<Set<string>>(new Set(['memory']))
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [editing, setEditing] = useState<ApiKey | null>(null)
  const [editName, setEditName] = useState('')
  const [editScopes, setEditScopes] = useState<Set<string>>(new Set())
  const [editBusy, setEditBusy] = useState(false)
  const [editMsg, setEditMsg] = useState('')
  const load = () => api.get<ApiKey[]>('/settings/api-keys').then(setRows).catch(() => {})
  const active = rows?.filter((k) => !k.revoked_at) ?? []
  const allSelected = active.length > 0 && active.every((k) => selected.has(k.id))
  const toggleAll = () => setSelected(allSelected ? new Set() : new Set(active.map((k) => k.id)))
  const toggle = (id: string) => {
    const next = new Set(selected)
    if (next.has(id)) next.delete(id)
    else next.add(id)
    setSelected(next)
  }
  const toggleScope = (s: string) => {
    const next = new Set(scopes)
    if (next.has(s)) next.delete(s)
    else next.add(s)
    setScopes(next)
  }
  const startEdit = (k: ApiKey) => {
    setEditing(k)
    setEditName(k.name)
    setEditScopes(new Set(k.scopes))
    setEditMsg('')
  }
  const toggleEditScope = (s: string) => {
    const next = new Set(editScopes)
    if (next.has(s)) next.delete(s)
    else next.add(s)
    setEditScopes(next)
  }
  const saveEdit = async () => {
    if (!editing) return
    setEditBusy(true)
    setEditMsg('')
    try {
      await api.put<ApiKey>(`/settings/api-keys/${editing.id}`, {
        name: editName.trim(),
        scopes: [...editScopes],
      })
      setEditing(null)
      load()
    } catch (ex) {
      setEditMsg(ex instanceof Error ? ex.message : '保存失败')
    } finally {
      setEditBusy(false)
    }
  }
  useEffect(() => {
    load()
  }, [])
  return (
    <div className="space-y-4">
      <Card className="p-4">
        <h3 className="text-sm font-semibold">签发新密钥</h3>
        <p className="mt-0.5 text-xs text-muted-foreground">给 AI 客户端签发 amk_ 密钥（scope 在签发时选择，至少一个）</p>
        <form
          className="mt-3 flex flex-wrap items-center gap-2"
          onSubmit={async (e) => {
            e.preventDefault()
            const r = await api.post<{ key: string }>('/settings/api-keys', { name, scopes: [...scopes] })
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
          <div className="flex flex-wrap items-center gap-x-4 gap-y-2" role="group" aria-label="scope 选择">
            {Object.entries(SCOPE_LABELS).map(([s, label]) => (
              <Checkbox
                key={s}
                checked={scopes.has(s)}
                onChange={() => toggleScope(s)}
                label={s}
              >
                {label}
              </Checkbox>
            ))}
          </div>
          <Button size="sm" type="submit" disabled={scopes.size === 0}>
            签发
          </Button>
        </form>
      </Card>
      {newKey && (
        <Card className="border-success/30 bg-success/10 p-4">
          <p className="text-xs text-muted-foreground">新 key（只显示这一次，给 AI 客户端用）：</p>
          <code className="mt-1.5 block break-all font-mono text-sm">{newKey}</code>
        </Card>
      )}
      {editing && (
        <Card className="border-primary/30 p-4">
          <h3 className="text-sm font-semibold">编辑密钥「{editing.name}」</h3>
          <p className="mt-0.5 text-xs text-muted-foreground">
            scope 调整即时生效，无需重签；key 前缀 {editing.key_prefix}… 不变。
          </p>
          <form
            className="mt-3 flex flex-wrap items-center gap-2"
            onSubmit={(e) => {
              e.preventDefault()
              saveEdit()
            }}
          >
            <label htmlFor="edit-key-name" className="text-sm font-medium">名称</label>
            <input
              id="edit-key-name"
              className={`${inputCls} w-48`}
              value={editName}
              onChange={(e) => setEditName(e.target.value)}
            />
            <div className="flex flex-wrap items-center gap-x-4 gap-y-2" role="group" aria-label="编辑 scope">
              {Object.entries(SCOPE_LABELS).map(([s, label]) => (
                <Checkbox
                  key={s}
                  checked={editScopes.has(s)}
                  onChange={() => toggleEditScope(s)}
                  label={s}
                >
                  {label}
                </Checkbox>
              ))}
            </div>
            <Button size="sm" type="submit" disabled={editBusy || editScopes.size === 0}>
              保存
            </Button>
            <Button size="sm" variant="ghost" type="button" onClick={() => setEditing(null)}>
              取消
            </Button>
          </form>
          {editMsg && <p className="mt-2 text-xs text-destructive">{editMsg}</p>}
        </Card>
      )}
      {rows === null ? (
        <Spinner />
      ) : active.length === 0 ? (
        <Empty text="无 API key" />
      ) : (
        <Card className="overflow-hidden">
          {selected.size > 0 && (
            <div className="flex items-center justify-between border-b border-border bg-muted/40 px-3 py-2">
              <span className="text-xs text-muted-foreground">已选 {selected.size} 把</span>
              <Button
                variant="destructive"
                size="sm"
                onClick={async () => {
                  if (!confirm(`批量删除 ${selected.size} 把 key？使用它们的 AI 将立即失权。`)) return
                  await api.post('/settings/api-keys/batch-revoke', { ids: [...selected] })
                  setSelected(new Set())
                  load()
                }}
              >
                批量删除
              </Button>
            </div>
          )}
          <div className="overflow-x-auto">
            <table className={tableCls.root}>
              <thead className={tableCls.thead}>
                <tr>
                  <th className={`${tableCls.th} w-10`}>
                    <Checkbox checked={allSelected} onChange={toggleAll} label="全选" />
                  </th>
                  <th className={tableCls.th}>名称</th>
                  <th className={tableCls.th}>前缀</th>
                  <th className={tableCls.th}>scopes</th>
                  <th className={tableCls.th}>创建</th>
                  <th className={tableCls.th}>最近使用</th>
                  <th className={tableCls.th} />
                </tr>
              </thead>
              <tbody>
                {active.map((k) => (
                  <tr key={k.id} className={tableCls.row}>
                    <td className={tableCls.td}>
                      <Checkbox checked={selected.has(k.id)} onChange={() => toggle(k.id)} label={`选择 ${k.name}`} />
                    </td>
                    <td className={`${tableCls.td} font-medium`}>{k.name}</td>
                    <td className={`${tableCls.td} font-mono`}>{k.key_prefix}…</td>
                    <td className={`${tableCls.td} text-xs text-muted-foreground`}>
                      {k.scopes.map((s) => SCOPE_LABELS[s] ?? s).join('、')}
                    </td>
                    <td className={`${tableCls.td} text-muted-foreground`}>{fmtTime(k.created_at)}</td>
                    <td className={`${tableCls.td} text-muted-foreground`}>
                      {k.last_used_at ? fmtTime(k.last_used_at) : '—'}
                    </td>
                    <td className={`${tableCls.td} text-right`}>
                      <div className="flex justify-end gap-1.5">
                        <Button variant="outline" size="sm" onClick={() => startEdit(k)}>
                          编辑
                        </Button>
                        <Button
                          variant="destructive"
                          size="sm"
                          onClick={async () => {
                            if (!confirm(`删除 key「${k.name}」？使用它的 AI 将立即失权。`)) return
                            await api.post(`/settings/api-keys/${k.id}/revoke`)
                            if (editing?.id === k.id) setEditing(null)
                            load()
                          }}
                        >
                          删除
                        </Button>
                      </div>
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

/** 危险操作 tab：主密钥重加密 + 清空记忆库（从设置页底部移入独立子 tab）。 */
function DangerZone() {
  return (
    <div className="space-y-4">
      <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-4">
        <h2 className="text-sm font-semibold text-destructive">危险操作</h2>
        <p className="mt-0.5 text-xs text-muted-foreground">以下操作不可逆，执行前会二次确认。</p>
      </div>
      <ReencryptPane />
      <DeepPurgePane />
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

/** F1 Web：清空记忆库——破坏半径清单 + 确认短语输入（AI 侧同短语走 deep purge API）。 */
function DeepPurgePane() {
  const [open, setOpen] = useState(false)
  const [phrase, setPhrase] = useState('')
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState('')
  const [err, setErr] = useState('')
  const [stats, setStats] = useState<Record<string, number> | null>(null)

  useEffect(() => {
    // 破坏半径清单：四层计数
    if (!open || stats) return
    Promise.all([
      api.get<unknown[]>('/memory/sessions?limit=500'),
      api.get<unknown[]>('/memory/atoms?limit=500'),
      api.get<unknown[]>('/memory/scenarios?limit=500'),
      api.get<unknown[]>('/memory/persona'),
      api.get<unknown[]>('/memory/entities'),
    ])
      .then(([s, a, sc, p, e]) => setStats({ 会话: s.length, 原子: a.length, 场景: sc.length, 画像: p.length, 实体: e.length }))
      .catch(() => setStats({}))
  }, [open, stats])

  const go = async () => {
    if (phrase !== '清空记忆库') return
    setBusy(true)
    setErr('')
    try {
      const res = await api.post<Record<string, number>>('/memory/purge', {
        deep: true,
        confirm: phrase,
      })
      setResult(`已清空：${Object.entries(res).map(([k, v]) => `${k} ${v}`).join('，')}`)
      setPhrase('')
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card className="border-destructive/30 p-4">
      <h3 className="font-medium">清空记忆库</h3>
      <p className="mt-1 text-sm text-muted-foreground">
        四层 + 实体链一键清空（不可逆）：会话 / 原子 / 场景 / 画像 / 实体全部删除。操作会记入审计。
      </p>
      {!open ? (
        <Button variant="destructive" size="sm" className="mt-3" onClick={() => setOpen(true)}>
          清空记忆库…
        </Button>
      ) : (
        <div className="mt-3 space-y-3">
          <div className="rounded-md border border-border p-3 text-sm">
            <p className="mb-1 font-medium">破坏半径（当前数据）</p>
            {stats === null ? (
              <p className="text-muted-foreground">统计中…</p>
            ) : (
              <ul className="grid grid-cols-2 gap-x-4 gap-y-1 font-mono text-xs text-muted-foreground md:grid-cols-3">
                {Object.entries(stats).map(([k, v]) => (
                  <li key={k}>
                    {k}：{v}
                  </li>
                ))}
              </ul>
            )}
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <input
              className={`${inputCls} w-64`}
              placeholder='输入"清空记忆库"确认'
              value={phrase}
              onChange={(e) => setPhrase(e.target.value)}
              aria-label="清空确认短语"
            />
            <Button variant="destructive" size="sm" disabled={busy || phrase !== '清空记忆库'} onClick={go}>
              {busy ? '执行中…' : '执行清空'}
            </Button>
            <Button variant="ghost" size="sm" onClick={() => { setOpen(false); setPhrase(''); setErr(''); setResult('') }}>
              取消
            </Button>
          </div>
          {err && <p className="text-sm text-destructive">{err}</p>}
          {result && <p className="text-sm text-success">{result}</p>}
        </div>
      )}
    </Card>
  )
}

// ---------- 节律（memory-rhythm：外部 cron 的观察面） ----------
// cron 住在外部（crontab），server 只观察不控制：心跳逾期、积压年龄、
// 安装向导与节律事件全部从既有数据面（jobs 表 + rhythm/status）读出。

type RhythmStatus = {
  last_heartbeat?: string | null
  last_heartbeat_by?: string | null
  pending_sessions: number
  oldest_pending_age_secs?: number | null
}

const EXPECTED_KEY = 'engram-rhythm-expected'
const EXPECTED_OPTIONS = [
  { value: '3600', label: '每小时' },
  { value: '21600', label: '每 6 小时' },
  { value: '86400', label: '每天' },
]

function humanAge(secs: number): string {
  if (secs < 3600) return `${Math.round(secs / 60)} 分钟`
  if (secs < 86400) return `${(secs / 3600).toFixed(1)} 小时`
  return `${(secs / 86400).toFixed(1)} 天`
}

function RhythmPane() {
  const [status, setStatus] = useState<RhythmStatus | null>(null)
  const [events, setEvents] = useState<Job[] | null>(null)
  const [err, setErr] = useState('')
  const [copied, setCopied] = useState(false)
  const [expected, setExpected] = useState(() => {
    // 期望周期持久化在首帧读取（此前 effect 里同步 setState 触发级联渲染告警）
    try {
      return localStorage.getItem(EXPECTED_KEY) ?? '86400'
    } catch {
      return '86400'
    }
  })
  // 页面加载时刻快照——逾期判定基于它（避免 render 期间调用 Date.now 非纯函数）
  const [nowTs] = useState(() => Date.now())

  useEffect(() => {
    api
      .get<RhythmStatus>('/memory/rhythm/status')
      .then(setStatus)
      .catch((e) => setErr(e.message))
    api
      .get<Job[]>('/jobs?limit=100')
      .then((rows) =>
        setEvents(
          rows
            .filter((j) => j.kind === 'rhythm_heartbeat' || (j.payload as Record<string, unknown>)?.reason === 'cron')
            .slice(0, 12),
        ),
      )
      .catch(() => setEvents([]))
  }, [])

  const onExpected = (v: string) => {
    setExpected(v)
    localStorage.setItem(EXPECTED_KEY, v)
  }

  // 逾期判定：超过期望周期 1.5 倍没有心跳 = cron 没来报到
  const expectedSecs = Number(expected)
  const overdue =
    !status?.last_heartbeat
      ? status === null
        ? null
        : 'never'
      : nowTs - new Date(status.last_heartbeat).getTime() > expectedSecs * 1500
        ? 'overdue'
        : 'ok'

  const origin = typeof window !== 'undefined' ? window.location.origin : 'http://127.0.0.1:19180'
  const crontab = [
    '# Engram 记忆节律（外部 cron）——AM 换成本站地址，KEY 换成专用 amk_（设置→API 密钥，memory scope）',
    `AM=${origin}`,
    'KEY="amk_专用密钥"',
    '# 心跳：每次运行报到（设置页据此判定逾期）——via=cron 是防 AI 伪造的显式声明',
    '25 3 * * * curl -s -X POST "$AM/memory/rhythm/heartbeat?via=cron" -H "authorization: Bearer $KEY"',
    '# 每日 full：全量蒸馏 + 整理（consolidate 日桶幂等，重跑安全）',
    `30 3 * * * curl -s -X POST "$AM/memory/distill" -H "authorization: Bearer $KEY" -H 'content-type: application/json' -d '{"full":true,"via":"cron"}'`,
    '# 每 6 小时兜底：扫 pending 会话（AI 写了没蒸馏的由这里接走）',
    `0 */6 * * * curl -s -X POST "$AM/memory/distill" -H "authorization: Bearer $KEY" -H 'content-type: application/json' -d '{"via":"cron"}'`,
  ].join('\n')

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(crontab)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      /* 剪贴板不可用时静默 */
    }
  }

  if (err) return <ErrorBox msg={err} />
  if (!status) return <Spinner />

  return (
    <div className="space-y-6">
      <Card className="p-5 space-y-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="text-sm font-semibold">cron 心跳</h2>
          <label className="flex items-center gap-2 text-xs text-muted-foreground">
            期望周期
            <select
              className={selectCls + ' h-8 w-28'}
              value={expected}
              onChange={(e) => onExpected(e.target.value)}
              aria-label="期望心跳周期"
            >
              {EXPECTED_OPTIONS.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </label>
        </div>
        {overdue === 'ok' && (
          <p className="text-sm text-success">
            ● 在役——最近心跳 {relTime(status.last_heartbeat!)}
            {status.last_heartbeat_by ? `（${status.last_heartbeat_by}）` : ''}
          </p>
        )}
        {overdue === 'overdue' && (
          <p className="text-sm text-destructive">
            ● 逾期——最近心跳 {relTime(status.last_heartbeat!)}
            {status.last_heartbeat_by ? `（${status.last_heartbeat_by}）` : ''}，超过期望周期 1.5 倍。检查
            crontab 是否在跑（crontab -l）与本站可达性。
          </p>
        )}
        {overdue === 'never' && (
          <p className="text-sm text-warning">● 未装——还没有任何心跳记录。按下方安装向导配置外部 cron。</p>
        )}
      </Card>

      <Card className="p-5 space-y-3">
        <h2 className="text-sm font-semibold">会话积压（cron 兜底对象）</h2>
        <p className="text-sm">
          pending 会话 <span className="font-mono tabular-nums">{status.pending_sessions}</span> 条
          {status.oldest_pending_age_secs != null && status.pending_sessions > 0 && (
            <>
              ，最老的已等 <span className="font-mono">{humanAge(status.oldest_pending_age_secs)}</span>
            </>
          )}
          {status.pending_sessions > 0 && (
            <span className="text-muted-foreground">（AI 写入但未蒸馏——cron 会接走；也可手动触发蒸馏）</span>
          )}
        </p>
      </Card>

      <Card className="p-5 space-y-3">
        <div className="flex items-center justify-between">
          <h2 className="text-sm font-semibold">安装向导（crontab 片段）</h2>
          <Button variant="outline" className="h-8" onClick={copy}>
            {copied ? '已复制' : '复制'}
          </Button>
        </div>
        <pre className="overflow-x-auto rounded-md bg-muted p-3 text-xs leading-relaxed">{crontab}</pre>
        <p className="text-xs text-muted-foreground">
          建议签发专用 key（名称如 cron，memory scope）——心跳与触发源都会标记 by，事件流可区分谁在跑。
        </p>
      </Card>

      <Card className="p-5 space-y-3">
        <h2 className="text-sm font-semibold">节律事件（近 12 条）</h2>
        {events === null ? (
          <Spinner />
        ) : events.length === 0 ? (
          <Empty text="还没有节律事件——装好 cron 后这里会出现心跳与触发记录" />
        ) : (
          <ul className="divide-y divide-border/60 text-sm">
            {events.map((j) => (
              <li key={j.id} className="flex items-center gap-3 py-2">
                <StatusBadge status={j.status} />
                <span className="font-mono text-xs">{j.kind}</span>
                <span className="ml-auto text-xs text-muted-foreground" title={fmtTime(j.created_at)}>
                  {relTime(j.created_at)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Card>
    </div>
  )
}

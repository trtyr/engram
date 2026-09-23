/** Settings 域：LLM providers / 路由 / API keys / 节律。 */
import { useEffect, useState } from 'react'
import { appConfirm } from '@/components/confirm'
import { api, type Job, type Provider } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner, StatusBadge, Tabs } from '@/components/ui-bits'
import { fmtTime, inputCls, selectCls, relTime } from '@/lib/ui'
import { Button } from '@/components/ui/button'

type Tab = 'providers' | 'routing' | 'rhythm' | 'danger' | 'migrate'

const TABS: { value: Tab; label: string }[] = [
  { value: 'routing', label: 'AI 功能' },
  { value: 'providers', label: '供应商' },
  { value: 'rhythm', label: '节律' },
  { value: 'danger', label: '危险操作' },
  { value: 'migrate', label: '数据迁移' },
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
      <PageHeader title="设置" desc="LLM 供应商、模型路由与记忆节律；账号/会话与 API 密钥在「账号与安全」页" />
      <Tabs items={TABS} value={tab} onChange={setTab} />
      {tab === 'routing' && <Routing />}
      {tab === 'providers' && <Providers />}
      {tab === 'rhythm' && <RhythmPane />}
      {tab === 'danger' && <DangerZone />}
      {tab === 'migrate' && <MigratePane />}
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
  // ④ 正反向同步（2026-09-18 数据同步线：/migrate/sync 的前端面）
  const [syncUrl, setSyncUrl] = useState('')
  const [syncToken, setSyncToken] = useState('')
  const [syncDir, setSyncDir] = useState<'push' | 'pull'>('push')
  const [syncDry, setSyncDry] = useState(false)
  const [syncReport, setSyncReport] = useState<{
    direction: string
    source_counts: Record<string, unknown>
    dry_run: boolean
    import_report?: Record<string, unknown> | null
  } | null>(null)

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

  async function doSync() {
    if (!syncUrl.trim() || !syncToken) return
    setBusy(true)
    try {
      const r = await api.post<{
        direction: string
        source_counts: Record<string, unknown>
        dry_run: boolean
        import_report?: Record<string, unknown> | null
      }>('/migrate/sync', {
        target_url: syncUrl.trim(),
        direction: syncDir,
        token: syncToken,
        dry_run: syncDry,
      })
      setSyncReport(r)
      setSyncToken('')
      setErr('')
    } catch (e) {
      setErr(e instanceof Error ? e.message : '同步失败')
    } finally {
      setBusy(false)
    }
  }

  // 把导入报告的嵌套树拍平成「分域 | 导入 | 跳过」行（与 CLI 输出同信息量）
  function flattenReport(
    node: unknown,
    prefix = '',
    rows: { path: string; imported: number; skipped: number }[] = [],
  ) {
    if (!node || typeof node !== 'object') return rows
    for (const [k, v] of Object.entries(node as Record<string, unknown>)) {
      if (v && typeof v === 'object') {
        const rec = v as Record<string, unknown>
        if (typeof rec.imported === 'number') {
          rows.push({
            path: prefix + k,
            imported: rec.imported,
            skipped: typeof rec.skipped === 'number' ? rec.skipped : 0,
          })
        } else {
          flattenReport(v, prefix + k + '.', rows)
        }
      }
    }
    return rows
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

      <Card className="p-4">
        <h3 className="text-sm font-semibold">④ 正反向同步（migrate key）</h3>
        <p className="mt-1 text-xs text-muted-foreground">
          推（push）＝本机数据同步到目标；拉（pull）＝目标数据同步到本机。
          目标凭证用 migrate scope 的 API key（「账号与安全」页签发）——admin 密码不过目标网络。
          由本机服务端转发（无 CORS 问题）；非 loopback 目标强制 https。
          合并语义：已存在记录跳过，可重复执行。
        </p>
        <div className="mt-2 flex flex-wrap items-center gap-2">
          <input
            className={`${inputCls} w-64`}
            placeholder="https://cloud.example.com"
            aria-label="目标实例地址"
            value={syncUrl}
            onChange={(e) => setSyncUrl(e.target.value)}
          />
          <input
            className={`${inputCls} w-52`}
            type="password"
            placeholder="目标 migrate key"
            aria-label="目标 migrate key"
            value={syncToken}
            onChange={(e) => setSyncToken(e.target.value)}
          />
          <select
            className={selectCls}
            aria-label="同步方向"
            value={syncDir}
            onChange={(e) => setSyncDir(e.target.value as 'push' | 'pull')}
          >
            <option value="push">推到目标（push）</option>
            <option value="pull">从目标拉取（pull）</option>
          </select>
          <label className="flex items-center gap-1 text-xs text-muted-foreground">
            <input type="checkbox" checked={syncDry} onChange={(e) => setSyncDry(e.target.checked)} />
            先预览（不写入）
          </label>
          <Button
            size="sm"
            disabled={busy || !syncUrl.trim() || !syncToken}
            onClick={doSync}
          >
            {syncDry ? '预览同步内容' : syncDir === 'push' ? '推送到目标' : '从目标拉取'}
          </Button>
        </div>
        {syncReport && (
          <div className="mt-3 space-y-2">
            <h4 className="text-xs font-semibold">
              {syncReport.dry_run
                ? '预览（未写入）——源包分域行数：'
                : `同步方向：${syncReport.direction === 'push' ? '推送到目标' : '从目标拉取'}`}
            </h4>
            <div className="flex flex-wrap gap-x-4 gap-y-1 font-mono text-xs">
              {Object.entries(syncReport.source_counts).map(([k, v]) => (
                <span key={k}>
                  {k}: <span className="text-foreground">{String(v)}</span>
                </span>
              ))}
            </div>
            {syncReport.import_report && (
              <table className="w-full text-xs">
                <thead>
                  <tr className="text-left text-muted-foreground">
                    <th className="py-1">分域</th>
                    <th className="py-1">导入</th>
                    <th className="py-1">跳过</th>
                  </tr>
                </thead>
                <tbody className="font-mono">
                  {flattenReport(syncReport.import_report).map((row) => (
                    <tr key={row.path}>
                      <td className="py-0.5">{row.path}</td>
                      <td className="py-0.5">+{row.imported}</td>
                      <td className="py-0.5 text-muted-foreground">跳过 {row.skipped}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
            {!syncReport.dry_run && syncReport.import_report && (
              <p className="text-xs text-muted-foreground">
                embedding/LLM provider/账号不随迁——导入端重建或重配。
              </p>
            )}
          </div>
        )}
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
                    if (
                      !(await appConfirm({
                        title: `删除「${p.name}」？`,
                        description: '该 provider 配置将移除，不可撤销。',
                        destructive: true,
                        confirmLabel: '删除',
                      }))
                    )
                      return
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

// ---------- 节律（内置节律线 roadmap v3：jobs 基建自续任务） ----------
// 蒸馏节律住在 server 内部：任务「先续期再干活」——跑完自动排下一期（周期桶幂等键防重），
// 启动自检补建；设置页只管配置（GET/PUT /settings/rhythm）与观察（读 jobs 表）。

type RhythmConfig = {
  enabled: boolean
  extract_every_hours: number
  consolidate_hour_local: number
}

const EXTRACT_HOURS = [1, 2, 4, 6, 8, 12, 24]

function RhythmPane() {
  const [cfg, setCfg] = useState<RhythmConfig | null>(null)
  const [jobs, setJobs] = useState<Job[] | null>(null)
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)
  const [saved, setSaved] = useState(false)

  const loadJobs = () =>
    api
      .get<Job[]>('/jobs?kind=rhythm_extract,rhythm_consolidate&limit=50')
      .then(setJobs)
      .catch(() => setJobs([]))

  useEffect(() => {
    api
      .get<RhythmConfig>('/settings/rhythm')
      .then(setCfg)
      .catch((e) => setErr(e.message))
    loadJobs()
  }, [])

  const save = async () => {
    if (!cfg || busy) return
    setBusy(true)
    try {
      setCfg(await api.put<RhythmConfig>('/settings/rhythm', cfg))
      setSaved(true)
      setTimeout(() => setSaved(false), 2000)
      await loadJobs()
    } catch (e) {
      setErr((e as Error).message)
    } finally {
      setBusy(false)
    }
  }

  if (err) return <ErrorBox msg={err} />
  if (!cfg) return <Spinner />

  const pending = (jobs ?? []).filter((j) => j.status === 'pending')
  const finished = (jobs ?? []).filter((j) => j.status !== 'pending' && j.status !== 'running')
  const okCount = (jobs ?? []).filter((j) => j.status === 'succeeded').length
  const rate = jobs !== null && jobs.length > 0 ? Math.round((okCount / jobs.length) * 100) : null
  const nextDue = (kind: string) => {
    const j = pending.find((p) => p.kind === kind)
    return j ? fmtTime(j.due_at) : '—（停用中，或保存后即入队）'
  }

  return (
    <div className="space-y-6">
      <Card className="p-5 space-y-4">
        <h2 className="text-sm font-semibold">节律配置</h2>
        <p className="text-xs text-muted-foreground">
          蒸馏节律住在 server 内部：任务跑完自动排下一期，重启自恢复——无需外部 crontab。
        </p>
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            className="h-4 w-4"
            checked={cfg.enabled}
            onChange={(e) => setCfg({ ...cfg, enabled: e.target.checked })}
          />
          启用节律（停用后在队的最后一期跑完即自然停止）
        </label>
        <div className="flex flex-wrap items-center gap-4">
          <label className="flex items-center gap-2 text-xs text-muted-foreground">
            增量蒸馏周期
            <select
              className={selectCls + ' h-8 w-32'}
              value={cfg.extract_every_hours}
              onChange={(e) => setCfg({ ...cfg, extract_every_hours: Number(e.target.value) })}
              aria-label="增量蒸馏周期"
            >
              {EXTRACT_HOURS.map((h) => (
                <option key={h} value={h}>
                  每 {h} 小时
                </option>
              ))}
            </select>
          </label>
          <label className="flex items-center gap-2 text-xs text-muted-foreground">
            每日全量整理（服务器时区）
            <select
              className={selectCls + ' h-8 w-24'}
              value={cfg.consolidate_hour_local}
              onChange={(e) => setCfg({ ...cfg, consolidate_hour_local: Number(e.target.value) })}
              aria-label="每日整理钟点"
            >
              {Array.from({ length: 24 }, (_, h) => (
                <option key={h} value={h}>
                  {String(h).padStart(2, '0')}:00
                </option>
              ))}
            </select>
          </label>
          <Button variant="outline" className="h-8" onClick={save} disabled={busy}>
            {busy ? '保存中…' : saved ? '已保存' : '保存'}
          </Button>
        </div>
        {!cfg.enabled && (
          <p className="text-sm text-warning">● 已停用——重新打开并保存后，下一期自动入队接上。</p>
        )}
      </Card>

      <Card className="p-5 space-y-3">
        <div className="flex items-center justify-between">
          <h2 className="text-sm font-semibold">运行面（读 jobs 表）</h2>
          {rate !== null && (
            <span className="text-xs text-muted-foreground">
              成功率 <span className="font-mono tabular-nums">{rate}%</span>（{okCount}/{jobs!.length}）
            </span>
          )}
        </div>
        <div className="grid gap-2 text-sm sm:grid-cols-2">
          <div className="rounded-md bg-muted/40 p-3">
            <p className="text-xs text-muted-foreground">下次增量蒸馏</p>
            <p className="font-mono text-xs">{nextDue('rhythm_extract')}</p>
          </div>
          <div className="rounded-md bg-muted/40 p-3">
            <p className="text-xs text-muted-foreground">下次每日整理</p>
            <p className="font-mono text-xs">{nextDue('rhythm_consolidate')}</p>
          </div>
        </div>
        {jobs === null ? (
          <Spinner />
        ) : finished.length === 0 ? (
          <Empty text="还没有执行记录——第一期到期后这里会出现" />
        ) : (
          <ul className="divide-y divide-border/60 text-sm">
            {finished.slice(0, 12).map((j) => (
              <li key={j.id} className="flex items-center gap-3 py-2">
                <StatusBadge status={j.status} />
                <span className="font-mono text-xs">
                  {j.kind === 'rhythm_extract' ? '增量蒸馏' : '每日整理'}
                </span>
                <span
                  className="ml-auto text-xs text-muted-foreground"
                  title={fmtTime(j.finished_at ?? j.created_at)}
                >
                  {relTime(j.finished_at ?? j.created_at)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Card>
    </div>
  )
}

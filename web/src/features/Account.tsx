/**
 * 账号与安全（/account 独立页；公网多Agent P001 / 账号与安全页线）：
 * 账号/会话管理 + API 密钥——面向公网部署的安全操作台。
 * 公网加固（后续步骤）的挂载点：会话过期策略、登录防爆破提示等安全语义在此页扩展。
 */
import { useEffect, useState } from 'react'
import { appConfirm } from '@/components/confirm'
import { api, type AdminSessionDto, type ApiKey } from '@/lib/api'
import { Card, Checkbox, Empty, ErrorBox, PageHeader, Spinner } from '@/components/ui-bits'
import { fmtTime, inputCls, tableCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'

export default function Account() {
  return (
    <div className="space-y-6">
      <PageHeader title="账号与安全" desc="账号/会话管理 + API 密钥——面向公网部署的安全操作台" />
      <AccountPane />
      <Keys />
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
    if (
      !(await appConfirm({
        title: '吊销该会话？',
        description: `会话 ${id} 的设备下次请求需重新登录。`,
        destructive: true,
        confirmLabel: '吊销',
      }))
    )
      return
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
    if (
      !(await appConfirm({
        title: '吊销其他全部会话？',
        description: '除当前设备外的所有登录都将失效。',
        destructive: true,
        confirmLabel: '吊销',
      }))
    )
      return
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
          <div>
            <h3 className="text-sm font-semibold">活跃会话</h3>
            <p className="mt-0.5 text-[11px] text-muted-foreground">每个浏览器的登录状态一条；发现不认识的设备就吊销</p>
          </div>
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
                <span className="font-medium text-foreground/80">{uaLabel(s.user_agent)}</span>
                {s.ip && <span className="font-mono text-muted-foreground">{s.ip}</span>}
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

/** User-Agent → 友好名（浏览器 · 系统）；解析不出返回「未知设备」。 */
function uaLabel(ua: string | null): string {
  if (!ua) return '未知设备'
  const browser = /Edg\//.test(ua)
    ? 'Edge'
    : /OPR\/|Opera/.test(ua)
      ? 'Opera'
      : /Chrome\//.test(ua)
        ? 'Chrome'
        : /Firefox\//.test(ua)
          ? 'Firefox'
          : /Safari\//.test(ua)
            ? 'Safari'
            : /curl/.test(ua)
              ? 'curl'
              : '未知浏览器'
  const os = /Windows/.test(ua)
    ? 'Windows'
    : /Mac OS X|Macintosh/.test(ua)
      ? 'macOS'
      : /Android/.test(ua)
        ? 'Android'
        : /iPhone|iPad|iOS/.test(ua)
          ? 'iOS'
          : /Linux/.test(ua)
            ? 'Linux'
            : ''
  return os ? `${browser} · ${os}` : browser
}

/** scope 中文标签（与后端 SCOPES 十项对齐）。 */
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
  migrate: '迁移同步',
}

function Keys() {
  const [rows, setRows] = useState<ApiKey[] | null>(null)
  const [newKey, setNewKey] = useState('')
  const [name, setName] = useState('')
  const [scopes, setScopes] = useState<Set<string>>(new Set(['memory']))
  const [expiresAt, setExpiresAt] = useState('') // datetime-local；空 = 永不过期（EN-62）
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
            const r = await api.post<{ key: string }>('/settings/api-keys', {
              name,
              scopes: [...scopes],
              ...(expiresAt ? { expires_at: new Date(expiresAt).toISOString() } : {}),
            })
            setNewKey(r.key)
            setName('')
            setExpiresAt('')
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
          <label htmlFor="key-expires" className="text-sm font-medium">
            过期 <span className="text-xs font-normal text-muted-foreground">（可选，留空永不过期）</span>
          </label>
          <input
            id="key-expires"
            type="datetime-local"
            className={`${inputCls} w-56`}
            value={expiresAt}
            onChange={(e) => setExpiresAt(e.target.value)}
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
                  if (
                    !(await appConfirm({
                      title: `批量删除 ${selected.size} 把 key？`,
                      description: '使用它们的 AI 将立即失权。',
                      destructive: true,
                      confirmLabel: '批量删除',
                    }))
                  )
                    return
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
                  <th className={tableCls.th}>过期</th>
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
                    <td className={`${tableCls.td} ${k.expires_at && new Date(k.expires_at) < new Date() ? 'text-destructive' : 'text-muted-foreground'}`}>
                      {k.expires_at ? fmtTime(k.expires_at) : '—'}
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
                            if (
                              !(await appConfirm({
                                title: `删除 key「${k.name}」？`,
                                description: '使用它的 AI 将立即失权。',
                                destructive: true,
                                confirmLabel: '删除',
                              }))
                            )
                              return
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

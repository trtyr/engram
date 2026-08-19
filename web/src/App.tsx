/**
 * 应用壳：登录守卫 + 侧边栏七域导航。
 */
import { NavLink, Route, Routes, useNavigate } from 'react-router-dom'
import { useEffect, useState } from 'react'
import { api, getToken, setToken } from '@/lib/api'
import Dashboard from '@/features/Dashboard'
import Memory from '@/features/Memory'
import Knowledge from '@/features/Knowledge'
import Wiki from '@/features/Wiki'
import CodeGraph from '@/features/CodeGraph'
import Jobs from '@/features/Jobs'
import Settings from '@/features/Settings'
import { Button } from '@/components/ui/button'

const NAV = [
  { to: '/', label: 'Dashboard' },
  { to: '/memory', label: 'Memory' },
  { to: '/knowledge', label: 'Knowledge' },
  { to: '/wiki', label: 'Wiki' },
  { to: '/codegraph', label: 'CodeGraph' },
  { to: '/jobs', label: 'Jobs' },
  { to: '/settings', label: 'Settings' },
] as const

function Login() {
  const [pw, setPw] = useState('')
  const [err, setErr] = useState('')
  const nav = useNavigate()
  return (
    <div className="flex min-h-screen items-center justify-center">
      <form
        className="w-72 space-y-4 rounded-lg border p-6"
        onSubmit={async (e) => {
          e.preventDefault()
          try {
            const r = await api.post<{ token: string }>('/auth/login', { password: pw })
            setToken(r.token)
            nav('/', { replace: true })
          } catch (ex) {
            setErr(ex instanceof Error ? ex.message : '登录失败')
          }
        }}
      >
        <h1 className="text-lg font-semibold">agent-memory</h1>
        <input
          type="password"
          className="w-full rounded-md border bg-transparent px-3 py-2 text-sm"
          placeholder="管理员密码"
          value={pw}
          onChange={(e) => setPw(e.target.value)}
        />
        {err && <p className="text-sm text-red-400">{err}</p>}
        <Button className="w-full" type="submit">
          登录
        </Button>
      </form>
    </div>
  )
}

export default function App() {
  const [authed, setAuthed] = useState<boolean | null>(null)
  useEffect(() => {
    if (!getToken()) {
      setAuthed(false)
      return
    }
    api
      .get('/jobs?limit=1')
      .then(() => setAuthed(true))
      .catch(() => setAuthed(false))
  }, [])

  if (authed === null) return <div className="min-h-screen" />
  if (!authed) return <Login />

  return (
    <div className="flex min-h-screen">
      <aside className="flex w-52 shrink-0 flex-col gap-1 border-r p-4">
        <h1 className="mb-4 px-2 text-lg font-semibold">agent-memory</h1>
        {NAV.map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            className={({ isActive }) =>
              `rounded-md px-3 py-2 text-sm ${isActive ? 'bg-accent text-accent-foreground' : 'text-muted-foreground hover:bg-accent/50'}`
            }
          >
            {item.label}
          </NavLink>
        ))}
      </aside>
      <main className="flex-1 overflow-auto p-8">
        <Routes>
          <Route path="/" element={<Dashboard />} />
          <Route path="/memory" element={<Memory />} />
          <Route path="/knowledge" element={<Knowledge />} />
          <Route path="/wiki" element={<Wiki />} />
          <Route path="/codegraph" element={<CodeGraph />} />
          <Route path="/jobs" element={<Jobs />} />
          <Route path="/settings" element={<Settings />} />
        </Routes>
      </main>
    </div>
  )
}

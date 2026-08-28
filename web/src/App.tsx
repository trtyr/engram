/**
 * 应用壳：登录守卫 + 侧边栏七域导航。
 */
import { NavLink, Navigate, Route, Routes } from 'react-router-dom'
import { useCallback, useEffect, useState } from 'react'
import {
  Brain,
  BookOpen,
  LayoutDashboard,
  ListChecks,
  Network,
  Settings as SettingsIcon,
  Waypoints,
} from 'lucide-react'
import { getToken } from '@/lib/api'
import { cn } from '@/lib/utils'
import { BrandMark } from '@/components/ui-bits'
import Login from '@/features/Login'
import Dashboard from '@/features/Dashboard'
import Memory from '@/features/Memory'
import Knowledge from '@/features/Knowledge'
import Wiki from '@/features/Wiki'
import CodeGraph from '@/features/CodeGraph'
import Jobs from '@/features/Jobs'
import Settings from '@/features/Settings'

const NAV = [
  { to: '/', label: 'Dashboard', icon: LayoutDashboard },
  { to: '/memory', label: 'Memory', icon: Brain },
  { to: '/knowledge', label: 'Knowledge', icon: BookOpen },
  { to: '/wiki', label: 'Wiki', icon: Network },
  { to: '/codegraph', label: 'CodeGraph', icon: Waypoints },
  { to: '/jobs', label: 'Jobs', icon: ListChecks },
  { to: '/settings', label: 'Settings', icon: SettingsIcon },
] as const

/** 已登录的主壳：侧边栏 + 七域路由。 */
function Shell() {
  return (
    <div className="flex min-h-screen">
      <aside className="flex w-56 shrink-0 flex-col border-r border-border bg-card/30">
        <div className="flex items-center gap-2.5 border-b border-border px-4 py-4">
          <BrandMark className="size-7" />
          <span className="text-sm font-semibold tracking-tight">agent-memory</span>
        </div>
        <nav className="flex-1 space-y-1 px-3 py-3">
          {NAV.map((item) => (
            <NavLink
              key={item.to}
              to={item.to}
              end={item.to === '/'}
              className={({ isActive }) =>
                cn(
                  'flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm transition-colors',
                  isActive
                    ? 'bg-brand/10 font-medium text-brand-strong'
                    : 'text-muted-foreground hover:bg-muted hover:text-foreground',
                )
              }
            >
              <item.icon className="size-4 opacity-80" />
              {item.label}
            </NavLink>
          ))}
        </nav>
        <div className="border-t border-border px-4 py-3">
          <p className="text-xs text-muted-foreground/70">v0.1.0</p>
        </div>
      </aside>
      <main className="flex-1 overflow-auto">
        <div className="mx-auto max-w-6xl px-6 py-8 lg:px-8">
          <Routes>
            <Route path="/" element={<Dashboard />} />
            <Route path="/memory" element={<Memory />} />
            <Route path="/knowledge" element={<Knowledge />} />
            <Route path="/wiki" element={<Wiki />} />
            <Route path="/codegraph" element={<CodeGraph />} />
            <Route path="/jobs" element={<Jobs />} />
            <Route path="/settings" element={<Settings />} />
          </Routes>
        </div>
      </main>
    </div>
  )
}

export default function App() {
  const [authed, setAuthed] = useState<boolean | null>(null)

  // 探活（挂载时一次）：401/网络失败 → 登录页
  useEffect(() => {
    if (!getToken()) {
      setAuthed(false)
      return
    }
    let cancelled = false
    fetch('/jobs?limit=1', { headers: { authorization: `Bearer ${getToken()}` } })
      .then((r) => !cancelled && setAuthed(r.ok))
      .catch(() => !cancelled && setAuthed(false))
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // 登录成功的状态上抛（Login 组件 prop）
  const onAuthed = useCallback(() => setAuthed(true), [])

  if (authed === null) return <div className="min-h-screen" />

  return (
    <Routes>
      <Route
        path="/login"
        element={authed ? <Navigate to="/" replace /> : <Login onAuthed={onAuthed} />}
      />
      <Route path="/*" element={authed ? <Shell /> : <Navigate to="/login" replace />} />
    </Routes>
  )
}

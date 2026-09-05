/**
 * 应用壳：登录守卫 + 侧边栏七域导航。
 * Engram：近黑/近白侧栏，选中态整行反转；域页 lazy（路由级分割）。
 * 侧栏四件套（frontend-polish R2）：收缩（localStorage）/ 分区 / 状态徽章 / 全局检索面板。
 */
import { NavLink, Navigate, Route, Routes, useLocation } from 'react-router-dom'
import { Suspense, lazy, useCallback, useEffect, useState } from 'react'
import {
  Brain,
  FolderKanban,
  LayoutDashboard,
  ListChecks,
  LogOut,
  Network,
  PanelLeftClose,
  PanelLeftOpen,
  Plug,
  Puzzle,
  Search,
  Settings as SettingsIcon,
  Users,
  Waypoints,
} from 'lucide-react'
import { clearToken, getToken } from '@/lib/api'
import { useSystemStatus } from '@/lib/status'
import { cn } from '@/lib/utils'
import { BrandMark } from '@/components/ui-bits'
import { ThemeToggle } from '@/components/ThemeToggle'
import { CommandPalette } from '@/components/CommandPalette'
import Login from '@/features/Login'

const Dashboard = lazy(() => import('@/features/Dashboard'))
const Memory = lazy(() => import('@/features/Memory'))
const Circle = lazy(() => import('@/features/Circle'))
const Wiki = lazy(() => import('@/features/Wiki'))
const CodeGraph = lazy(() => import('@/features/CodeGraph'))
const Projects = lazy(() => import('@/features/Projects'))
const ProjectDetail = lazy(() => import('@/features/ProjectDetail'))
const Skills = lazy(() => import('@/features/Skills'))
const Jobs = lazy(() => import('@/features/Jobs'))
const Settings = lazy(() => import('@/features/Settings'))
const Mcp = lazy(() => import('@/features/Mcp'))

type NavItem = {
  to: string
  label: string
  icon: typeof Brain
  badge?: (s: { failed: number; distilling: number }) => number | null
  pulse?: (s: { failed: number; distilling: number }) => boolean
}

/** 分区语义：首屏 ｜ 资产域 ｜ 系统。 */
const NAV_GROUPS: { label: string | null; items: NavItem[] }[] = [
  {
    label: null,
    items: [{ to: '/', label: '概览', icon: LayoutDashboard }],
  },
  {
    label: '资产域',
    items: [
      { to: '/memory', label: '用户记忆', icon: Brain, pulse: (s) => s.distilling > 0 },
      { to: '/circle', label: '圈子', icon: Users },
      { to: '/wiki', label: 'Wiki', icon: Network },
      { to: '/codegraph', label: '代码图谱', icon: Waypoints },
      { to: '/projects', label: '项目', icon: FolderKanban },
      { to: '/skills', label: '技能', icon: Puzzle },
    ],
  },
  {
    label: '系统',
    items: [
      { to: '/jobs', label: '任务', icon: ListChecks, badge: (s) => s.failed || null },
      { to: '/mcp', label: 'MCP', icon: Plug },
      { to: '/settings', label: '设置', icon: SettingsIcon },
    ],
  },
]

/** 已登录的主壳：桌面侧边栏（可收缩）/ 移动端顶部导航条 + 七域路由。 */
function Shell({ onLogout }: { onLogout: () => void }) {
  const [collapsed, setCollapsed] = useState(() => {
    try {
      return localStorage.getItem('engram-sidebar') === 'collapsed'
    } catch {
      return false
    }
  })
  const [openedAt, setOpenedAt] = useState<string | null>(null)
  const status = useSystemStatus()
  const location = useLocation()
  const locationKey = location.pathname + location.search
  // 派生：路由变化后旧位置打开的面板自然关闭（浏览器返回/命中跳转统一收口）
  const paletteOpen = openedAt !== null && openedAt === locationKey

  const toggleCollapsed = useCallback(() => {
    setCollapsed((c) => {
      try {
        localStorage.setItem('engram-sidebar', c ? 'expanded' : 'collapsed')
      } catch {
        /* 忽略 */
      }
      return !c
    })
  }, [])

  // 全局快捷键：Cmd/Ctrl+K 切换面板；/ 直开（输入框内除外）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        setOpenedAt((v) => (v !== null ? null : locationKey))
      } else if (e.key === '/' && openedAt === null) {
        const t = e.target as HTMLElement | null
        if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
        e.preventDefault()
        setOpenedAt(locationKey)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [locationKey, openedAt])

  return (
    <div className="flex min-h-screen md:h-screen md:flex-row flex-col">
      <aside
        className={cn(
          'flex w-full shrink-0 flex-col border-b border-sidebar-border bg-sidebar transition-[width] duration-200',
          collapsed ? 'md:w-15' : 'md:w-52',
          'md:border-b-0 md:border-r',
        )}
      >
        <div
          className={cn(
            'flex items-center justify-between gap-1 border-b border-sidebar-border px-4 py-3 md:py-4 md:pr-2 md:pl-2.5',
            collapsed && 'md:flex-col md:justify-stretch md:gap-1',
          )}
        >
          <div className="flex min-w-0 shrink-0 items-center gap-2.5">
            <BrandMark
              className={cn('shrink-0 text-sidebar-primary', collapsed ? 'md:size-6' : 'size-5')}
            />
            <span
              className={cn(
                'truncate text-sm font-semibold tracking-tight text-sidebar-accent-foreground',
                collapsed && 'md:hidden',
              )}
            >
              Engram
            </span>
          </div>
          <div className="flex items-center gap-0.5">
            <button
              type="button"
              aria-label="全局检索"
              title="全局检索（Cmd/Ctrl+K 或 /）"
              className={cn(
                'rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground',
                collapsed ? 'md:p-2' : 'p-1.5',
              )}
              onClick={() => setOpenedAt(locationKey)}
            >
              <Search className={cn(collapsed ? 'md:size-5' : 'size-4')} aria-hidden="true" />
            </button>
            <div className="md:hidden">
              <ThemeToggle />
            </div>
          </div>
        </div>
        <nav
          className="flex gap-1 overflow-x-auto px-3 py-2 md:flex-1 md:flex-col md:overflow-visible md:py-3"
          aria-label="主导航"
        >
          {NAV_GROUPS.map((group, gi) => (
            <div key={group.label ?? 'root'} className="contents md:block">
              {group.label && (
                <p
                  className={cn(
                    'hidden px-3 pt-4 pb-1 font-mono text-[10px] tracking-wider text-muted-foreground/60 uppercase md:block',
                    collapsed && 'md:hidden',
                  )}
                >
                  {group.label}
                </p>
              )}
              {gi > 0 && (
                <div
                  className={cn('hidden border-t border-sidebar-border md:block', collapsed ? '' : 'md:hidden')}
                  aria-hidden="true"
                />
              )}
              {group.items.map((item) => {
                const pulsing = !!item.pulse?.(status)
                const railLabel = pulsing ? `${item.label} · 蒸馏中` : item.label
                return (
                <NavLink
                  key={item.to}
                  to={item.to}
                  end={item.to === '/'}
                  title={collapsed ? railLabel : undefined}
                  aria-label={collapsed ? railLabel : undefined}
                  className={({ isActive }) =>
                    cn(
                      'relative flex shrink-0 items-center gap-2.5 rounded-md px-3 py-1.5 text-sm font-medium transition-colors',
                      collapsed && 'md:justify-center md:px-0 md:py-2',
                      isActive
                        ? 'bg-foreground text-background'
                        : 'text-muted-foreground hover:bg-muted hover:text-foreground',
                    )
                  }
                >
                  {/* 收起态：脉冲点让位，图标自身呼吸（md:engram-pulse）+ 放大到 20px */}
                  <item.icon
                    className={cn(
                      'shrink-0',
                      collapsed ? 'md:size-5' : 'size-4',
                      collapsed && pulsing && 'md:engram-pulse',
                    )}
                    aria-hidden="true"
                  />
                  <span className={cn(collapsed && 'md:hidden')}>{item.label}</span>
                  {item.badge?.(status) != null && (
                    <>
                      {/* 展开态 / 移动端：计数芯片 */}
                      <span
                        className={cn(
                          'ml-auto rounded border border-destructive/40 px-1 font-mono text-[10px] leading-4 text-destructive',
                          collapsed && 'md:hidden',
                        )}
                      >
                        {item.badge!(status)}
                      </span>
                      {/* 收起态（桌面）：角标圆点 */}
                      <span
                        className={cn(
                          'absolute top-1 right-1 hidden size-1.5 rounded-full bg-destructive md:block',
                          !collapsed && 'md:hidden',
                        )}
                        aria-hidden="true"
                      />
                    </>
                  )}
                  {pulsing && (
                    <span
                      className={cn(
                        'ml-auto size-1.5 shrink-0 rounded-full bg-info engram-pulse',
                        collapsed && 'md:hidden',
                      )}
                      aria-label="蒸馏进行中"
                    />
                  )}
                </NavLink>
                )
              })}
            </div>
          ))}
        </nav>
        <div
          className={cn(
            'flex items-center justify-between border-t border-sidebar-border px-2.5 py-2.5',
            collapsed && 'md:flex-col md:justify-stretch md:gap-1',
          )}
        >
          {/* 系统区：左=版本信息，右=动作簇（收起/主题/登出贴排）；收起态整列纵向堆叠 */}
          <p
            className={cn(
              'pl-1 font-mono text-xs text-muted-foreground/70',
              collapsed && 'md:hidden',
            )}
          >
            v{__APP_VERSION__}
          </p>
          <div className={cn('flex items-center gap-0.5', collapsed && 'md:flex-col md:gap-1')}>
            <button
              type="button"
              aria-label={collapsed ? '展开侧边栏' : '收起侧边栏'}
              title={collapsed ? '展开侧边栏' : '收起侧边栏'}
              className={cn(
                'hidden rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground md:block',
                collapsed ? 'md:p-2' : 'p-1.5',
              )}
              onClick={toggleCollapsed}
            >
              {collapsed ? (
                <PanelLeftOpen className="md:size-5" aria-hidden="true" />
              ) : (
                <PanelLeftClose className="size-4" aria-hidden="true" />
              )}
            </button>
            <ThemeToggle iconClass={collapsed ? 'md:size-5' : 'size-4'} />
            {/* 登出：动作簇末位；hover 走 destructive 语义 */}
            <button
              type="button"
              aria-label="登出"
              title="登出"
              className={cn(
                'rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive',
                collapsed && 'md:p-2',
              )}
              onClick={onLogout}
            >
              <LogOut className={cn(collapsed ? 'md:size-5' : 'size-4')} aria-hidden="true" />
            </button>
          </div>
        </div>
      </aside>
      <main className="flex-1 overflow-auto">
        {/* 1440 封顶 + 24px 边距：常用屏幕（≤1512）两种侧栏状态下内容都撑满可用宽，
            收缩释放的宽度交给内容而非空白；超大屏居中封顶防表格无限拉伸 */}
        <div className="mx-auto w-full max-w-[1440px] px-4 py-6 md:px-6 md:py-6">
          <Suspense fallback={<div className="min-h-40" />}>
            <Routes>
              <Route path="/" element={<Dashboard />} />
              <Route path="/memory" element={<Memory />} />
              <Route path="/circle" element={<Circle />} />
              <Route path="/knowledge" element={<Navigate to="/wiki" replace />} />
              <Route path="/wiki" element={<Wiki />} />
              <Route path="/codegraph" element={<CodeGraph />} />
              <Route path="/projects" element={<Projects />} />
              <Route path="/projects/:id" element={<ProjectDetail />} />
              <Route path="/skills" element={<Skills />} />
              <Route path="/jobs" element={<Jobs />} />
              <Route path="/mcp" element={<Mcp />} />
              <Route path="/settings" element={<Settings />} />
            </Routes>
          </Suspense>
        </div>
      </main>
      {paletteOpen && <CommandPalette onClose={() => setOpenedAt(null)} />}
    </div>
  )
}

export default function App() {
  const [authed, setAuthed] = useState<boolean | null>(null)

  // 探活（挂载时一次）：仅 401（凭证失效）→ 登录页；
  // 5xx（服务抖动/部署窗口）不踢人——否则一次 503 让全员"被登出"（2026-08-31 事故）
  useEffect(() => {
    if (!getToken()) {
      setAuthed(false)
      return
    }
    let cancelled = false
    fetch('/jobs?limit=1', { headers: { authorization: `Bearer ${getToken()}` } })
      .then((r) => {
        if (cancelled) return
        if (r.status === 401) setAuthed(false)
        else if (r.ok) setAuthed(true)
        // 5xx：保持 null → 显示加载态但用户可等；不再误杀会话
      })
      .catch(() => !cancelled && undefined)
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // 会话中途失效（api 层 401 广播）→ 回登录页，不再死加载
  useEffect(() => {
    const onExpired = () => setAuthed(false)
    window.addEventListener('engram-auth-expired', onExpired)
    return () => window.removeEventListener('engram-auth-expired', onExpired)
  }, [])

  // 登录成功的状态上抛（Login 组件 prop）
  const onAuthed = useCallback(() => setAuthed(true), [])

  // 登出：客户端清除会话（后端无 logout 端点，ams_ 随 TTL 自然过期），authed=false 路由自动回 /login
  const onLogout = useCallback(() => {
    clearToken()
    setAuthed(false)
  }, [])

  if (authed === null) return <div className="min-h-screen" />

  return (
    <Routes>
      <Route
        path="/login"
        element={authed ? <Navigate to="/" replace /> : <Login onAuthed={onAuthed} />}
      />
      <Route path="/*" element={authed ? <Shell onLogout={onLogout} /> : <Navigate to="/login" replace />} />
    </Routes>
  )
}

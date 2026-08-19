import { NavLink, Route, Routes } from 'react-router-dom'

/**
 * 应用壳：侧边栏七域导航（Phase 6 逐步充实各域页面）。
 * 域清单与 docs/plantree/plans/agent-memory-platform/topics/frontend.md 一致。
 */
const NAV = [
  { to: '/', label: 'Dashboard' },
  { to: '/memory', label: 'Memory' },
  { to: '/knowledge', label: 'Knowledge' },
  { to: '/wiki', label: 'Wiki' },
  { to: '/codegraph', label: 'CodeGraph' },
  { to: '/jobs', label: 'Jobs' },
  { to: '/settings', label: 'Settings' },
] as const

const STUB: Record<string, string> = {
  '/': '统计与用量总览（Phase 6a）',
  '/memory': '会话 / 原子 / 场景 / 画像（Phase 6b）',
  '/knowledge': '文档摄取与检索（Phase 6c）',
  '/wiki': '页面 / 图谱 / Lint（Phase 6d）',
  '/codegraph': '项目注册与查询（Phase 6e）',
  '/jobs': '任务队列与事件（Phase 6e）',
  '/settings': 'LLM / API Key / 参数（Phase 6e）',
}

export default function App() {
  return (
    <div className="flex min-h-screen bg-background text-foreground">
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
      <main className="flex-1 p-8">
        <Routes>
          {NAV.map((item) => (
            <Route key={item.to} path={item.to} element={<Stub label={STUB[item.to]} />} />
          ))}
        </Routes>
      </main>
    </div>
  )
}

function Stub({ label }: { label: string }) {
  return (
    <div className="rounded-lg border border-dashed p-8 text-sm text-muted-foreground">
      {label} — 骨架占位，Phase 6 实现。
    </div>
  )
}

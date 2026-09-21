import { NavLink } from 'react-router-dom'

/**
 * 代码图谱的两个页面切换（2026-09-21 拆页）：
 * `/codegraph` = **查看**（已导入 + 调用图），`/codegraph/import` = **导入**（两个入口）。
 * 两页共用同一根 sidebar 入口，靠这个 tab 条来回切。
 */
export default function CgTabs() {
  const cls = ({ isActive }: { isActive: boolean }) =>
    `rounded-md px-2.5 py-1 text-sm transition-colors ${
      isActive ? 'bg-muted font-medium text-foreground' : 'text-muted-foreground hover:text-foreground'
    }`
  return (
    <nav className="flex shrink-0 gap-1 rounded-lg border border-border p-1" aria-label="代码图谱页面">
      <NavLink to="/codegraph" end className={cls}>
        查看
      </NavLink>
      <NavLink to="/codegraph/import" className={cls}>
        导入
      </NavLink>
    </nav>
  )
}

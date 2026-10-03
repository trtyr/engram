/**
 * 路由一致性守护测试（2026-10-03）：源码里所有 <Link to="/..."> 的目标必须存在对应 <Route>。
 *
 * 由来：P009 移除 /jobs 页面时，Dashboard 里两处硬编码 <Link to="/jobs"> 被漏掉，
 * 点进去落在 Shell 内未匹配 → 白屏（用户可见回归，审计抓出）。
 * 这类「删路由不等于删引用」的漏网靠人眼扫不住——用测试收口。
 */
import { describe, expect, it } from 'vitest'

/** 源码文本表：vite 在构建期展开为 { '/src/App.tsx': '...', ... }（raw 形式）。 */
const SOURCES = import.meta.glob('./**/*.{ts,tsx}', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

/** 排除测试自身与测试辅助。 */
const RUNTIME_SOURCES = Object.fromEntries(
  Object.entries(SOURCES).filter(([p]) => !/\.test\.tsx?$/.test(p)),
)

/** 从 App.tsx 提取已声明的路由路径（含 :param 形式）。 */
function declaredRoutes(): Set<string> {
  const app = SOURCES['./App.tsx']
  const routes = new Set<string>()
  for (const m of app.matchAll(/<Route\s+path="([^"]+)"/g)) {
    routes.add(m[1])
  }
  return routes
}

/** 判断某个链接目标是否被路由覆盖（支持 /:param 段与 #hash、?query 后缀）。 */
function isCovered(target: string, routes: Set<string>): boolean {
  const clean = target.split('#')[0].split('?')[0]
  if (routes.has(clean)) return true
  // 逐段比对：路由里的 :param 段可匹配任意字面段
  for (const r of routes) {
    const rSeg = r.split('/').filter(Boolean)
    const tSeg = clean.split('/').filter(Boolean)
    if (rSeg.length !== tSeg.length) continue
    if (rSeg.every((s, i) => s.startsWith(':') || s === tSeg[i])) return true
  }
  return false
}

describe('路由一致性（链接目标必须有对应 Route）', () => {
  it('源码里所有 <Link to="/..."> 都能命中已声明路由', () => {
    const routes = declaredRoutes()
    // 非路由域的链接（外部/锚点/登录等）白名单
    const allow = new Set(['/login', '/', '/knowledge', '/file-view'])
    const offenders: string[] = []

    for (const [file, src] of Object.entries(RUNTIME_SOURCES)) {
      for (const m of src.matchAll(/<Link[^>]*\bto=["'](\/[^"']*)["']/g)) {
        const target = m[1]
        if (allow.has(target.split('#')[0].split('?')[0])) continue
        if (!isCovered(target, routes)) {
          offenders.push(`${file} → to="${target}"`)
        }
      }
    }

    expect(offenders, `以下链接指向不存在的路由（点了会白屏）：\n${offenders.join('\n')}`).toEqual([])
  })

  it('导航表（NAV_GROUPS）里的每一项都有对应 Route', () => {
    const app = SOURCES['./App.tsx']
    const routes = declaredRoutes()
    const offenders: string[] = []
    for (const m of app.matchAll(/\{\s*to:\s*'(\/[^']*)'/g)) {
      const target = m[1]
      if (!isCovered(target, routes)) offenders.push(`NAV_GROUPS → to: '${target}'`)
    }
    expect(offenders, `导航项指向不存在的路由：\n${offenders.join('\n')}`).toEqual([])
  })
})

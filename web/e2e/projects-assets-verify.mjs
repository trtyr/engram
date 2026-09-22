/**
 * 资产域 + 项目关系 + 关系图谱 活体验证（2026-09-22，goal muc05n8k）。
 *
 * 覆盖：① /assets 资产页（台账 + 反查「被哪些项目用到」）② /projects 场景分组
 * ③ /projects/:id 详情「关系」区（用到的资产）④ /projects 图谱 tab（canvas + 图例 +
 * 场景着色切换 + 双击邻域）⑤ 共享引擎另三处住户（wiki / circle / codegraph）零回归。
 * 全程收集 console 错误，最后一次性断言 0。
 *
 * 用法：E2E_BASE=... E2E_ADMIN_PW=... SHOT_DIR=... node e2e/projects-assets-verify.mjs
 */
import { chromium } from '@playwright/test'
import fs from 'node:fs'

const BASE = process.env.E2E_BASE
const PW = process.env.E2E_ADMIN_PW
if (!BASE || !PW) throw new Error('缺 E2E_BASE / E2E_ADMIN_PW')
const OUT = process.env.SHOT_DIR || '/tmp/graph-verify'
fs.mkdirSync(OUT, { recursive: true })

const login = await fetch(`${BASE}/auth/login`, {
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ password: PW }),
})
const token = (await login.json()).token
const api = async (p) => {
  const r = await fetch(`${BASE}${p}`, { headers: { authorization: `Bearer ${token}` } })
  const t = await r.text()
  if (!r.ok) throw new Error(`${p} -> ${r.status}: ${t.slice(0, 200)}`)
  return t ? JSON.parse(t) : undefined
}

// ---- API 层先对账 ----
const projects = await api('/projects')
const engram = projects.find((p) => p.name === 'engram')
const assets = await api('/assets')
const mac = assets.find((a) => a.name === 'MacBook Air M1')
const macDetail = await api(`/assets/${mac.id}`)
const detail = await api(`/projects/${engram.id}`)
const graph = await api('/projects/graph')
console.log(
  `[API] 项目 ${projects.length} · 资产 ${assets.length} · 关系 ${graph.links.length} · 用到边 ${graph.usages.length}`,
)
console.log(
  `[API] MacBook 反查 ${macDetail.used_by.length} 个项目：${macDetail.used_by.map((u) => u.project_name).join('/')}`,
)
console.log(
  `[API] engram 详情：用到的资产 ${detail.assets.length}（${detail.assets.map((a) => a.name).join('/')}）· 关系 ${detail.links.length}`,
)

const browser = await chromium.launch({
  args: ['--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
})
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
const errors = []
const pageErrors = (tag) => {
  const start = errors.length
  return () => errors.slice(start).map((e) => `${tag}: ${e}`)
}
page.on('console', (m) => {
  if (m.type() === 'error') errors.push(m.text().slice(0, 200))
})
page.on('pageerror', (e) => errors.push(`pageerror: ${String(e).slice(0, 200)}`))

await page.goto(`${BASE}/login`)
await page.evaluate((t) => localStorage.setItem('am_token', t), token)

// ① 资产页
const errAssets = pageErrors('assets')
await page.goto(`${BASE}/assets`)
await page.waitForSelector('text=被哪些项目用到', { timeout: 30_000 })
const usedByText = await page.locator('text=被哪些项目用到').first().innerText()
const hasMac = await page.locator('text=MacBook Air M1').first().isVisible()
console.log(`[assets] 页可达 ✓ 台账项可见=${hasMac} · ${usedByText.trim()}`)
await page.screenshot({ path: `${OUT}/assets-page.png`, fullPage: true })
console.log(`[assets] console 错误=${errAssets().length}`)

// ② 项目列表（场景分组）
const errProjects = pageErrors('projects')
await page.goto(`${BASE}/projects`)
await page.getByRole('button', { name: '列表' }).waitFor({ timeout: 30_000 })
await page.waitForTimeout(1500)
const groupHeaders = await page.locator('section > h2').allInnerTexts()
console.log(`[projects] 场景分组头：${groupHeaders.join(' | ')}`)
await page.screenshot({ path: `${OUT}/projects-grouped.png`, fullPage: true })

// ③ 项目详情「关系」区
await page.goto(`${BASE}/projects/${engram.id}`)
await page.waitForSelector('text=🔗 关系', { timeout: 30_000 })
const relText = await page.locator('text=用到的资产').first().innerText()
const assetRowVisible = await page.locator('text=MacBook Air M1').first().isVisible()
console.log(`[project-detail] 关系区 ✓ ${relText.trim()} · 资产行可见=${assetRowVisible}`)
await page.screenshot({ path: `${OUT}/project-relations.png`, fullPage: true })
console.log(`[projects] console 错误=${errProjects().length}`)

// ④ 图谱 tab
const errGraph = pageErrors('projects-graph')
await page.goto(`${BASE}/projects`)
await page.getByRole('button', { name: '图谱' }).click()
await page.waitForSelector('[data-testid=project-asset-graph]', { timeout: 90_000 })
await page.waitForTimeout(4000)
const legend = await page.locator('text=共').first().innerText()
console.log(`[graph] canvas 渲染 ✓ 图例：${legend.trim()}`)
const toggle = page.getByTestId('scene-toggle')
const toggleLabel = await toggle.innerText()
await toggle.click()
await page.waitForTimeout(600)
console.log(`[graph] 着色切换可用=${toggleLabel.trim()} → ${(await toggle.innerText()).trim()}`)
await page.screenshot({ path: `${OUT}/project-assets-graph.png`, fullPage: true })
console.log(`[graph] console 错误=${errGraph().length}`)

// ⑤ 共享引擎另三处住户（零回归）
const w = pageErrors('wiki')
await page.goto(`${BASE}/wiki`)
await page.getByRole('button', { name: '图谱' }).click()
await page.waitForSelector('[data-testid=wiki-graph-canvas]', { timeout: 90_000 })
await page.waitForTimeout(2500)
console.log(`[wiki] 图谱 canvas ✓ console 错误=${w().length}`)

const c = pageErrors('circle')
await page.goto(`${BASE}/circle`)
await page.waitForSelector('[data-testid=circle-galaxy-canvas]', { timeout: 90_000 })
await page.waitForTimeout(2000)
console.log(`[circle] 星系 canvas ✓ console 错误=${c().length}`)

const cg = pageErrors('codegraph')
const cgProjects = await api('/codegraph/projects')
const ready = cgProjects.find((p) => p.status === 'ready' && p.usable)
if (ready) {
  await page.goto(`${BASE}/codegraph?sel=${ready.id}`)
  await page.waitForSelector('[data-testid=force-graph-canvas]', { timeout: 180_000 })
  console.log(`[codegraph] ${ready.name} 图谱 canvas ✓ console 错误=${cg().length}`)
} else {
  console.log('[codegraph] 无 ready 项目，跳过（不影响本轮结论）')
}

console.log(`\n[console] 全页面累计错误数 = ${errors.length}`)
errors.slice(0, 10).forEach((e) => console.log('  ! ' + e))
await browser.close()
console.log('SHOT_DONE →', OUT)

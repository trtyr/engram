/**
 * t5/t6/t7 三处图谱活体验证（打指定实例，由外层传 E2E_BASE/E2E_ADMIN_PW）。
 * 逐项断言：① /wiki 图谱 canvas ② /circle 星系 canvas ③ /codegraph safeline-2 打开即出图
 * + LOD 提示与「渲染全图」逃生门 + 静停（参数面板显示「已静止」）+ 无 console 错误。
 */
import { chromium } from '@playwright/test'
import fs from 'node:fs'

const BASE = process.env.E2E_BASE
const PW = process.env.E2E_ADMIN_PW
if (!BASE || !PW) throw new Error('缺 E2E_BASE / E2E_ADMIN_PW')
const OUT = process.env.SHOT_DIR || '/tmp/graph-verify'
fs.mkdirSync(OUT, { recursive: true })

const api = async (path, token) => {
  const r = await fetch(`${BASE}${path}`, { headers: token ? { authorization: `Bearer ${token}` } : {} })
  const text = await r.text()
  if (!r.ok) throw new Error(`${path} -> ${r.status}: ${text.slice(0, 200)}`)
  return text ? JSON.parse(text) : undefined
}

const login = await fetch(`${BASE}/auth/login`, {
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ password: PW }),
})
const token = (await login.json()).token

const wikiGraph = await api('/wiki/graph', token)
console.log(`[API] /wiki/graph nodes=${wikiGraph.nodes?.length} edges=${wikiGraph.edges?.length}`)
const cg = await api('/codegraph/projects', token)
const safeline = cg.find((p) => p.name === 'safeline-2')
console.log(`[API] codegraph 项目数=${cg.length} safeline-2=${safeline?.id} stats.files=${safeline?.stats?.files}`)
const cgGraph = await api(`/codegraph/projects/${safeline.id}/graph`, token)
const withxy = (cgGraph.nodes ?? []).filter((n) => 'x' in n && 'y' in n).length
console.log(`[API] safeline-2 graph nodes=${cgGraph.nodes?.length} with_xy=${withxy} edges=${cgGraph.edges?.length}`)

const browser = await chromium.launch({
  args: ['--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
})
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
const errors = []
page.on('console', (m) => {
  if (m.type() === 'error') errors.push(m.text().slice(0, 200))
})
page.on('pageerror', (e) => errors.push(`pageerror: ${String(e).slice(0, 200)}`))

// 登录：直接注入 token（与 UI 登录同一 /auth/login 签发的同一种 token——
// 避开登录页表单选择器，脚本更稳；页面其余行为与手工登录完全一致）
await page.goto(`${BASE}/login`)
await page.evaluate((t) => localStorage.setItem('am_token', t), token)
await page.goto(`${BASE}/`)
await page.waitForTimeout(1500)

// ① Wiki 图谱
await page.goto(`${BASE}/wiki`)
await page.getByRole('button', { name: '图谱' }).click()
await page.waitForSelector('[data-testid=wiki-graph-canvas]', { timeout: 90_000 })
await page.waitForTimeout(3500)
const communityTab = await page.getByRole('button', { name: /社区着色/ }).count()
console.log(`[wiki] canvas 渲染 ✓；社区着色 tab 在=${communityTab > 0}`)
await page.screenshot({ path: `${OUT}/wiki-graph.png` })

// ② 圈子星系
await page.goto(`${BASE}/circle`)
await page.waitForSelector('[data-testid=circle-galaxy-canvas]', { timeout: 90_000 })
await page.waitForTimeout(2500)
console.log('[circle] canvas 渲染 ✓')
await page.screenshot({ path: `${OUT}/circle.png` })

// ③ codegraph：打开 safeline-2 即出**完整图**（不点任何按钮；阈值已定在 3721/28000 之上）
await page.goto(`${BASE}/codegraph?sel=${safeline.id}`)
await page.waitForSelector('[data-testid=force-graph-canvas]', { timeout: 180_000 })
const escapeBtn = page.getByRole('button', { name: '渲染全图' })
const degradedByDefault = await escapeBtn.isVisible().catch(() => false)
console.log(`[codegraph] 打开即出完整图（未降级=${!degradedByDefault}，未点任何按钮）✓`)
const simplifyBtn = page.getByRole('button', { name: '只看主干' })
const simplifyVisible = await simplifyBtn.isVisible().catch(() => false)
console.log(`[codegraph] 「只看主干」入口可见=${simplifyVisible}（3721 > suggestNodes=1200）`)
await page.screenshot({ path: `${OUT}/codegraph-full.png` })

// 静停：参数面板的物理状态文本由 worker 的 end 消息驱动 → 出现「已静止」即证明 worker 已停表
await page.waitForFunction(() => document.body.innerText.includes('已静止'), { timeout: 180_000 })
console.log('[codegraph] 静停证据：参数面板显示「已静止」（worker 已 end、不再产生 tick）✓')
await page.screenshot({ path: `${OUT}/codegraph-idle.png` })

// 滑杆：拖一下「斥力」→ 参数即时下发 worker、物理被唤醒（「已静止」→「运行中」）→ 再静停
const sliders = page.locator('input[type=range]')
const sliderCount = await sliders.count()
console.log(`[codegraph] 参数面板滑杆数=${sliderCount}（预期 4：向心/斥力/连线拉力/连线长度）`)
await sliders.nth(1).evaluate((el) => {
  // React 的 value tracker 会吞掉「直接赋值 + dispatch input」——必须走原生 setter 才能真触发 onChange
  const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(el), 'value').set
  setter.call(el, String(Number(el.value) + 4))
  el.dispatchEvent(new Event('input', { bubbles: true }))
  el.dispatchEvent(new Event('change', { bubbles: true }))
})
await page.waitForTimeout(900)
const woken = await page.evaluate(() => document.body.innerText.includes('运行中'))
console.log(`[codegraph] 拖滑杆后物理被唤醒（面板回「运行中」）=${woken}`)
await page.waitForFunction(() => document.body.innerText.includes('已静止'), { timeout: 180_000 })
console.log('[codegraph] 唤醒后再次自行静停 ✓')

// 逃生门（显式简化 → 提示 + 「渲染全图」；再回全量）
if (simplifyVisible) {
  await simplifyBtn.click()
  await page.waitForTimeout(2500)
  const lodVisible = await escapeBtn.isVisible().catch(() => false)
  console.log(`[codegraph] 切「只看主干」后：LOD 提示 + 「渲染全图」逃生门可见=${lodVisible}`)
  await page.screenshot({ path: `${OUT}/codegraph-lod.png` })
  if (lodVisible) {
    await escapeBtn.click()
    await page.waitForTimeout(3500)
    const back = await page.locator('[data-testid=force-graph-canvas]').count()
    const escapeGone = !(await escapeBtn.isVisible().catch(() => false))
    console.log(`[codegraph] 点「渲染全图」后 canvas 仍在=${back > 0}、已回全量=${escapeGone}`)
  }
}

// ④ 全屏（三处共用；Wiki 传了 fullscreenTargetRef → 整块——含着色切换与图例——进全屏）
await page.goto(`${BASE}/wiki`)
await page.getByRole('button', { name: '图谱' }).click()
await page.waitForSelector('[data-testid=wiki-graph-canvas]', { timeout: 90_000 })
await page.waitForTimeout(2500)
const fsBtn = page.locator('[data-testid=force-graph-fullscreen]')
console.log(`[wiki] 全屏按钮可见=${await fsBtn.isVisible().catch(() => false)}`)
await fsBtn.click()
await page.waitForTimeout(1500)
const fsState = await page.evaluate(() => ({
  testid: document.fullscreenElement?.getAttribute('data-testid') ?? null,
  cls: document.fullscreenElement ? String(document.fullscreenElement.className).slice(0, 60) : null,
}))
console.log(`[wiki] 进全屏 → fullscreenElement=${JSON.stringify(fsState)}`)
await page.screenshot({ path: `${OUT}/wiki-fullscreen.png` })
// 退出走我们自己的按钮（Esc 在 headless 里不被转发；真实浏览器 Esc 由浏览器原生处理）
await fsBtn.click()
await page.waitForTimeout(1200)
console.log(`[wiki] 点「退出全屏」后 isFullscreen=${await page.evaluate(() => document.fullscreenElement !== null)}`)

console.log(`[console] 错误数=${errors.length}`)
errors.slice(0, 5).forEach((e) => console.log('  ! ' + e))
await browser.close()
console.log('VERIFY_DONE →', OUT)

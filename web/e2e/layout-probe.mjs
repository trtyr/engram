/**
 * 布局探针：量出「内容结束后的空白」到底归谁——打印关键容器与「空白大块」候选。
 * 用法：E2E_BASE=... E2E_ADMIN_PW=... node e2e/layout-probe.mjs
 */
import { chromium } from '@playwright/test'

const BASE = process.env.E2E_BASE
const PW = process.env.E2E_ADMIN_PW
if (!BASE || !PW) throw new Error('缺 E2E_BASE / E2E_ADMIN_PW')

const login = await fetch(`${BASE}/auth/login`, {
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ password: PW }),
})
const token = (await login.json()).token
const cg = await (await fetch(`${BASE}/codegraph/projects`, { headers: { authorization: `Bearer ${token}` } })).json()
const safeline = cg.find((p) => p.name === 'safeline-2')

const browser = await chromium.launch({
  args: ['--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
})
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
await page.goto(`${BASE}/login`)
await page.evaluate((t) => localStorage.setItem('am_token', t), token)

const dump = async (label) => {
  const data = await page.evaluate(() => {
    const h = (el) => Math.round(el.getBoundingClientRect().height)
    const t = (el) => Math.round(el.getBoundingClientRect().top)
    const cls = (el) => String(el.className ?? '').slice(0, 70)
    const chain = []
    const canvas = document.querySelector(
      '[data-testid=force-graph-canvas], [data-testid=wiki-graph-canvas], [data-testid=circle-galaxy-canvas]',
    )
    let el = canvas
    while (el && el !== document.body.parentElement) {
      chain.push(`  ${el.tagName.toLowerCase()} h=${h(el)} top=${t(el)} ${cls(el)}`)
      el = el.parentElement
    }
    // 「空白大块」候选：高度 > 80、无文本、且不含任何有文本的后代
    const empties = []
    document.querySelectorAll('div,section,aside,main').forEach((d) => {
      const r = d.getBoundingClientRect()
      if (r.height < 80 || r.width < 200) return
      const hasText = (d.innerText ?? '').trim().length > 0
      if (hasText) return
      empties.push(`  h=${Math.round(r.height)} w=${Math.round(r.width)} top=${Math.round(r.top)} ${cls(d)}`)
    })
    return {
      viewport: window.innerHeight,
      scrollH: document.documentElement.scrollHeight,
      canvasH: canvas ? h(canvas) : null,
      chain,
      empties: empties.slice(0, 12),
    }
  })
  console.log(`\n===== ${label} =====`)
  console.log(`viewport=${data.viewport} documentScrollH=${data.scrollH} canvasH=${data.canvasH}`)
  console.log('— canvas 祖先链（由内到外）—')
  data.chain.forEach((c) => console.log(c))
  console.log('— 空白大块候选（无文本、h>80、w>200）—')
  data.empties.forEach((c) => console.log(c))
}

// ① 代码图谱：safeline-2
await page.goto(`${BASE}/codegraph?sel=${safeline.id}`)
await page.waitForSelector('[data-testid=force-graph-canvas]', { timeout: 180_000 })
await page.waitForTimeout(4000)
await dump('代码图谱 /codegraph')

// ② Wiki 图谱
await page.goto(`${BASE}/wiki`)
await page.getByRole('button', { name: '图谱' }).click()
await page.waitForSelector('[data-testid=wiki-graph-canvas]', { timeout: 90_000 })
await page.waitForTimeout(4000)
await dump('Wiki 图谱 /wiki')

await browser.close()
console.log('\nPROBE_DONE')

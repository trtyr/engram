/**
 * 项目页场景分类截图（场景值域扩类后的人工复核用）。
 * 用法：E2E_BASE=... E2E_ADMIN_PW=... node e2e/projects-scene-shot.mjs
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

const browser = await chromium.launch({
  args: ['--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
})
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
const errors = []
page.on('console', (m) => {
  if (m.type() === 'error') errors.push(m.text().slice(0, 160))
})
page.on('pageerror', (e) => errors.push(`pageerror: ${String(e).slice(0, 160)}`))

await page.goto(`${BASE}/login`)
await page.evaluate((t) => localStorage.setItem('am_token', t), token)
await page.goto(`${BASE}/projects`)
await page.waitForTimeout(2500)

// 场景筛选下拉的选项（应含六场景）
const filterOptions = await page.locator('select').first().locator('option').allInnerTexts()
console.log('[projects] 筛选下拉选项:', filterOptions.join(' / '))

// 切到运维场景，确认只剩两件事
await page.locator('select').first().selectOption('ops')
await page.waitForTimeout(1200)
const names = await page.locator('a[href^="/projects/"]').allInnerTexts()
console.log('[projects] ops 场景下的项目:', names.map((s) => s.trim()).join(' | '))
await page.screenshot({ path: `${OUT}/projects-ops.png` })

await page.locator('select').first().selectOption('')
await page.waitForTimeout(1200)
await page.screenshot({ path: `${OUT}/projects-all.png` })
console.log(`[console] 错误数=${errors.length}`)
errors.slice(0, 5).forEach((e) => console.log('  ! ' + e))
await browser.close()
console.log('SHOT_DONE →', OUT)

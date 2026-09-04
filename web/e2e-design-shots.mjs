/**
 * 设计审计截图：登录 + 七域页 + Wiki 全量渲染页 + 图谱。
 * 用法：E2E_BASE=http://127.0.0.1:19180 E2E_ADMIN_PW=design-audit-pw node e2e-design-shots.mjs
 */
import { chromium } from '@playwright/test'
import fs from 'node:fs'

const BASE = process.env.E2E_BASE ?? 'http://127.0.0.1:19180'
const PW = process.env.E2E_ADMIN_PW
if (!PW) throw new Error('缺 E2E_ADMIN_PW')
const OUT = 'docs/design/screenshots'
fs.mkdirSync(OUT, { recursive: true })

const shots = [
  { name: '01-login', path: '/login', settle: 600 },
  { name: '02-dashboard', path: '/', settle: 1200 },
  { name: '03-memory-sessions', path: '/memory', settle: 1200 },
  { name: '04-memory-persona', path: '/memory', click: 'text=画像', settle: 1000 },
  { name: '06-wiki-pages', path: '/wiki', settle: 1200 },
  { name: '07-wiki-markdown-full', path: '/wiki?page=markdown-kitchen-sink', settle: 3500 },
  { name: '08-wiki-graph', path: '/wiki', click: 'text=图谱', settle: 2500 },
  { name: '09-wiki-insights', path: '/wiki', click: 'text=洞察', settle: 2000 },
  { name: '10-codegraph', path: '/codegraph', settle: 1200 },
  { name: '11-jobs', path: '/jobs', settle: 1200 },
  { name: '12-settings', path: '/settings', settle: 1500 },
]

const browser = await chromium.launch()
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })

// 登录
await page.goto(`${BASE}/login`)
await page.getByLabel('管理员密码').fill(PW)
await page.getByRole('button', { name: '登录' }).click()
await page.waitForURL(`${BASE}/`, { timeout: 10_000 })
await page.locator('aside nav a[href="/memory"]').waitFor({ timeout: 10_000 })

for (const s of shots) {
  await page.goto(`${BASE}${s.path}`)
  await page.waitForTimeout(s.settle ?? 1000)
  if (s.click) {
    try { await page.locator(s.click).first().click({ timeout: 3000 }) } catch { /* 记录即可 */ }
    await page.waitForTimeout(s.settle ?? 1000)
  }
  await page.screenshot({ path: `${OUT}/${s.name}.png`, fullPage: true })
  console.log('shot:', s.name)
}

// Jobs：点开一行看事件时间线
await page.goto(`${BASE}/jobs`)
await page.waitForTimeout(800)
try {
  await page.locator('tbody tr').first().click({ timeout: 3000 })
  await page.waitForTimeout(800)
  await page.screenshot({ path: `${OUT}/13-jobs-events.png`, fullPage: true })
  console.log('shot: 13-jobs-events')
} catch (e) { console.log('jobs events skip:', e.message.slice(0, 80)) }

// Memory atoms tab（治理视图）
await page.goto(`${BASE}/memory`)
await page.waitForTimeout(800)
try {
  await page.getByRole('button', { name: '原子' }).click()
  await page.waitForTimeout(900)
  await page.screenshot({ path: `${OUT}/14-memory-atoms.png`, fullPage: true })
  console.log('shot: 14-memory-atoms')
} catch (e) { console.log('atoms skip:', e.message.slice(0, 80)) }

await browser.close()
console.log('done →', OUT)

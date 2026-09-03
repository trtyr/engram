/**
 * Wiki 新 IA 双主题截图：目录树 / 图谱独立视图 / 运维二级入口。
 * 用法（一次性栈，由外层 bash 起）：E2E_BASE=... E2E_ADMIN_PW=... node e2e/wiki-shots.mjs
 */
import { chromium } from '@playwright/test'
import fs from 'node:fs'

const BASE = process.env.E2E_BASE
const PW = process.env.E2E_ADMIN_PW
if (!BASE || !PW) throw new Error('缺 E2E_BASE / E2E_ADMIN_PW')
const OUT = 'docs/design/screenshots'
fs.mkdirSync(OUT, { recursive: true })

async function api(method, path, body, token) {
  const headers = { 'content-type': 'application/json' }
  if (token) headers.authorization = `Bearer ${token}`
  const r = await fetch(`${BASE}${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  const text = await r.text()
  if (!r.ok) throw new Error(`${method} ${path} -> ${r.status}: ${text.slice(0, 160)}`)
  return text ? JSON.parse(text) : undefined
}

const login = await api('POST', '/auth/login', { password: PW })
const token = login.token
await api('PUT', '/wiki/pages/shot-async', { title: '异步', content: '# 异步\n\nTokio 是异步运行时。', folder: '技术/Rust' }, token)
await api('PUT', '/wiki/pages/shot-net', { title: '网络', content: '# 网络\n\nHTTP 基础。', folder: '技术' }, token)
await api('PUT', '/wiki/pages/shot-root', { title: '根页面', content: '# 根页面\n\n无文件夹。' }, token)

const browser = await chromium.launch()
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
await page.goto(`${BASE}/login`)
await page.getByLabel('管理员密码').fill(PW)
await page.getByRole('button', { name: '登录' }).click()
await page.waitForURL(`${BASE}/`, { timeout: 10_000 })
await page.goto(`${BASE}/wiki`)
await page.waitForTimeout(1200)

for (const theme of ['light', 'dark']) {
  await page.evaluate((t) => {
    localStorage.setItem('engram-theme', t)
    document.documentElement.classList.toggle('dark', t === 'dark')
  }, theme)
  await page.waitForTimeout(500)

  await page.screenshot({ path: `${OUT}/r31-wiki-tree-${theme}.png`, fullPage: true })

  await page.getByRole('button', { name: '图谱' }).click()
  await page.waitForTimeout(2500)
  await page.screenshot({ path: `${OUT}/r31-wiki-graph-${theme}.png`, fullPage: true })
  await page.getByRole('button', { name: '目录' }).click()
  await page.waitForTimeout(300)

  await page.getByRole('button', { name: '运维' }).click()
  await page.waitForTimeout(800)
  await page.screenshot({ path: `${OUT}/r31-wiki-ops-${theme}.png`, fullPage: true })
  await page.getByRole('button', { name: '← 返回 Wiki' }).click()
  await page.waitForTimeout(300)
}

await browser.close()
console.log('done →', OUT)

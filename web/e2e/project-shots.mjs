/** 项目记忆双视图截图：列表页 + 详情页（左树右内容）。 */
import { chromium } from '@playwright/test'
import fs from 'node:fs'

const BASE = process.env.E2E_BASE ?? 'http://127.0.0.1:8090'
const PW = process.env.E2E_ADMIN_PW ?? 'dev-pw'
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
const projects = await api('GET', '/projects', undefined, token)
if (!projects.length) throw new Error('无项目，先建一个')
const pid = projects[0].id

const browser = await chromium.launch()
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
await page.goto(`${BASE}/login`)
await page.getByLabel('管理员密码').fill(PW)
await page.getByRole('button', { name: '登录' }).click()
await page.waitForURL(`${BASE}/`, { timeout: 10_000 })

await page.goto(`${BASE}/projects`)
await page.waitForTimeout(1200)
await page.screenshot({ path: `${OUT}/project-list-light.png`, fullPage: true })

await page.goto(`${BASE}/projects/${pid}`)
await page.waitForTimeout(1200)
await page.screenshot({ path: `${OUT}/project-detail-light.png`, fullPage: true })

await browser.close()
console.log('done →', OUT)

import { test } from '@playwright/test'

const ADMIN_PW = process.env.E2E_ADMIN_PW ?? 'dev-admin'

test('debug sigma', async ({ page }) => {
  const logs: string[] = []
  page.on('console', (m) => logs.push(`[${m.type()}] ${m.text().slice(0, 300)}`))
  page.on('pageerror', (e) => logs.push(`PAGEERROR: ${e.message.slice(0, 400)}`))
  await page.goto('/')
  await page.getByPlaceholder('管理员密码').fill(ADMIN_PW)
  await page.getByRole('button', { name: '登录' }).click()
  await page.getByRole('link', { name: 'Memory' }).waitFor({ timeout: 10_000 })
  await page.getByRole('link', { name: 'Wiki' }).click()
  await page.getByRole('button', { name: 'graph' }).click()
  await page.waitForTimeout(5000)
  const fs = await import('node:fs')
  const html = await page.content()
  fs.writeFileSync('/tmp/dbg-sigma.txt', `HAS-CANVAS: ${html.includes('wiki-graph-canvas')}\nHAS-EMPTY: ${html.includes('图谱为空')}\nMAIN: ${(await page.locator('main').innerText().catch(() => '(none)')).slice(0, 300)}\nLOGS:\n${logs.join('\n')}`)
})

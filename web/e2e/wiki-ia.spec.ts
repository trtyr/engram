/**
 * Wiki Obsidian IA 实测：目录树（folder 层级）+ 阅读 + 图谱视图切换 + 收件箱 + 运维二级。
 * 前置：一次性栈（scripts/e2e-local.sh）。
 */
import { expect, test } from '@playwright/test'

const ADMIN_PW = process.env.E2E_ADMIN_PW
if (!ADMIN_PW) throw new Error('缺 E2E_ADMIN_PW：由启动脚本注入（勿在 spec 里写死密码）')
const BASE = process.env.E2E_BASE ?? 'http://127.0.0.1:19180'

async function api(method: string, path: string, body?: unknown, token?: string) {
  const headers: Record<string, string> = { 'content-type': 'application/json' }
  if (token) headers.authorization = `Bearer ${token}`
  const r = await fetch(`${BASE}${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  const text = await r.text()
  const json = text ? JSON.parse(text) : undefined
  if (!r.ok) throw new Error(`${method} ${path} -> ${r.status}: ${text.slice(0, 200)}`)
  return json
}

test('Wiki Obsidian IA：目录树 + 阅读 + 图谱 + 收件箱 + 运维', async ({ page }) => {
  test.setTimeout(300_000)

  // 管理员登录 + 造 3 个不同 folder 的页面（PUT 人工页，无需 LLM）
  const login = await api('POST', '/auth/login', { password: ADMIN_PW })
  const adminToken: string = login.token
  await api('PUT', '/wiki/pages/rust-async', { title: 'Rust 异步', content: '# Rust 异步\n\nTokio 是异步运行时。', folder: '技术/Rust' }, adminToken)
  await api('PUT', '/wiki/pages/network', { title: '网络', content: '# 网络\n\nHTTP 基础。', folder: '技术' }, adminToken)
  await api('PUT', '/wiki/pages/root-page', { title: '根页面', content: '# 根页面\n\n无文件夹。' }, adminToken)

  await page.goto('/')
  await page.getByLabel('管理员密码').fill(ADMIN_PW)
  await page.getByRole('button', { name: '登录' }).click()
  await expect(page.locator('aside nav a[href="/memory"]')).toBeVisible({ timeout: 10_000 })

  await page.locator('aside nav a[href="/wiki"]').click()

  // 目录树：多级 folder（技术 → Rust）+ 根页面
  await expect(page.getByRole('button', { name: '技术', exact: true })).toBeVisible({ timeout: 10_000 })
  await expect(page.getByRole('button', { name: 'Rust', exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Rust 异步', exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: '网络', exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: '根页面', exact: true })).toBeVisible()

  // 点页面节点 → 阅读区加载 Markdown
  await page.getByRole('button', { name: 'Rust 异步', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Rust 异步', level: 2 })).toBeVisible()
  await expect(page.getByText('Tokio 是异步运行时。')).toBeVisible()

  // 树 → 图视图切换 → canvas 渲染 → 切回树
  await page.getByRole('button', { name: '图谱' }).click()
  await expect(page.getByTestId('wiki-graph-canvas'), 'sigma 图谱应渲染').toBeVisible({ timeout: 30_000 })
  await page.getByRole('button', { name: '目录' }).click()
  await expect(page.getByRole('button', { name: '技术', exact: true })).toBeVisible()

  // 收件箱入口 → dropzone → 返回
  await page.getByRole('button', { name: '收件箱' }).click()
  await expect(page.getByTestId('dropzone'), '文档上传区应存在').toBeVisible()
  await page.getByRole('button', { name: '← 返回 Wiki' }).click()

  // 运维二级入口 → 5 子项
  await page.getByRole('button', { name: '运维' }).click()
  await expect(page.getByRole('button', { name: '洞察' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Lint' })).toBeVisible()
  await expect(page.getByRole('button', { name: '提案' })).toBeVisible()
  await expect(page.getByRole('button', { name: '原料' })).toBeVisible()
  await expect(page.getByRole('button', { name: '目标' })).toBeVisible()
})

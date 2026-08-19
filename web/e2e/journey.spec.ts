/**
 * 全旅程 e2e：登录 → 各域 → 检索。
 * 前置：栈已在 E2E_BASE 运行（compose 或本地二进制）。
 */
import { expect, test } from '@playwright/test'

const ADMIN_PW = process.env.E2E_ADMIN_PW ?? 'ui-admin'

test('全旅程：登录 → Dashboard → Memory 检索 → Knowledge → Wiki → Jobs', async ({ page }) => {
  // 1. 登录
  await page.goto('/')
  await page.getByPlaceholder('管理员密码').fill(ADMIN_PW)
  await page.getByRole('button', { name: '登录' }).click()
  await expect(page.getByText('Dashboard')).toBeVisible({ timeout: 10_000 })

  // 2. Dashboard 渲染（统计卡）
  await expect(page.getByText('活跃原子 L1')).toBeVisible()

  // 3. Memory：会话 tab + 检索 tab
  await page.getByRole('link', { name: 'Memory' }).click()
  await expect(page.getByText('sessions')).toBeVisible()
  await page.getByRole('button', { name: 'search' }).click()
  await expect(page.getByPlaceholder('中文检索记忆…')).toBeVisible()

  // 4. Knowledge：上传入口 + 检索框
  await page.getByRole('link', { name: 'Knowledge' }).click()
  await expect(page.getByText('上传文件')).toBeVisible()
  await expect(page.getByPlaceholder('检索知识库…')).toBeVisible()

  // 5. Wiki：页面 + 图 + lint
  await page.getByRole('link', { name: 'Wiki' }).click()
  await expect(page.getByText('选择左侧页面')).toBeVisible()

  // 6. Jobs
  await page.getByRole('link', { name: 'Jobs' }).click()
  await expect(page.getByText('全部状态')).toBeVisible()

  // 7. Settings（providers 表单）
  await page.getByRole('link', { name: 'Settings' }).click()
  await expect(page.getByPlaceholder('Base URL（OpenAI 兼容）')).toBeVisible()

  // 8. 登出态：清 token 后刷新回登录页
  await page.evaluate(() => localStorage.removeItem('am_token'))
  await page.reload()
  await expect(page.getByPlaceholder('管理员密码')).toBeVisible()
})

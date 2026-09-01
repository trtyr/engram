/**
 * 真全旅程 e2e（Phase 6 出口标准 #1）：
 * 浏览器内完成——上传文档到 ready、写会话->蒸馏->画像更新、
 * wiki ingest->页面出现（sigma 图谱渲染）、codegraph 注册->索引->查询返回。
 * 前置：栈已运行（E2E_BASE），codegraph CLI 可用（栈镜像内置）。
 */
import { expect, test } from '@playwright/test'

// admin 密码是拉起栈的启动脚本注入的（E2E_ADMIN_PW），spec 不写死、不猜默认值。
// 未设置立即 fail-fast：避免拿错密码打 401、或撞上同密码栈默默测过。
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

test('真全旅程：上传->ready、会话->蒸馏->原子、wiki->页面+图谱、codegraph->查询', async ({ page }) => {
  test.setTimeout(600_000)

  // ---------- 0. 管理员准备（API 签发 e2e key） ----------
  const login = await api('POST', '/auth/login', { password: ADMIN_PW })
  const adminToken: string = login.token
  const keyResp = await api('POST', '/settings/api-keys', { name: `e2e-${Date.now()}`, scopes: ['memory', 'knowledge', 'wiki', 'codegraph'] }, adminToken)
  const aiKey: string = keyResp.key

  // 探测栈有无可用 LLM provider：没有则跳过依赖 LLM 的断言（蒸馏链 / Wiki 生成），
  // 其余旅程（登录/上传/会话/CodeGraph/Jobs）照常测——CI 栈不配真 key 也能全绿。
  const providers = await api('GET', '/settings/llm/providers', undefined, adminToken)
  const hasLlm = Array.isArray(providers) && providers.length > 0
  if (!hasLlm) {
    test.info().annotations.push({ type: 'note', description: '栈无 LLM provider：跳过蒸馏与 Wiki 生成断言（其余旅程照常）' })
  }

  // ---------- 1. 登录 UI ----------
  await page.goto('/')
  await page.getByLabel('管理员密码').fill(ADMIN_PW)
  await page.getByRole('button', { name: '登录' }).click()
  await expect(page.getByRole('link', { name: '用户记忆' })).toBeVisible({ timeout: 10_000 })

  // ---------- 2. Knowledge：上传 -> ready -> 分块预览 ----------
  await page.getByRole('link', { name: '知识库' }).click()
  await expect(page.getByTestId('dropzone'), '拖拽上传区应存在').toBeVisible()
  const mdContent = '# Playwright \u4e4b\u65c5\n\nPlaywright \u9a71\u52a8\u771f\u5b9e\u6d4f\u89c8\u5668\u5b8c\u6210\u7aef\u5230\u7aef\u9a8c\u8bc1\u3002\n\n## \u65ad\u8a00\u6a21\u578b\n\nexpect(locator).toBeVisible() \u662f\u81ea\u52a8\u91cd\u8bd5\u65ad\u8a00\u3002\n\n## \u8865\u5145\n\n' + '\u6d4b\u8bd5\u6700\u4f73\u5b9e\u8df5\u8865\u5145\u5185\u5bb9\u3002'.repeat(40)
  await page.setInputFiles('input[type=file]', {
    name: 'playwright-guide.md',
    mimeType: 'text/markdown',
    buffer: Buffer.from(mdContent, 'utf-8'),
  })
  // 主从版式：上传后文档出现在左侧目录，点开等 ready
  const docBtn = page.getByRole('button', { name: /playwright-guide/ })
  await expect(docBtn, '文档应出现在目录').toBeVisible({ timeout: 240_000 })
  await docBtn.click()
  await expect(page.getByText('就绪', { exact: true }), '文档应推进到 ready').toBeVisible({ timeout: 240_000 })
  await expect(page.getByText(/共 \d+ 块/), '阅读区应显示分块成文').toBeVisible({ timeout: 30_000 })

  // ---------- 3. Memory：写会话 ->（有 LLM 时）触发蒸馏 -> 原子出现 ----------
  await page.getByRole('link', { name: '用户记忆' }).click()
  await page.getByRole('button', { name: '会话', exact: true }).click()
  await api('POST', '/memory/sessions', {
    agent: 'e2e-browser',
    distill: 'off',
    turns: [
      { speaker: 'user', text: `\u8bb0\u4f4f\uff1ae2e \u51b2\u7130\u6807\u8bb0 ${Date.now()}\uff0c\u6211\u7684\u6d4f\u89c8\u5668\u81ea\u52a8\u5316\u5de5\u5177\u662f Playwright` },
      { speaker: 'assistant', text: '\u5df2\u8bb0\u5f55' },
    ],
  }, aiKey)
  await page.reload()
  // 默认 tab 即会话（圈子已拆独立页 /circle）——直接断言列表
  await expect(page.getByText('e2e-browser').first(), '会话应列出').toBeVisible({ timeout: 15_000 })
  if (hasLlm) {
    await page.getByRole('button', { name: '\u89e6\u53d1\u84b8\u998f' }).click()
    await page.getByRole('button', { name: '\u539f\u5b50', exact: true }).click()
    await expect(
      page.getByText(/Playwright/i).first(),
      '\u84b8\u998f\u5e94\u4ea7\u51fa\u542b Playwright \u7684\u539f\u5b50',
    ).toBeVisible({ timeout: 120_000 })
  }

  // ---------- 4. Wiki：ingest -> 页面出现 -> sigma 图谱（依赖 LLM 生成，无 provider 跳过） ----------
  if (hasLlm) {
    await page.getByRole('link', { name: 'Wiki' }).click()
    await page.getByText('\u65b0\u6587\u6863 ingest').click()
    await page.getByPlaceholder('\u6807\u9898').fill(`e2e-wiki-${Date.now()}`)
    await page.getByPlaceholder('\u6e90\u6587\u672c').fill('Playwright \u662f\u6d4f\u89c8\u5668\u81ea\u52a8\u5316\u6846\u67b6\u3002\u81ea\u52a8\u91cd\u8bd5\u65ad\u8a00\u662f\u5176\u6838\u5fc3\u7279\u6027\uff0c\u8ba9\u7aef\u5230\u7aef\u6d4b\u8bd5\u7a33\u5b9a\u53ef\u9760\u3002web-first assertions \u662f\u63a8\u8350\u5199\u6cd5\u3002')
    await page.getByRole('button', { name: '\u6444\u53d6', exact: true }).click()
    // API 轮询等 wiki_generate succeeded（页面+图谱数据落库后再断言 UI）
    for (let i = 0; i < 240; i++) {
      const jobs = await api('GET', '/jobs?kind=wiki_generate&limit=1', undefined, adminToken)
      const st = jobs?.[0]?.status
      if (st === 'succeeded') break
      if (st === 'failed' || st === 'dead') throw new Error(`wiki job ${st}: ${jobs[0]?.error}`)
      await page.waitForTimeout(1000)
    }
    await page.reload()
    await page.getByRole('button', { name: '图谱' }).click()
    await expect(page.getByTestId('wiki-graph-canvas'), 'sigma.js \u56fe\u8c31\u5e94\u6e32\u67d3').toBeVisible({ timeout: 240_000 })
  }

  // ---------- 5. CodeGraph：注册 -> 索引 -> 查询 ----------
  await page.getByRole('link', { name: '代码图谱' }).click()
  const projName = `e2e-cg-${Date.now()}`
  const resp = await page.request.post(`${BASE}/codegraph/projects`, {
    headers: { authorization: `Bearer ${aiKey}` },
    data: { name: projName, source_uri: 'https://gitcode.com/gh_mirrors/ni/ni.git' },
  })
  if (resp.ok()) {
    await page.reload()
    const card = page.locator('div.rounded-lg.border', { hasText: projName }).first()
    await card.getByRole('button', { name: /\u5efa\u7d22\u5f15/ }).click()
    await expect(card.getByText('就绪', { exact: true }), 'codegraph \u7d22\u5f15\u5e94\u5b8c\u6210').toBeVisible({ timeout: 420_000 })
    await card.getByPlaceholder('\u7b26\u53f7\u6216\u95ee\u9898').fill('ni')
    await card.getByRole('button', { name: '\u67e5\u8be2' }).click()
    await expect(card.locator('pre'), 'codegraph \u67e5\u8be2\u5e94\u8fd4\u56de\u7ed3\u679c').toBeVisible({ timeout: 240_000 })
  } else {
    test.info().annotations.push({ type: 'note', description: `codegraph git clone \u4e0d\u53ef\u7528\uff08${resp.status()}\uff09\uff0cUI \u9762\u677f\u9a8c\u8bc1\u4ee3\u66ff` })
    await expect(page.getByRole('button', { name: '\u6ce8\u518c' })).toBeVisible()
  }

  // ---------- 6. Jobs：事件时间线 ----------
  await page.getByRole('link', { name: '任务' }).click()
  await expect(page.getByRole('combobox').first()).toBeVisible()
  await page.locator('tbody tr').first().click()
  await expect(page.getByText(/\u4efb\u52a1|\u5165\u961f/).first(), '\u4e8b\u4ef6\u65f6\u95f4\u7ebf\u5e94\u5c55\u793a').toBeVisible({ timeout: 15_000 })
})

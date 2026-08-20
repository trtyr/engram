# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: journey.spec.ts >> 真全旅程：上传->ready、会话->蒸馏->原子、wiki->页面+图谱、codegraph->查询
- Location: e2e/journey.spec.ts:26:1

# Error details

```
Error: 拖拽上传区应存在

expect(locator).toBeVisible() failed

Locator: getByTestId('dropzone')
Expected: visible
Timeout: 5000ms
Error: element(s) not found

Call log:
  - 拖拽上传区应存在 with timeout 5000ms
  - waiting for getByTestId('dropzone')

```

```yaml
- complementary:
  - heading "agent-memory" [level=1]
  - link "Dashboard":
    - /url: /
  - link "Memory":
    - /url: /memory
  - link "Knowledge":
    - /url: /knowledge
  - link "Wiki":
    - /url: /wiki
  - link "CodeGraph":
    - /url: /codegraph
  - link "Jobs":
    - /url: /jobs
  - link "Settings":
    - /url: /settings
- main:
  - heading "Knowledge" [level=1]
  - button "上传文件"
  - textbox "https://…"
  - button "摄取 URL"
  - textbox "检索知识库…"
  - button "检索"
  - table:
    - rowgroup:
      - row "标题 状态 时间 错误":
        - columnheader "标题"
        - columnheader "状态"
        - columnheader "时间"
        - columnheader "错误"
        - columnheader
    - rowgroup:
      - row "清华大学开源软件镜像站 | Tsinghua Open Source Mirror ready 2026/8/20 20:21:38 分块 删除":
        - cell "清华大学开源软件镜像站 | Tsinghua Open Source Mirror"
        - cell "ready"
        - cell "2026/8/20 20:21:38"
        - cell
        - cell "分块 删除":
          - button "分块"
          - button "删除"
      - 'row "https://hub.docker.com/v2/repositories/library/postgres/ failed 2026/8/20 20:18:18 URL 抓取失败: 抓取失败：error sending request for url (https://hub.docker.com/v2/repositories/library/postgres/) 分块 删除"':
        - cell "https://hub.docker.com/v2/repositories/library/postgres/"
        - cell "failed"
        - cell "2026/8/20 20:18:18"
        - 'cell "URL 抓取失败: 抓取失败：error sending request for url (https://hub.docker.com/v2/repositories/library/postgres/)"'
        - cell "分块 删除":
          - button "分块"
          - button "删除"
```

# Test source

```ts
  1   | /**
  2   |  * 真全旅程 e2e（Phase 6 出口标准 #1）：
  3   |  * 浏览器内完成——上传文档到 ready、写会话->蒸馏->画像更新、
  4   |  * wiki ingest->页面出现（sigma 图谱渲染）、codegraph 注册->索引->查询返回。
  5   |  * 前置：栈已运行（E2E_BASE），codegraph CLI 可用（栈镜像内置）。
  6   |  */
  7   | import { expect, test } from '@playwright/test'
  8   | 
  9   | const ADMIN_PW = process.env.E2E_ADMIN_PW ?? 'release-admin'
  10  | const BASE = process.env.E2E_BASE ?? 'http://127.0.0.1:19180'
  11  | 
  12  | async function api(method: string, path: string, body?: unknown, token?: string) {
  13  |   const headers: Record<string, string> = { 'content-type': 'application/json' }
  14  |   if (token) headers.authorization = `Bearer ${token}`
  15  |   const r = await fetch(`${BASE}${path}`, {
  16  |     method,
  17  |     headers,
  18  |     body: body === undefined ? undefined : JSON.stringify(body),
  19  |   })
  20  |   const text = await r.text()
  21  |   const json = text ? JSON.parse(text) : undefined
  22  |   if (!r.ok) throw new Error(`${method} ${path} -> ${r.status}: ${text.slice(0, 200)}`)
  23  |   return json
  24  | }
  25  | 
  26  | test('真全旅程：上传->ready、会话->蒸馏->原子、wiki->页面+图谱、codegraph->查询', async ({ page }) => {
  27  |   test.setTimeout(600_000)
  28  | 
  29  |   // ---------- 0. 管理员准备（API 签发 e2e key） ----------
  30  |   const login = await api('POST', '/auth/login', { password: ADMIN_PW })
  31  |   const adminToken: string = login.token
  32  |   const keyResp = await api('POST', '/settings/api-keys', { name: `e2e-${Date.now()}`, scopes: ['memory', 'knowledge', 'wiki', 'codegraph'] }, adminToken)
  33  |   const aiKey: string = keyResp.key
  34  | 
  35  |   // ---------- 1. 登录 UI ----------
  36  |   await page.goto('/')
  37  |   await page.getByPlaceholder('管理员密码').fill(ADMIN_PW)
  38  |   await page.getByRole('button', { name: '登录' }).click()
  39  |   await expect(page.getByRole('link', { name: 'Memory' })).toBeVisible({ timeout: 10_000 })
  40  | 
  41  |   // ---------- 2. Knowledge：上传 -> ready -> 分块预览 ----------
  42  |   await page.getByRole('link', { name: 'Knowledge' }).click()
> 43  |   await expect(page.getByTestId('dropzone'), '拖拽上传区应存在').toBeVisible()
      |                                                          ^ Error: 拖拽上传区应存在
  44  |   const mdContent = '# Playwright \u4e4b\u65c5\n\nPlaywright \u9a71\u52a8\u771f\u5b9e\u6d4f\u89c8\u5668\u5b8c\u6210\u7aef\u5230\u7aef\u9a8c\u8bc1\u3002\n\n## \u65ad\u8a00\u6a21\u578b\n\nexpect(locator).toBeVisible() \u662f\u81ea\u52a8\u91cd\u8bd5\u65ad\u8a00\u3002\n\n## \u8865\u5145\n\n' + '\u6d4b\u8bd5\u6700\u4f73\u5b9e\u8df5\u8865\u5145\u5185\u5bb9\u3002'.repeat(40)
  45  |   await page.setInputFiles('input[type=file]', {
  46  |     name: 'playwright-guide.md',
  47  |     mimeType: 'text/markdown',
  48  |     buffer: Buffer.from(mdContent, 'utf-8'),
  49  |   })
  50  |   await expect(
  51  |     page.getByRole('row', { name: /playwright-guide/ }).getByText('ready', { exact: true }),
  52  |     '\u6587\u6863\u5e94\u63a8\u8fdb\u5230 ready',
  53  |   ).toBeVisible({ timeout: 90_000 })
  54  |   await page.getByRole('row', { name: /playwright-guide/ }).getByRole('button', { name: '\u5206\u5757' }).click()
  55  |   await expect(page.getByText(/#\d+/).first(), '\u5206\u5757\u9884\u89c8\u5e94\u5c55\u793a').toBeVisible({ timeout: 30_000 })
  56  | 
  57  |   // ---------- 3. Memory：写会话 -> 触发蒸馏 -> 原子出现 ----------
  58  |   await page.getByRole('link', { name: 'Memory' }).click()
  59  |   await page.getByRole('button', { name: 'sessions' }).click()
  60  |   await api('POST', '/memory/sessions', {
  61  |     agent: 'e2e-browser',
  62  |     distill: 'off',
  63  |     turns: [
  64  |       { speaker: 'user', text: `\u8bb0\u4f4f\uff1ae2e \u51b2\u7130\u6807\u8bb0 ${Date.now()}\uff0c\u6211\u7684\u6d4f\u89c8\u5668\u81ea\u52a8\u5316\u5de5\u5177\u662f Playwright` },
  65  |       { speaker: 'assistant', text: '\u5df2\u8bb0\u5f55' },
  66  |     ],
  67  |   }, aiKey)
  68  |   await page.reload()
  69  |   await expect(page.getByText('e2e-browser'), '\u4f1a\u8bdd\u5e94\u5217\u51fa').toBeVisible({ timeout: 15_000 })
  70  |   await page.getByRole('button', { name: '\u89e6\u53d1\u84b8\u998f' }).click()
  71  |   await page.getByRole('button', { name: 'atoms' }).click()
  72  |   await expect(
  73  |     page.getByText(/Playwright/i).first(),
  74  |     '\u84b8\u998f\u5e94\u4ea7\u51fa\u542b Playwright \u7684\u539f\u5b50',
  75  |   ).toBeVisible({ timeout: 120_000 })
  76  | 
  77  |   // ---------- 4. Wiki：ingest -> 页面出现 -> sigma 图谱 ----------
  78  |   await page.getByRole('link', { name: 'Wiki' }).click()
  79  |   await page.getByText('\u65b0\u6587\u6863 ingest').click()
  80  |   await page.getByPlaceholder('\u6807\u9898').fill(`e2e-wiki-${Date.now()}`)
  81  |   await page.getByPlaceholder('\u6e90\u6587\u672c').fill('Playwright \u662f\u6d4f\u89c8\u5668\u81ea\u52a8\u5316\u6846\u67b6\u3002\u81ea\u52a8\u91cd\u8bd5\u65ad\u8a00\u662f\u5176\u6838\u5fc3\u7279\u6027\uff0c\u8ba9\u7aef\u5230\u7aef\u6d4b\u8bd5\u7a33\u5b9a\u53ef\u9760\u3002web-first assertions \u662f\u63a8\u8350\u5199\u6cd5\u3002')
  82  |   await page.getByRole('button', { name: '\u6444\u53d6', exact: true }).click()
  83  |   await expect(
  84  |     page.getByText(/Playwright/i).first(),
  85  |     'wiki ingest \u5e94\u4ea7\u51fa\u9875\u9762',
  86  |   ).toBeVisible({ timeout: 180_000 })
  87  |   await page.getByRole('button', { name: 'graph' }).click()
  88  |   await expect(page.getByTestId('wiki-graph-canvas'), 'sigma.js \u56fe\u8c31\u5e94\u6e32\u67d3').toBeVisible({ timeout: 30_000 })
  89  | 
  90  |   // ---------- 5. CodeGraph：注册 -> 索引 -> 查询 ----------
  91  |   await page.getByRole('link', { name: 'CodeGraph' }).click()
  92  |   const projName = `e2e-cg-${Date.now()}`
  93  |   const resp = await page.request.post(`${BASE}/codegraph/projects`, {
  94  |     headers: { authorization: `Bearer ${aiKey}` },
  95  |     data: { name: projName, source_uri: 'https://gitcode.com/gh_mirrors/ni/ni.git' },
  96  |   })
  97  |   if (resp.ok()) {
  98  |     await page.reload()
  99  |     const card = page.locator('div.rounded-lg.border', { hasText: projName }).first()
  100 |     await card.getByRole('button', { name: /\u5efa\u7d22\u5f15/ }).click()
  101 |     await expect(card.getByText('ready', { exact: true }), 'codegraph \u7d22\u5f15\u5e94\u5b8c\u6210').toBeVisible({ timeout: 420_000 })
  102 |     await card.getByPlaceholder('\u7b26\u53f7\u6216\u95ee\u9898').fill('ni')
  103 |     await card.getByRole('button', { name: '\u67e5\u8be2' }).click()
  104 |     await expect(card.locator('pre'), 'codegraph \u67e5\u8be2\u5e94\u8fd4\u56de\u7ed3\u679c').toBeVisible({ timeout: 90_000 })
  105 |   } else {
  106 |     test.info().annotations.push({ type: 'note', description: `codegraph git clone \u4e0d\u53ef\u7528\uff08${resp.status()}\uff09\uff0cUI \u9762\u677f\u9a8c\u8bc1\u4ee3\u66ff` })
  107 |     await expect(page.getByRole('button', { name: '\u6ce8\u518c' })).toBeVisible()
  108 |   }
  109 | 
  110 |   // ---------- 6. Jobs：事件时间线 ----------
  111 |   await page.getByRole('link', { name: 'Jobs' }).click()
  112 |   await expect(page.getByRole('combobox')).toBeVisible()
  113 |   await page.locator('tbody tr').first().click()
  114 |   await expect(page.getByText(/\u4efb\u52a1|\u5165\u961f/).first(), '\u4e8b\u4ef6\u65f6\u95f4\u7ebf\u5e94\u5c55\u793a').toBeVisible({ timeout: 15_000 })
  115 | })
  116 | 
```
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

  // 目录树：多级 folder（技术 → Rust）+ 根页面（WAI-ARIA tree 语义：role=treeitem）
  await expect(page.getByRole('treeitem', { name: '技术', exact: true })).toBeVisible({ timeout: 10_000 })
  await expect(page.getByRole('treeitem', { name: 'Rust', exact: true })).toBeVisible()
  await expect(page.getByRole('treeitem', { name: 'Rust 异步', exact: true })).toBeVisible()
  await expect(page.getByRole('treeitem', { name: '网络', exact: true })).toBeVisible()
  await expect(page.getByRole('treeitem', { name: '根页面', exact: true })).toBeVisible()

  // 折叠「技术」→ 子树消失 → 刷新后仍折叠（localStorage 持久化）。
  // 注意：必须在选中页面之前做——一旦 URL 带 ?page=，刷新会触发深链自动展开定位，覆盖折叠态（预期行为）。
  const techBtn = page.getByRole('treeitem', { name: '技术', exact: true })
  await techBtn.click()
  await expect(techBtn).toHaveAttribute('aria-expanded', 'false')
  await expect(page.getByRole('treeitem', { name: 'Rust 异步', exact: true })).toHaveCount(0)
  await page.reload()
  const techBtn2 = page.getByRole('treeitem', { name: '技术', exact: true })
  await expect(techBtn2).toBeVisible({ timeout: 10_000 })
  await expect(techBtn2).toHaveAttribute('aria-expanded', 'false')
  await techBtn2.click() // 恢复展开
  await expect(page.getByRole('treeitem', { name: 'Rust 异步', exact: true })).toBeVisible()

  // 点页面节点 → 阅读区加载 Markdown + URL 写回（可刷新/分享）
  await page.getByRole('treeitem', { name: 'Rust 异步', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Rust 异步', level: 2 })).toBeVisible()
  await expect(page.getByText('Tokio 是异步运行时。')).toBeVisible()
  await expect(page).toHaveURL(/page=rust-async/)

  // 树 → 图视图切换 → canvas 渲染 → 切回树
  await page.getByRole('button', { name: '图谱' }).click()
  await expect(page.getByTestId('wiki-graph-canvas'), 'sigma 图谱应渲染').toBeVisible({ timeout: 30_000 })
  await page.getByRole('button', { name: '目录' }).click()
  await expect(page.getByRole('treeitem', { name: '技术', exact: true })).toBeVisible()

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

  // 深链直达：?page= 免点击打开（wikilink / 分享 / 刷新同路径）
  await page.goto('/wiki?page=root-page')
  await expect(page.getByRole('heading', { name: '根页面', level: 2 })).toBeVisible({ timeout: 10_000 })
})

test('Wiki 布局几何：空态撑满 / 底部贴视口 / 中窄双列 / 树宽拖拽持久 / tree 语义', async ({ page }) => {
  test.setTimeout(120_000)

  const login = await api('POST', '/auth/login', { password: ADMIN_PW })
  const adminToken: string = login.token
  await api('PUT', '/wiki/pages/geo-a', { title: '几何A', content: '# 几何A', folder: '几何/嵌套' }, adminToken)
  await api('PUT', '/wiki/pages/geo-root', { title: '几何根页', content: '# 几何根页' }, adminToken)

  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/')
  await page.getByLabel('管理员密码').fill(ADMIN_PW)
  await page.getByRole('button', { name: '登录' }).click()
  await expect(page.locator('aside nav a[href="/memory"]')).toBeVisible({ timeout: 10_000 })
  await page.locator('aside nav a[href="/wiki"]').click()
  await expect(page.getByRole('treeitem', { name: '几何', exact: true })).toBeVisible({ timeout: 10_000 })

  // ① tree 语义（#22）：容器 role=tree，folder/treeitem 带 aria-level，嵌套层级递增。
  // folder 默认全展开（aria-expanded=true）——click 是折叠 toggle。
  const tree = page.getByRole('tree', { name: 'Wiki 页面目录' })
  await expect(tree).toBeVisible()
  const geoFolder = page.getByRole('treeitem', { name: '几何', exact: true })
  await expect(geoFolder).toHaveAttribute('aria-level', '1')
  await expect(geoFolder).toHaveAttribute('aria-expanded', 'true')
  const nestFolder = page.getByRole('treeitem', { name: '嵌套', exact: true })
  await expect(nestFolder).toHaveAttribute('aria-level', '2')
  await expect(page.getByRole('treeitem', { name: '几何A', exact: true })).toHaveAttribute('aria-level', '3')
  await expect(page.getByRole('treeitem', { name: '几何根页', exact: true })).toHaveAttribute('aria-level', '1')
  // 折叠/展开经 treeitem click 生效
  await nestFolder.click()
  await expect(page.getByRole('treeitem', { name: '几何A', exact: true })).toHaveCount(0)
  await nestFolder.click()
  await expect(page.getByRole('treeitem', { name: '几何A', exact: true })).toBeVisible()

  // ② 空态撑满阅读列（#1：未选页时引导区宽 ≥ 600px——旧实现缩成 86px 小条）
  const emptyBox = page.getByTestId('wiki-empty-guide')
  await expect(emptyBox).toBeVisible()
  const emptyW = (await emptyBox.boundingBox())!.width
  expect(emptyW, `空态宽度 ${Math.round(emptyW)}px 应撑满阅读列（≥600）`).toBeGreaterThanOrEqual(600)

  // ③ 底部贴视口（#2：工作区 bottom 距视口底 ≤ 48px——旧实现 150px 死空白）
  const wsBottom = await page.evaluate(() => {
    const ws = document.querySelector('main .lg\\:h-\\[calc\\(100vh-3rem\\)\\]')
    return ws ? ws.getBoundingClientRect().bottom : -1
  })
  expect(wsBottom, `工作区 bottom=${Math.round(wsBottom)} 应贴近视口底（差 ≤48px）`).toBeGreaterThanOrEqual(900 - 48)

  // ④ 双列断点（#3）：lg(1024) 起树/阅读双列同行且树列独立滚动；900px 自然堆叠、无横向溢出
  await page.setViewportSize({ width: 1100, height: 700 })
  const cols = await page.evaluate(() => {
    const ws = document.querySelector('main .lg\\:h-\\[calc\\(100vh-3rem\\)\\] .lg\\:flex-row')
    if (!ws) return null
    const kids = [...ws.children] as HTMLElement[]
    const r = (el: Element) => el.getBoundingClientRect()
    const scroller = ws.querySelector('[class*="overflow-y-auto"]')
    return {
      twoCol: kids.length >= 3 && Math.abs(r(kids[0]).y - r(kids[kids.length - 1]).y) < 4,
      treeScroll: scroller ? getComputedStyle(scroller).overflowY === 'auto' : false,
    }
  })
  expect(cols?.twoCol, '1100px（lg）应保持树/阅读双列同行').toBe(true)
  expect(cols?.treeScroll, '树列应独立滚动（overflow-y auto）').toBe(true)
  await page.setViewportSize({ width: 900, height: 700 })
  const stacked = await page.evaluate(() => ({
    overflowX: document.documentElement.scrollWidth - document.documentElement.clientWidth,
  }))
  expect(stacked.overflowX, '900px 堆叠态不应有横向溢出').toBeLessThanOrEqual(2)

  // ⑤ 树宽拖拽（#6 分割线）：拖到 ~420px → 树列宽生效，reload 后持久化
  await page.setViewportSize({ width: 1440, height: 900 })
  const sep = page.getByRole('separator')
  await expect(sep).toBeVisible()
  const sepBox = (await sep.boundingBox())!
  // 向右拖 +140px（约 280→420）——move 参数是绝对视口坐标
  await page.mouse.move(sepBox.x + sepBox.width / 2, sepBox.y + 300)
  await page.mouse.down()
  await page.mouse.move(sepBox.x + 140, sepBox.y + 300, { steps: 8 })
  await page.mouse.up()
  await page.waitForTimeout(150)
  const treeWAfter = await page.evaluate(() => {
    const el = document.querySelector('main [style*="--tree-w"]')
    return el ? el.getBoundingClientRect().width : -1
  })
  expect(treeWAfter, `拖拽后树宽 ${Math.round(treeWAfter)} 应≈420`).toBeGreaterThanOrEqual(405)
  expect(treeWAfter).toBeLessThanOrEqual(435)
  await page.reload()
  await expect(tree).toBeVisible({ timeout: 10_000 })
  const treeWReload = await page.evaluate(() => {
    const el = document.querySelector('main [style*="--tree-w"]')
    return el ? el.getBoundingClientRect().width : -1
  })
  expect(Math.abs(treeWReload - treeWAfter), 'reload 后树宽应持久化（±6px）').toBeLessThanOrEqual(6)
})

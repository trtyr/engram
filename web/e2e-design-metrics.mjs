/**
 * 计算样式审计：对每个页面提取可量化的设计指标（字体/间距/颜色/对比度/圆角/布局）。
 * 输出 docs/design/metrics.json + 控制台摘要。
 */
import { chromium } from '@playwright/test'
import fs from 'node:fs'

const BASE = process.env.E2E_BASE ?? 'http://127.0.0.1:19180'
const PW = process.env.E2E_ADMIN_PW ?? 'design-audit-pw'

const pages = [
  { name: 'dashboard', path: '/' },
  { name: 'memory', path: '/memory' },
  { name: 'wiki', path: '/wiki' },
  { name: 'wiki-markdown', path: '/wiki?page=markdown-kitchen-sink' },
  { name: 'codegraph', path: '/codegraph' },
  { name: 'settings', path: '/settings' },
]

const EXTRACT = () => {
  const lum = (r, g, b) => {
    const f = (c) => { c /= 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4 }
    return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
  }
  const contrast = (fg, bg) => {
    const l1 = lum(...fg), l2 = lum(...bg)
    const [a, b] = l1 > l2 ? [l1, l2] : [l2, l1]
    return (a + 0.05) / (b + 0.05)
  }
  const parse = (s) => {
    const m = s.match(/rgba?\(([\d.]+),\s*([\d.]+),\s*([\d.]+)(?:,\s*([\d.]+))?\)/)
    return m ? [+m[1], +m[2], +m[3], m[4] === undefined ? 1 : +m[4]] : null
  }
  const out = { fonts: {}, sizes: {}, weights: {}, radii: {}, colors: {}, spacings: {}, contrasts: [], layout: {} }
  const els = [...document.querySelectorAll('body *')]
  const visible = els.filter((e) => {
    const r = e.getBoundingClientRect()
    return r.width > 0 && r.height > 0
  })
  for (const e of visible) {
    const cs = getComputedStyle(e)
    if (e.textContent?.trim() && !e.children.length) {
      const key = `${cs.fontSize}/${cs.fontWeight}/${cs.fontFamily.split(',')[0].trim()}`
      out.sizes[key] = (out.sizes[key] ?? 0) + 1
      // 对比度：向上找不透明背景
      if (cs.color && e.textContent.trim().length > 1) {
        let bg = e.parentElement, bgc = null
        while (bg && !bgc) { const c = parse(getComputedStyle(bg).backgroundColor); if (c && c[3] > 0.9) bgc = c; else bg = bg.parentElement }
        const fg = parse(cs.color)
        if (fg && bgc && out.contrasts.length < 400) {
          out.contrasts.push({
            text: e.textContent.trim().slice(0, 24),
            tag: e.tagName, cls: (e.className + '').slice(0, 60),
            size: cs.fontSize, ratio: +contrast(fg.slice(0, 3), bgc.slice(0, 3)).toFixed(2),
          })
        }
      }
    }
    const r = e.getBoundingClientRect()
    if (cs.borderRadius && cs.borderRadius !== '0px') out.radii[cs.borderRadius] = (out.radii[cs.borderRadius] ?? 0) + 1
    const c = parse(cs.backgroundColor)
    if (c && c[3] > 0.05) { const k = cs.backgroundColor; out.colors[k] = (out.colors[k] ?? 0) + 1 }
    // 主容器的 padding/gap
    if (r.width > 600 && r.height > 200) {
      const k = `pad:${cs.padding} gap:${cs.gap || '0px'}`
      out.spacings[k] = (out.spacings[k] ?? 0) + 1
    }
  }
  out.layout.viewport = { w: innerWidth, h: innerHeight }
  out.layout.docHeight = document.documentElement.scrollHeight
  out.layout.bodyFont = getComputedStyle(document.body).fontFamily
  return out
}

const browser = await chromium.launch()
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
// 登录
await page.goto(`${BASE}/login`)
await page.getByLabel('管理员密码').fill(PW)
await page.getByRole('button', { name: '登录' }).click()
await page.waitForURL(`${BASE}/`, { timeout: 10_000 })
await page.waitForTimeout(800)

const result = {}
for (const pg of pages) {
  if (pg.nav) await pg.nav(page)
  else await page.goto(`${BASE}${pg.path}`)
  await page.waitForTimeout(1500)
  result[pg.name] = await page.evaluate(EXTRACT)
  fs.writeFileSync('../docs/design/metrics.json', JSON.stringify(result, null, 1))
  console.log('extracted:', pg.name)
}
// jobs 页要走客户端导航（路由与 API 冲突）
try {
  await page.click('aside nav a[href="/jobs"]', { timeout: 8000 })
  await page.waitForTimeout(1500)
  result['jobs'] = await page.evaluate(EXTRACT)
  fs.writeFileSync('../docs/design/metrics.json', JSON.stringify(result, null, 1))
  console.log('extracted: jobs')
} catch (e) { console.log('jobs skip:', e.message.slice(0, 60)) }
// login 最后采（清 storage）
await page.evaluate(() => localStorage.clear())
await page.goto(`${BASE}/login`)
await page.waitForTimeout(800)
result['login'] = await page.evaluate(EXTRACT)
fs.writeFileSync('../docs/design/metrics.json', JSON.stringify(result, null, 1))
console.log('extracted: login')

fs.writeFileSync('../docs/design/metrics.json', JSON.stringify(result, null, 1))
console.log('done → docs/design/metrics.json')

// 摘要：低对比度 top10 + 字号/圆角分布
for (const [name, d] of Object.entries(result)) {
  const low = d.contrasts.filter((c) => c.ratio < 4.5).sort((a, b) => a.ratio - b.ratio).slice(0, 5)
  console.log(`\n== ${name} ==`)
  console.log('  低对比(<4.5):', low.map((c) => `${c.ratio}@${c.size}「${c.text.slice(0, 10)}」`).join(' | ') || '无')
  console.log('  字号分布:', Object.entries(d.sizes).sort((a, b) => b[1] - a[1]).slice(0, 5).map(([k, n]) => `${k}×${n}`).join(' , '))
  console.log('  圆角分布:', Object.entries(d.radii).sort((a, b) => b[1] - a[1]).slice(0, 4).map(([k, n]) => `${k}×${n}`).join(' , '))
}
await browser.close()

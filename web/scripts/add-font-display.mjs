// 构建后处理：给产物 CSS 里所有 @font-face 注入 font-display: swap。
// 根因（2026-09-27 Lighthouse）：lxgw-wenkai-webfont 包的 @font-face 未声明
// font-display（默认 block 行为）——16 个中文字体子集全部加载完成前文字不绘制，
// 真实首绘 11.7s。face 描述符无法在用户 CSS 覆盖，故在 dist 产物上正则注入。
import { readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const dir = join(process.cwd(), 'dist', 'static')
let patched = 0
for (const f of readdirSync(dir).filter((f) => f.endsWith('.css'))) {
  const p = join(dir, f)
  const s = readFileSync(p, 'utf8')
  if (!s.includes('@font-face')) continue
  const out = s.replaceAll('@font-face{', '@font-face{font-display:swap;')
  writeFileSync(p, out)
  patched += (s.match(/@font-face\{/g) ?? []).length
  console.log(`font-display:swap → ${f}`)
}
console.log(`patched ${patched} @font-face rules`)

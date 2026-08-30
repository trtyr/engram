/** 非组件 UI 常量与工具。独立于组件文件，保证 fast-refresh 只对组件生效。 */

/** ISO 时间 → 本地可读（中文、24 小时制）。 */
export function fmtTime(iso: string): string {
  return new Date(iso).toLocaleString('zh-CN', { hour12: false })
}

/** 统一表格样式：发丝线 + 等宽数字。 */
export const tableCls = {
  root: 'w-full text-sm',
  thead: 'border-b border-border text-left',
  th: 'px-3 py-2 text-xs font-medium text-muted-foreground',
  row: 'border-b border-border/60 transition-colors last:border-b-0 hover:bg-muted/40',
  td: 'px-3 py-2 align-top',
  tdMono: 'px-3 py-2 align-top font-mono text-xs',
}

/** 统一输入框 / 下拉样式。 */
export const inputCls =
  'rounded-md border border-input bg-card px-3 py-1.5 text-sm outline-none transition-colors placeholder:text-muted-foreground/60 focus-visible:border-foreground/40'
export const selectCls = inputCls

/** 非组件 UI 常量与工具。独立于组件文件，保证 fast-refresh 只对组件生效。 */

/** ISO 时间 → 本地可读（中文、24 小时制）。 */
export function fmtTime(iso: string): string {
  return new Date(iso).toLocaleString('zh-CN', { hour12: false })
}

/** 统一表格样式。 */
export const tableCls = {
  root: 'w-full text-sm',
  thead: 'border-b border-border text-left',
  th: 'px-3 py-2 text-xs font-medium text-muted-foreground',
  row: 'border-b border-border/50 transition-colors hover:bg-muted/30',
  td: 'px-3 py-2.5 align-top',
}

/** 统一输入框 / 下拉样式。 */
export const inputCls =
  'rounded-md border border-input bg-card px-3 py-1.5 text-sm outline-none transition-colors placeholder:text-muted-foreground/60 focus-visible:border-brand/60 focus-visible:ring-2 focus-visible:ring-brand/30'
export const selectCls = inputCls

/** 待办/工单两页共享的展示常量与类型（kind=todo|ticket 同表，展示层各走各的心智）。 */

/** todo 优先级（ticket 用 severity，不用这个） */
export const PRIO_LABEL: Record<string, string> = { low: '低', normal: '普通', high: '高' }
export const PRIO_CLASS: Record<string, string> = {
  high: 'border-destructive/40 text-destructive',
  normal: 'border-border text-muted-foreground',
  low: 'border-border text-muted-foreground/70',
}
/** 待办页勾选行的优先级色点（微软 To Do 式轻量标记） */
export const PRIO_DOT: Record<string, string> = {
  high: 'bg-destructive',
  normal: 'bg-muted-foreground/40',
  low: 'bg-muted-foreground/25',
}

export const SEVERITY_CLASS: Record<string, string> = {
  P0: 'border-destructive bg-destructive/10 text-destructive',
  P1: 'border-warning/60 bg-warning/10 text-warning',
  P2: 'border-border text-muted-foreground',
  P3: 'border-border text-muted-foreground/70',
}

/** 工单状态中文标签 */
export const TICKET_STATUS_LABEL: Record<string, string> = {
  open: '待确认',
  confirmed: '已确认',
  in_progress: '处理中',
  resolved: '已解决',
  verified: '已验证',
  archived: '已归档',
}

/** 工单状态徽章配色 */
export const TICKET_STATUS_CLASS: Record<string, string> = {
  open: 'border-info/40 bg-info/10 text-info',
  confirmed: 'border-primary/40 bg-primary/10 text-primary',
  in_progress: 'border-warning/60 bg-warning/10 text-warning',
  resolved: 'border-success/50 bg-success/10 text-success',
  verified: 'border-success bg-success/15 text-success',
  archived: 'border-border text-muted-foreground',
}

/** 工单状态下一步流转（点按钮推进；resolved 由后端校验 resolution 必填） */
export const TICKET_NEXT: Record<string, { next: string; label: string }> = {
  open: { next: 'confirmed', label: '确认' },
  confirmed: { next: 'in_progress', label: '开始处理' },
  in_progress: { next: 'resolved', label: '标记解决' },
  resolved: { next: 'verified', label: '验证通过' },
}

/** 工单终态集合（行淡化、隐藏流转） */
export const TICKET_DONEISH = ['resolved', 'verified', 'archived']

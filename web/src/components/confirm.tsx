/**
 * 应用内确认弹窗（全站替代浏览器原生 confirm / prompt——2026-09-08 用户明确要求）。
 *
 * 用法两步：
 * 1. App 根部挂一次 <GlobalConfirm />；
 * 2. 任意代码处 `if (!(await appConfirm({ title, description?, destructive?, inputMatch? }))) return`
 *    —— Promise 语义与原生 confirm 一致（true=确认 / false=取消），调用点改造成本最低。
 *
 * 设计（对齐「墨白正统」）：1px 发丝线卡片 + 遮罩，零圆角花活；
 * destructive=true 确认键承担红色语义；inputMatch 是高危操作双因子（输入确认短语才能点确认）。
 * Esc / 点遮罩 = 取消；Enter = 确认（短语模式输对才可用）。
 */

import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

export type ConfirmOptions = {
  title: string
  description?: string
  confirmLabel?: string
  cancelLabel?: string
  /** 破坏性操作：确认按钮红色 */
  destructive?: boolean
  /** 确认短语模式：须精确输入该短语，确认键才可用 */
  inputMatch?: string
}

type Request = { opts: ConfirmOptions; resolve: (v: boolean) => void }

let subscriber: ((r: Request | null) => void) | null = null
let current: Request | null = null

export function appConfirm(opts: ConfirmOptions): Promise<boolean> {
  return new Promise((resolve) => {
    // 理论上单弹窗串行；真重叠时前一个按取消结算，不挂死调用方
    current?.resolve(false)
    current = { opts, resolve }
    subscriber?.(current)
  })
}

function settle(v: boolean) {
  current?.resolve(v)
  current = null
  subscriber?.(null)
}

/** 全站挂一次（App 根部）；无确认请求时渲染 null。 */
export function GlobalConfirm() {
  const [req, setReq] = useState<Request | null>(null)
  const [typed, setTyped] = useState('')
  useEffect(() => {
    subscriber = setReq
    return () => {
      subscriber = null
    }
  }, [])
  useEffect(() => {
    setTyped('')
    if (!req) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') settle(false)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [req])
  if (!req) return null
  const { opts } = req
  const needPhrase = opts.inputMatch != null
  const phraseOk = !needPhrase || typed === opts.inputMatch
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) settle(false)
      }}
    >
      <div
        role="alertdialog"
        aria-modal="true"
        aria-label={opts.title}
        className="w-full max-w-sm rounded-lg border border-border bg-card p-5"
      >
        <h3 className="text-sm font-medium">{opts.title}</h3>
        {opts.description && (
          <p className="mt-1.5 text-sm leading-relaxed text-muted-foreground">{opts.description}</p>
        )}
        {needPhrase && (
          <input
            autoFocus
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && phraseOk) settle(true)
            }}
            placeholder={`输入「${opts.inputMatch}」以确认`}
            className={cn(
              'mt-3 w-full rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/60',
              typed && !phraseOk && 'border-warning',
            )}
          />
        )}
        <div className="mt-4 flex justify-end gap-2">
          <Button
            variant="ghost"
            size="sm"
            autoFocus={!needPhrase}
            onClick={() => settle(false)}
          >
            {opts.cancelLabel ?? '取消'}
          </Button>
          <Button
            variant={opts.destructive ? 'destructive' : 'default'}
            size="sm"
            disabled={!phraseOk}
            onClick={() => settle(true)}
          >
            {opts.confirmLabel ?? '确认'}
          </Button>
        </div>
      </div>
    </div>
  )
}

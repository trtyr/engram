/**
 * 主题控制：系统跟随 + 手动覆盖（engram-theme），index.html 引导脚本防闪烁。
 * 变更广播（engram-theme-change）：sigma/mermaid 等一次性取色的组件订阅后重渲染。
 * 跨标签同步：storage 事件；系统偏好实时跟随：matchMedia change（仅无手动覆盖时）。
 */
import { useCallback, useEffect, useState } from 'react'

export type Theme = 'light' | 'dark'

const THEME_EVENT = 'engram-theme-change'

function current(): Theme {
  return document.documentElement.classList.contains('dark') ? 'dark' : 'light'
}

function apply(t: Theme) {
  document.documentElement.classList.toggle('dark', t === 'dark')
}

function broadcast() {
  window.dispatchEvent(new CustomEvent(THEME_EVENT))
}

/** 订阅主题变化（含跨标签/系统跟随触发的变化）。返回取消函数。 */
export function onThemeChange(cb: () => void): () => void {
  window.addEventListener(THEME_EVENT, cb)
  return () => window.removeEventListener(THEME_EVENT, cb)
}

export function useTheme() {
  const [theme, setThemeState] = useState<Theme>(current)

  // 其他实例/标签页切了主题 → 本实例状态同步
  useEffect(() => onThemeChange(() => setThemeState(current)), [])

  const setTheme = useCallback((t: Theme) => {
    try {
      localStorage.setItem('engram-theme', t)
    } catch {
      /* 私密模式等场景忽略 */
    }
    apply(t)
    setThemeState(t)
    broadcast()
  }, [])
  const toggle = useCallback(() => setTheme(theme === 'dark' ? 'light' : 'dark'), [theme, setTheme])
  return { theme, setTheme, toggle }
}

/** 主题订阅：等宽数据组件（sigma/mermaid）重渲染用——切主题、跨标签、系统跟随都会触发。 */
export function useThemeTick() {
  const [tick, setTick] = useState(0)
  useEffect(() => onThemeChange(() => setTick((t) => t + 1)), [])
  return tick
}

// ---- 模块级监听：跨标签 + 系统偏好（guard 测试环境 jsdom 无 matchMedia） ----
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (e) => {
    if (e.key !== 'engram-theme' || !e.newValue) return
    apply(e.newValue === 'dark' ? 'dark' : 'light')
    broadcast()
  })
  if (typeof window.matchMedia === 'function') {
    window
      .matchMedia('(prefers-color-scheme: dark)')
      .addEventListener('change', (e) => {
        try {
          if (localStorage.getItem('engram-theme')) return // 有手动覆盖则不跟随
        } catch {
          /* 忽略 */
        }
        apply(e.matches ? 'dark' : 'light')
        broadcast()
      })
  }
}

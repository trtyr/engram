import { clsx, type ClassValue } from "clsx"
import { twMerge } from "tailwind-merge"

/**
 * 写文本到剪贴板。优先 Clipboard API（仅 HTTPS/localhost 安全上下文可用），
 * 非 secure context（如 http://192.168.x.x 局域网部署）下 navigator.clipboard 为
 * undefined——回退隐藏 textarea + execCommand('copy')。返回是否成功，调用方给反馈。
 */
export async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text)
      return true
    }
  } catch {
    // 权限拒绝等 → 走回退
  }
  try {
    const ta = document.createElement('textarea')
    ta.value = text
    ta.style.position = 'fixed'
    ta.style.opacity = '0'
    document.body.appendChild(ta)
    ta.focus()
    ta.select()
    const ok = document.execCommand('copy')
    document.body.removeChild(ta)
    return ok
  } catch {
    return false
  }
}

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

/**
 * App 顶层路由单测（P019-M3）：
 * /file-view 探活期间（authed=null）不误跳 /login——旧实现新标签页冷启动先跳 /login，
 * 探活成功后又跳 /，用户被吞到概览页而非文件页。
 */
import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'

vi.mock('@/lib/api', () => ({
  getToken: () => 'tok',
  clearToken: vi.fn(),
  logoutSession: vi.fn(),
  api: {
    get: vi.fn(async () => ({})),
    post: vi.fn(async () => ({})),
  },
}))
vi.mock('@/lib/status', () => ({
  useSystemStatus: () => ({}),
}))

import App from '@/App'

describe('App 探活竞态', () => {
  it('/file-view 探活期间渲染等待态而非 /login', () => {
    // fetch 永不 resolve：authed 恒 null（探活进行中）
    vi.stubGlobal('fetch', vi.fn(() => new Promise(() => {})))
    render(
      <MemoryRouter initialEntries={['/file-view']}>
        <App />
      </MemoryRouter>,
    )
    // 探活期间：顶层 early-return 空白等待态（不渲染任何路由 → 不可能误跳 /login）
    expect(document.querySelector('.min-h-screen')).toBeTruthy()
    expect(screen.queryByText(/登录/)).toBeNull()
    expect(screen.queryByPlaceholderText(/密码/)).toBeNull()
    vi.unstubAllGlobals()
  })
})

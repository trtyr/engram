/** RhythmPane（设置→节律）：逾期警示 / 积压年龄 / 安装向导片段。 */
import { cleanup, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const apiGet = vi.fn()

vi.mock('@/lib/api', () => ({
  api: {
    get: (...a: unknown[]) => apiGet(...a),
    post: vi.fn(),
    patch: vi.fn(),
    del: vi.fn(),
  },
}))

import Settings from './Settings'

const DAY = 86_400_000

describe('Settings 节律分区', () => {
  beforeEach(() => {
    // Node 实验版 localStorage 抢注（--localstorage-file 警告）导致 jsdom 的不可用——stub 掉
    vi.stubGlobal('localStorage', {
      getItem: vi.fn(() => null),
      setItem: vi.fn(),
      removeItem: vi.fn(),
      clear: vi.fn(),
    })
    apiGet.mockReset()
  })
  afterEach(cleanup)

  const openPane = async () => {
    render(<Settings />)
    screen.getByRole('button', { name: '节律' }).click()
    await waitFor(() => expect(apiGet).toHaveBeenCalledWith('/memory/rhythm/status'))
  }

  it('心跳逾期渲染警示（超过期望周期 1.5 倍）', async () => {
    apiGet.mockImplementation((url: string) => {
      if (url === '/memory/rhythm/status')
        return Promise.resolve({
          last_heartbeat: new Date(Date.now() - 3 * DAY).toISOString(),
          last_heartbeat_by: 'key:cron',
          pending_sessions: 2,
          oldest_pending_age_secs: 9000,
        })
      if (url === '/jobs?limit=100') return Promise.resolve([])
      return Promise.resolve([])
    })
    await openPane()
    expect(await screen.findByText(/● 逾期/)).toBeTruthy()
    expect(screen.getByText(/key:cron/)).toBeTruthy()
    // 积压对象面
    expect(screen.getByText('2')).toBeTruthy()
    expect(screen.getByText(/2\.5 小时/)).toBeTruthy()
  })

  it('未装状态渲染引导（无心跳记录）', async () => {
    apiGet.mockImplementation((url: string) => {
      if (url === '/memory/rhythm/status')
        return Promise.resolve({ last_heartbeat: null, pending_sessions: 0 })
      if (url === '/jobs?limit=100') return Promise.resolve([])
      return Promise.resolve([])
    })
    await openPane()
    expect(await screen.findByText(/未装/)).toBeTruthy()
  })

  it('安装向导 crontab 片段含端点与幂等说明', async () => {
    apiGet.mockImplementation((url: string) => {
      if (url === '/memory/rhythm/status')
        return Promise.resolve({
          last_heartbeat: new Date().toISOString(),
          pending_sessions: 0,
        })
      if (url === '/jobs?limit=100') return Promise.resolve([])
      return Promise.resolve([])
    })
    await openPane()
    const pre = await screen.findByText(/\/memory\/rhythm\/heartbeat/, { selector: 'pre' })
    expect(pre.textContent).toContain('/memory/distill')
    expect(pre.textContent).toContain('"full":true,"via":"cron"')
    expect(pre.textContent).toContain('amk_')
  })
})

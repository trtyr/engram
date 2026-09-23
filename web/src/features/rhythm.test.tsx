/** RhythmPane（设置→节律）：内置节律面板——配置回显 / 保存 / 运行面（读 jobs 表）。 */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const apiGet = vi.fn()
const apiPut = vi.fn()

vi.mock('@/lib/api', () => ({
  api: {
    get: (...a: unknown[]) => apiGet(...a),
    post: vi.fn(),
    patch: vi.fn(),
    put: (...a: unknown[]) => apiPut(...a),
    del: vi.fn(),
  },
}))

import Settings from './Settings'

describe('Settings 节律分区（内置节律）', () => {
  beforeEach(() => {
    // Node 实验版 localStorage 抢注（--localstorage-file 警告）导致 jsdom 的不可用——stub 掉
    vi.stubGlobal('localStorage', {
      getItem: vi.fn(() => null),
      setItem: vi.fn(),
      removeItem: vi.fn(),
      clear: vi.fn(),
    })
    apiGet.mockReset()
    apiPut.mockReset()
  })
  afterEach(cleanup)

  const openPane = async () => {
    render(<Settings />)
    screen.getByRole('button', { name: '节律' }).click()
    await waitFor(() => expect(apiGet).toHaveBeenCalledWith('/settings/rhythm'))
  }

  const mockJobs = [
    {
      id: 'a',
      kind: 'rhythm_extract',
      status: 'pending',
      due_at: '2030-01-01T06:00:00Z',
      created_at: '2026-01-01T00:00:00Z',
    },
    {
      id: 'b',
      kind: 'rhythm_extract',
      status: 'succeeded',
      created_at: '2026-01-01T00:00:00Z',
      finished_at: '2026-01-01T00:05:00Z',
    },
    {
      id: 'c',
      kind: 'rhythm_consolidate',
      status: 'failed',
      created_at: '2026-01-01T00:00:00Z',
      finished_at: '2026-01-01T00:06:00Z',
    },
  ]

  it('配置回显 + 下次触发 + 成功率（读 jobs 表）', async () => {
    apiGet.mockImplementation((url: string) => {
      if (url === '/settings/rhythm')
        return Promise.resolve({ enabled: true, extract_every_hours: 6, consolidate_hour_local: 3 })
      if (url.startsWith('/jobs?kind=rhythm')) return Promise.resolve(mockJobs)
      return Promise.resolve([])
    })
    await openPane()
    // 成功率 1/3 = 33%
    expect(await screen.findByText(/33%/)).toBeTruthy()
    // 下次触发展示 pending 的到期时间
    expect(screen.getByText(/下次增量蒸馏/)).toBeTruthy()
    expect(screen.getByText(/下次每日整理/)).toBeTruthy()
    // 缺省配置回显：每 6 小时
    expect(
      (screen.getByLabelText('增量蒸馏周期') as HTMLSelectElement).value === '6',
    ).toBeTruthy()
  })

  it('保存按钮把配置 PUT 上去并刷新运行面', async () => {
    apiGet.mockImplementation((url: string) => {
      if (url === '/settings/rhythm')
        return Promise.resolve({ enabled: true, extract_every_hours: 6, consolidate_hour_local: 3 })
      if (url.startsWith('/jobs?kind=rhythm')) return Promise.resolve([])
      return Promise.resolve([])
    })
    apiPut.mockResolvedValue({ enabled: false, extract_every_hours: 6, consolidate_hour_local: 3 })
    await openPane()
    fireEvent.click(screen.getByLabelText(/启用节律/))
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    await waitFor(() =>
      expect(apiPut).toHaveBeenCalledWith(
        '/settings/rhythm',
        expect.objectContaining({ enabled: false }),
      ),
    )
  })

  it('停用状态显示自然停止提示', async () => {
    apiGet.mockImplementation((url: string) => {
      if (url === '/settings/rhythm')
        return Promise.resolve({ enabled: false, extract_every_hours: 6, consolidate_hour_local: 3 })
      if (url.startsWith('/jobs?kind=rhythm')) return Promise.resolve([])
      return Promise.resolve([])
    })
    await openPane()
    expect(await screen.findByText(/已停用/)).toBeTruthy()
  })
})

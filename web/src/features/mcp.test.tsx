/**
 * MCP 管理页测试：状态条总开关 / 工具拨杆开关 / 接入卡（密钥只选不建）。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import Mcp from './Mcp'

// ---- api mock ----
let mcpState = { enabled: true, disabled_tools: [] as string[] }
const calls: { put: [string, unknown][] } = { put: [] }
const mcpInfo = () => ({
  endpoint: '/mcp',
  protocol_version: '2025-11-25',
  server_name: 'engram',
  server_version: '0.1.0',
  enabled: mcpState.enabled,
  disabled_tools: mcpState.disabled_tools,
  instructions: 'Engram——用户长期记忆平台。\n1. 会话开始：调用 memory_context。',
  tools: [
    {
      name: 'memory_context',
      description: '装载用户记忆上下文包。\n何时用：会话开始。',
      read_only: true,
      destructive: false,
    },
    {
      name: 'memory_write_session',
      description: '写入一段对话到 L0 会话。',
      read_only: false,
      destructive: false,
    },
    {
      name: 'memory_forget',
      description: '遗忘会话。',
      read_only: false,
      destructive: true,
    },
  ],
})
vi.mock('@/lib/api', async (importOriginal) => {
  const mod = await importOriginal<typeof import('@/lib/api')>()
  return {
    ...mod,
    api: {
      ...mod.api,
      get: vi.fn(async (p: string) => {
        if (p === '/settings/mcp') return mcpInfo()
        if (p === '/settings/api-keys') {
          return [
            {
              id: 'k1',
              name: 'claude-code',
              key_prefix: 'amk_abc12345',
              scopes: ['memory'],
              created_at: '2026-09-01T00:00:00Z',
              last_used_at: null,
              revoked_at: null,
            },
            {
              id: 'k2',
              name: 'wiki-bot',
              key_prefix: 'amk_def67890',
              scopes: ['wiki'],
              created_at: '2026-09-01T00:00:00Z',
              last_used_at: null,
              revoked_at: null,
            },
          ]
        }
        return []
      }),
      put: vi.fn(async (p: string, b?: unknown) => {
        calls.put.push([p, b])
        if (p === '/settings/mcp') {
          const body = b as { enabled?: boolean; disabled_tools?: string[] }
          if (body.enabled !== undefined) mcpState.enabled = body.enabled
          if (body.disabled_tools !== undefined) mcpState.disabled_tools = body.disabled_tools
          return mcpInfo()
        }
        return {}
      }),
    },
  }
})

describe('Mcp 管理页', () => {
  beforeEach(() => {
    calls.put.length = 0
    mcpState = { enabled: true, disabled_tools: [] }
    vi.clearAllMocks()
  })

  it('状态条 + 工具面密集列表（语义徽标、开关），接入卡不出现非 memory key', async () => {
    render(
      <MemoryRouter>
        <Mcp />
      </MemoryRouter>,
    )
    // 状态条
    await waitFor(() => screen.getByText('运行中'))
    expect(screen.getAllByText('http://localhost:3000/mcp').length).toBeGreaterThan(0)
    // 工具面：三行 + 语义徽标 + 启用计数
    expect(screen.getByText('工具面')).toBeTruthy()
    expect(screen.getByText('启用 3/3')).toBeTruthy()
    expect(screen.getByText('memory_context')).toBeTruthy()
    expect(screen.getByText('memory_write_session')).toBeTruthy()
    expect(screen.getByText('memory_forget')).toBeTruthy()
    expect(screen.getByText('只读')).toBeTruthy()
    expect(screen.getByText('破坏性')).toBeTruthy()
    // 每个工具一个拨杆开关
    expect(screen.getAllByRole('switch').length).toBe(3)
    // 接入卡：非 memory scope 的 key（wiki-bot）不进选择器
    expect(screen.queryByText('wiki-bot')).toBeNull()
    expect(screen.getByText(/claude-code（amk_abc12345…）/)).toBeTruthy()
  })

  it('总开关：关闭 → PUT enabled=false；再开 → PUT enabled=true', async () => {
    render(
      <MemoryRouter>
        <Mcp />
      </MemoryRouter>,
    )
    const closeBtn = await screen.findByRole('button', { name: '关闭服务' })
    fireEvent.click(closeBtn)
    await waitFor(() => expect(calls.put.length).toBe(1))
    expect(calls.put[0]).toEqual(['/settings/mcp', { enabled: false }])
    expect(mcpState.enabled).toBe(false)

    // 状态条刷新为已关闭，按钮变「开启服务」
    await waitFor(() => screen.getByText('已关闭'))
    fireEvent.click(screen.getByRole('button', { name: '开启服务' }))
    await waitFor(() => expect(calls.put.length).toBe(2))
    expect(calls.put[1]).toEqual(['/settings/mcp', { enabled: true }])
  })

  it('工具拨杆：停用 → PUT 增量 disabled_tools；启用 → 移除', async () => {
    render(
      <MemoryRouter>
        <Mcp />
      </MemoryRouter>,
    )
    await waitFor(() => screen.getByText('memory_write_session'))

    // 停用 memory_write_session（该行拨杆）
    fireEvent.click(screen.getByRole('switch', { name: '停用 memory_write_session' }))
    await waitFor(() => expect(calls.put.length).toBe(1))
    expect(calls.put[0]).toEqual(['/settings/mcp', { disabled_tools: ['memory_write_session'] }])
    expect(mcpState.disabled_tools).toEqual(['memory_write_session'])

    // 状态刷新：计数变 2/3，开关语义反转
    await waitFor(() => screen.getByText(/启用 2\/3/))
    const enableSwitch = screen.getByRole('switch', { name: '启用 memory_write_session' })
    expect(enableSwitch.getAttribute('aria-checked')).toBe('false')

    // 再启用 → 移除
    fireEvent.click(enableSwitch)
    await waitFor(() => expect(calls.put.length).toBe(2))
    expect(calls.put[1]).toEqual(['/settings/mcp', { disabled_tools: [] }])
    await waitFor(() => screen.getByText(/启用 3\/3/))
  })
})

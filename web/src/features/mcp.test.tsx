/**
 * MCP 管理页测试：状态条总开关 / 域 Tabs / 工具拨杆开关。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
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
      domain: 'memory',
      description: '装载用户记忆上下文包。\n何时用：会话开始。',
      read_only: true,
      destructive: false,
    },
    {
      name: 'memory_write_session',
      domain: 'memory',
      description: '写入一段对话到 L0 会话。',
      read_only: false,
      destructive: false,
    },
    {
      name: 'memory_forget',
      domain: 'memory',
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
      get: vi.fn(async (p: string) => (p === '/settings/mcp' ? mcpInfo() : [])),
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

  it('状态条 + 域 Tabs（记忆域默认选中）+ 工具开关列表', async () => {
    render(<Mcp />)
    // 状态条
    await waitFor(() => screen.getByText('运行中'))
    expect(screen.getAllByText('http://localhost:3000/mcp').length).toBeGreaterThan(0)
    // 域 tab（后端同源 domain → 中文标签 + 计数）
    const tab = screen.getByRole('button', { name: '用户记忆' })
    expect(tab.getAttribute('aria-pressed')).toBe('true')
    expect(screen.getByText('用户记忆域工具')).toBeTruthy()
    // 工具行：名称 + 语义徽标 + 拨杆
    expect(screen.getByText('memory_context')).toBeTruthy()
    expect(screen.getByText('memory_write_session')).toBeTruthy()
    expect(screen.getByText('memory_forget')).toBeTruthy()
    expect(screen.getByText('只读')).toBeTruthy()
    expect(screen.getByText('破坏性')).toBeTruthy()
    expect(screen.getAllByRole('switch').length).toBe(3)
    expect(screen.getByText(/启用 3\/3/)).toBeTruthy()
  })

  it('总开关：关闭 → PUT enabled=false；再开 → PUT enabled=true', async () => {
    render(<Mcp />)
    const closeBtn = await screen.findByRole('button', { name: '关闭服务' })
    fireEvent.click(closeBtn)
    await waitFor(() => expect(calls.put.length).toBe(1))
    expect(calls.put[0]).toEqual(['/settings/mcp', { enabled: false }])
    expect(mcpState.enabled).toBe(false)

    await waitFor(() => screen.getByText('已关闭'))
    fireEvent.click(screen.getByRole('button', { name: '开启服务' }))
    await waitFor(() => expect(calls.put.length).toBe(2))
    expect(calls.put[1]).toEqual(['/settings/mcp', { enabled: true }])
  })

  it('工具拨杆：停用 → PUT 增量 disabled_tools；启用 → 移除', async () => {
    render(<Mcp />)
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

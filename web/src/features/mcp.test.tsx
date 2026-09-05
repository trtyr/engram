/**
 * MCP 管理页测试：服务开关 / 工具粒度开关 / 连接配置（密钥在设置页管理，本页只读列表）。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react'
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

  it('渲染服务状态与工具清单（含语义标注），非 memory scope 的 key 不进选择器', async () => {
    render(<Mcp />)
    await waitFor(() => screen.getByText('工具管理（3）'))
    // 服务状态卡
    expect(screen.getByText('MCP 服务运行中')).toBeTruthy()
    // 端点信息（jsdom origin = http://localhost:3000）
    expect(screen.getAllByText('http://localhost:3000/mcp').length).toBeGreaterThan(0)
    // 工具清单：三个工具都在，语义标注正确
    expect(screen.getByText('memory_context')).toBeTruthy()
    expect(screen.getByText('memory_write_session')).toBeTruthy()
    expect(screen.getByText('memory_forget')).toBeTruthy()
    expect(screen.getByText('只读')).toBeTruthy()
    expect(screen.getByText('破坏性')).toBeTruthy()
    // 非 memory scope 的 key（wiki-bot）不进密钥选择器
    expect(screen.queryByText('wiki-bot')).toBeNull()
    expect(screen.getByText(/claude-code（amk_abc12345…/)).toBeTruthy()
  })

  it('服务开关：关闭 → PUT enabled=false；再开 → PUT enabled=true', async () => {
    render(<Mcp />)
    const closeBtn = await screen.findByRole('button', { name: '关闭服务' })
    fireEvent.click(closeBtn)
    await waitFor(() => expect(calls.put.length).toBe(1))
    expect(calls.put[0]).toEqual(['/settings/mcp', { enabled: false }])
    expect(mcpState.enabled).toBe(false)

    // 状态刷新为已关闭，按钮变「开启服务」
    await waitFor(() => screen.getByText('MCP 服务已关闭'))
    fireEvent.click(screen.getByRole('button', { name: '开启服务' }))
    await waitFor(() => expect(calls.put.length).toBe(2))
    expect(calls.put[1]).toEqual(['/settings/mcp', { enabled: true }])
  })

  it('工具开关：停用 → PUT 增量 disabled_tools；启用 → 移除', async () => {
    render(<Mcp />)
    await waitFor(() => screen.getByText('工具管理（3）'))

    // 停用 memory_write_session（该行按钮显示「停用」）——按行定位
    const row = screen.getByText('memory_write_session').closest('tr')!
    fireEvent.click(within(row).getByRole('button', { name: '停用' }))
    await waitFor(() => expect(calls.put.length).toBe(1))
    expect(calls.put[0]).toEqual(['/settings/mcp', { disabled_tools: ['memory_write_session'] }])
    expect(mcpState.disabled_tools).toEqual(['memory_write_session'])

    // 状态刷新后该行显示「停用」，按钮变「启用」
    const rowAfter = screen.getByText('memory_write_session').closest('tr')!
    await waitFor(() => expect(within(rowAfter).getByText('停用', { selector: 'span' })).toBeTruthy())
    fireEvent.click(within(rowAfter).getByRole('button', { name: '启用' }))
    await waitFor(() => expect(calls.put.length).toBe(2))
    expect(calls.put[1]).toEqual(['/settings/mcp', { disabled_tools: [] }])
  })
})

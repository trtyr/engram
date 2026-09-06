/**
 * MCP 管理页测试：状态条总开关 / 域 Tabs / 域工具 + 域内操作两级开关（渐进式发现）。
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
  instructions:
    'Engram——单用户 AI 长期记忆平台。MCP 工具面采用渐进式发现……参数细节用 {"action":"help"} 取回。',
  tools: [
    {
      name: 'memory',
      domain: 'memory',
      description:
        '用户记忆域（单一入口）。\n【本域操作 3 个】调用形态 {"action":"…",…}\n- context：装载上下文\n- write_session：写入会话\n- forget：【破坏性】遗忘',
      read_only: false,
      destructive: true,
      parameters: {
        type: 'object',
        properties: { action: { type: 'string', description: '操作名。' } },
        required: ['action'],
      },
      actions: [
        {
          action: 'context',
          summary: '装载用户记忆上下文包（会话开场调用一次）',
          destructive: false,
          disabled: mcpState.disabled_tools.includes('memory.context'),
          parameters: {
            type: 'object',
            properties: {
              query: { type: 'string', description: '可选：相关性查询词。' },
            },
            required: [],
          },
        },
        {
          action: 'write_session',
          summary: '写入一段对话到 L0 会话。',
          destructive: false,
          disabled: mcpState.disabled_tools.includes('memory.write_session'),
          parameters: { type: 'object', properties: {}, required: [] },
        },
        {
          action: 'forget',
          summary: '遗忘会话（void 作废 / erase 物理删除）。',
          destructive: true,
          disabled: mcpState.disabled_tools.includes('memory.forget'),
          parameters: { type: 'object', properties: {}, required: [] },
        },
      ],
    },
    {
      name: 'wiki',
      domain: 'wiki',
      description: 'Wiki 域（单一入口）：世界知识库。\n【本域操作 2 个】',
      read_only: false,
      destructive: true,
      parameters: {
        type: 'object',
        properties: { action: { type: 'string', description: '操作名。' } },
        required: ['action'],
      },
      actions: [
        {
          action: 'search',
          summary: 'Wiki 检索（FTS + 向量融合）。',
          destructive: false,
          disabled: mcpState.disabled_tools.includes('wiki.search'),
          parameters: { type: 'object', properties: {}, required: [] },
        },
        {
          action: 'write_page',
          summary: '写/覆盖一个页面。',
          destructive: false,
          disabled: mcpState.disabled_tools.includes('wiki.write_page'),
          parameters: { type: 'object', properties: {}, required: [] },
        },
      ],
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

  it('状态条 + 域 Tabs（记忆域默认选中）+ 域工具行（操作收起）', async () => {
    render(<Mcp />)
    await waitFor(() => screen.getByText('运行中'))
    expect(screen.getAllByText('http://localhost:3000/mcp').length).toBeGreaterThan(0)
    // 域 tab：标签 + 操作数计数
    const tab = screen.getByRole('button', { name: '用户记忆' })
    expect(tab.getAttribute('aria-pressed')).toBe('true')
    expect(screen.getByText('用户记忆域工具')).toBeTruthy()
    // 域工具行 + 整域拨杆（操作收起时不渲染操作开关）
    expect(screen.getByText('memory')).toBeTruthy()
    expect(screen.getAllByRole('switch').length).toBe(1)
    expect(screen.getByText(/操作启用 3\/3/)).toBeTruthy()
  })

  it('展开域工具：操作列表 + 破坏性标注 + 单操作参数 Schema', async () => {
    render(<Mcp />)
    await waitFor(() => screen.getByText('memory'))
    expect(screen.queryByText('工具描述')).toBeNull()

    // 展开域工具 → 描述 + 操作列表
    fireEvent.click(screen.getByText('memory'))
    expect(screen.getByText('工具描述')).toBeTruthy()
    expect(screen.getByText(/域内操作（3）/)).toBeTruthy()
    expect(screen.getByText('context')).toBeTruthy()
    expect(screen.getByText('write_session')).toBeTruthy()
    expect(screen.getByText('forget')).toBeTruthy()
    expect(screen.getByText(/装载用户记忆上下文包/)).toBeTruthy()
    // forget 操作行带破坏性角标（动作级，页面唯一）
    expect(screen.getAllByText('破坏性').length).toBeGreaterThanOrEqual(1)
    // 操作行展开参数
    fireEvent.click(screen.getByText('context'))
    expect(screen.getByText('可选：相关性查询词。')).toBeTruthy()
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

  it('操作拨杆：停用 memory.forget → PUT 域.action 键；启用 → 移除', async () => {
    render(<Mcp />)
    await waitFor(() => screen.getByText('memory'))
    fireEvent.click(screen.getByText('memory'))
    await waitFor(() => screen.getByRole('switch', { name: '停用 memory.forget' }))

    fireEvent.click(screen.getByRole('switch', { name: '停用 memory.forget' }))
    await waitFor(() => expect(calls.put.length).toBe(1))
    expect(calls.put[0]).toEqual(['/settings/mcp', { disabled_tools: ['memory.forget'] }])
    expect(mcpState.disabled_tools).toEqual(['memory.forget'])

    // 计数变 2/3，开关语义反转
    await waitFor(() => screen.getByText(/操作启用 2\/3/))
    const enableSwitch = screen.getByRole('switch', { name: '启用 memory.forget' })
    expect(enableSwitch.getAttribute('aria-checked')).toBe('false')

    fireEvent.click(enableSwitch)
    await waitFor(() => expect(calls.put.length).toBe(2))
    expect(calls.put[1]).toEqual(['/settings/mcp', { disabled_tools: [] }])
    await waitFor(() => screen.getByText(/操作启用 3\/3/))
  })

  it('Wiki 域 Tab：切换后展示 wiki 工具，操作可单独停用', async () => {
    render(<Mcp />)
    await waitFor(() => screen.getByText('运行中'))

    fireEvent.click(screen.getByRole('button', { name: 'Wiki' }))
    expect(screen.getByText('Wiki域工具')).toBeTruthy()
    expect(screen.getByText('wiki')).toBeTruthy()
    expect(screen.getByText(/操作启用 2\/2/)).toBeTruthy()

    // 展开后停用 wiki.write_page → PUT 增量 + 计数刷新
    fireEvent.click(screen.getByText('wiki'))
    fireEvent.click(screen.getByRole('switch', { name: '停用 wiki.write_page' }))
    await waitFor(() => expect(calls.put.length).toBe(1))
    expect(calls.put[0]).toEqual(['/settings/mcp', { disabled_tools: ['wiki.write_page'] }])
    await waitFor(() => screen.getByText(/操作启用 1\/2/))
  })
})

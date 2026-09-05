/**
 * MCP 管理页测试：端点信息 / 工具清单渲染 / 密钥签发带 scope。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import Mcp from './Mcp'

// ---- api mock ----
const calls: { post: [string, unknown][] } = { post: [] }
vi.mock('@/lib/api', async (importOriginal) => {
  const mod = await importOriginal<typeof import('@/lib/api')>()
  return {
    ...mod,
    api: {
      ...mod.api,
      get: vi.fn(async (p: string) => {
        if (p === '/settings/mcp') {
          return {
            endpoint: '/mcp',
            protocol_version: '2025-11-25',
            server_name: 'engram',
            server_version: '0.1.0',
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
          }
        }
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
      post: vi.fn(async (p: string, b?: unknown) => {
        calls.post.push([p, b])
        if (p === '/settings/api-keys') return { key: 'amk_new' }
        return {}
      }),
    },
  }
})

describe('Mcp 管理页', () => {
  beforeEach(() => {
    calls.post.length = 0
    vi.clearAllMocks()
  })

  it('渲染端点信息与工具清单（含语义标注）', async () => {
    render(<Mcp />)
    await waitFor(() => screen.getByText('工具清单（3）'))
    // 端点信息卡（jsdom origin = http://localhost:3000）
    expect(screen.getAllByText('http://localhost:3000/mcp').length).toBeGreaterThan(0)
    // 工具清单：三个工具都在
    expect(screen.getByText('memory_context')).toBeTruthy()
    expect(screen.getByText('memory_write_session')).toBeTruthy()
    expect(screen.getByText('memory_forget')).toBeTruthy()
    // 语义标注：只读 / 写入 / 破坏性
    expect(screen.getByText('只读')).toBeTruthy()
    expect(screen.getByText('破坏性')).toBeTruthy()
    // 非 memory scope 的 key（wiki-bot）不出现在 MCP 密钥表
    expect(screen.queryByText('wiki-bot')).toBeNull()
    expect(screen.getByText('claude-code')).toBeTruthy()
  })

  it('签发 MCP 密钥：默认 memory scope，勾选后带 erase', async () => {
    render(<Mcp />)
    const nameInput = await screen.findByLabelText('名称')
    const submit = screen.getByRole('button', { name: '签发' })

    fireEvent.change(nameInput, { target: { value: 'cursor' } })
    fireEvent.click(submit)
    await waitFor(() => expect(calls.post.length).toBe(1))
    expect(calls.post[0]).toEqual(['/settings/api-keys', { name: 'cursor', scopes: ['memory'] }])

    // 勾选 erase → 签发带双 scope
    fireEvent.change(nameInput, { target: { value: 'cursor-erase' } })
    fireEvent.click(screen.getByRole('checkbox'))
    fireEvent.click(submit)
    await waitFor(() => expect(calls.post.length).toBe(2))
    expect(calls.post[1]).toEqual(['/settings/api-keys', { name: 'cursor-erase', scopes: ['memory', 'erase'] }])
  })
})

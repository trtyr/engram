/**
 * P3 功能测试：跨域 /search、provider 编辑/删除、re-embed。
 * 以 Dashboard GlobalSearch、Settings Providers、Knowledge ChunksPanel 的行为面为对象。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'

// ---- api mock ----
vi.mock('@/lib/api', () => {
  const state = {
    providers: [] as { id: string; name: string; base_url: string; models: { id: string; capabilities: string[] }[]; is_default: boolean }[],
    docs: [] as { id: string; title: string; source_uri: string; mime: string | null; status: string; error: string | null; created_at: string }[],
    chunks: [] as { seq: number; content: string; embed_failed: boolean }[],
    searchResult: null as { query: string; hits: { id: string; domain: string; score: number; snippet: string; title?: string | null }[] } | null,
  }
  const api = {
    get: vi.fn(async (p: string) => {
      if (p.startsWith('/settings/llm/providers')) return state.providers
      if (p.startsWith('/knowledge/documents/') && p.endsWith('/chunks')) return state.chunks
      if (p.startsWith('/knowledge/documents')) return state.docs
      if (p.startsWith('/memory/atoms')) return []
      if (p.startsWith('/wiki/pages')) return []
      if (p.startsWith('/memory/persona')) return []
      if (p.startsWith('/jobs')) return []
      if (p.startsWith('/llm/usage')) return []
      return []
    }),
    post: vi.fn(async (p: string, _b?: unknown) => {
      if (p === '/search') return state.searchResult
      if (p.includes('/re-embed')) return undefined
      return {}
    }),
    put: vi.fn(async () => ({})),
    del: vi.fn(async () => undefined),
    __state: state,
  }
  return { api }
})

import { api } from '@/lib/api'

interface MockState {
  providers: { id: string; name: string; base_url: string; models: { id: string; capabilities: string[] }[]; is_default: boolean }[]
  docs: { id: string; title: string; source_uri: string; mime: string | null; status: string; error: string | null; created_at: string }[]
  chunks: { seq: number; content: string; embed_failed: boolean }[]
  searchResult: { query: string; hits: { id: string; domain: string; score: number; snippet: string; title?: string | null }[] } | null
}

const mockState = (api as unknown as { __state: MockState }).__state

import Dashboard from '@/features/Dashboard'
import Settings from '@/features/Settings'
import Knowledge from '@/features/Knowledge'

const wrap = (ui: React.ReactElement) => <MemoryRouter initialEntries={['/']}>{ui}</MemoryRouter>

beforeEach(() => {
  vi.clearAllMocks()
  mockState.providers = []
  mockState.docs = []
  mockState.chunks = []
  mockState.searchResult = null
})

describe('跨域统一检索 GlobalSearch', () => {
  it('输入 query 后调 POST /search 并展示 domain 标签', async () => {
    mockState.searchResult = {
      query: 'tokio',
      hits: [
        { id: 'h1', domain: 'memory', score: 0.9, snippet: '记忆片段', title: '偏好' },
        { id: 'h2', domain: 'wiki', score: 0.8, snippet: 'wiki 片段', title: 'Tokio' },
      ],
    }
    render(wrap(<Dashboard />))
    const input = await screen.findByPlaceholderText(/跨域检索/)
    fireEvent.change(input, { target: { value: 'tokio' } })
    fireEvent.click(screen.getByRole('button', { name: '搜索' }))
    await waitFor(() => {
      expect(api.post).toHaveBeenCalledWith('/search', { query: 'tokio', limit: 10 })
      expect(screen.getByText('记忆')).toBeInTheDocument()
      expect(screen.getByText('Wiki')).toBeInTheDocument()
      expect(screen.getByText('Tokio')).toBeInTheDocument()
    })
  })
})

describe('Provider 编辑 / 删除', () => {
  const p1 = {
    id: 'p1',
    name: 'openai',
    base_url: 'https://api.openai.com/v1',
    models: [{ id: 'gpt-4', capabilities: ['chat'] }],
    is_default: true,
  }

  it('编辑：填表单后 PUT（name 不可改，key 留空不传）', async () => {
    mockState.providers = [p1]
    render(wrap(<Settings />))
    await screen.findByText('openai')
    fireEvent.click(screen.getByRole('button', { name: '编辑' }))
    const baseInput = screen.getByPlaceholderText('Base URL（OpenAI 兼容）') as HTMLInputElement
    expect(baseInput.value).toBe('https://api.openai.com/v1')
    fireEvent.click(screen.getByRole('button', { name: '保存修改' }))
    await waitFor(() => {
      expect(api.put).toHaveBeenCalledWith(
        '/settings/llm/providers/p1',
        expect.objectContaining({ base_url: 'https://api.openai.com/v1' }),
      )
      // key 留空 → 不应出现在 body
      const body = (api.put as ReturnType<typeof vi.fn>).mock.calls[0][1] as Record<string, unknown>
      expect(body).not.toHaveProperty('api_key')
    })
  })

  it('删除：确认后 DELETE', async () => {
    mockState.providers = [p1]
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    render(wrap(<Settings />))
    await screen.findByText('openai')
    fireEvent.click(screen.getByRole('button', { name: '删除' }))
    await waitFor(() => {
      expect(api.del).toHaveBeenCalledWith('/settings/llm/providers/p1')
    })
  })
})

describe('Knowledge re-embed', () => {
  it('有嵌入失败块时显示重嵌按钮并 POST re-embed', async () => {
    mockState.docs = [
      { id: 'd1', title: 'doc', source_uri: '', mime: null, status: 'ready', error: null, created_at: '2026-08-20T00:00:00Z' },
    ]
    mockState.chunks = [
      { seq: 1, content: '块1', embed_failed: true },
      { seq: 2, content: '块2', embed_failed: false },
    ]
    render(wrap(<Knowledge />))
    await screen.findByText('doc')
    fireEvent.click(screen.getByRole('button', { name: '分块' }))
    await screen.findByText('1 个分块嵌入失败（FTS 降级）')
    fireEvent.click(screen.getByRole('button', { name: '重嵌缺失块' }))
    await waitFor(() => {
      expect(api.post).toHaveBeenCalledWith('/knowledge/documents/d1/re-embed')
    })
  })
})

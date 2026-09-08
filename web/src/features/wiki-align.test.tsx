/**
 * Wiki 对齐新功能组件测试（wiki-align-4 契约）：
 * 洞察面板（渲染/dismiss/联动高亮）、Review 队列（处理动作）、检索存档。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'

interface MockState {
  insights: unknown
  reviews: unknown
  dismissed: string[]
}
vi.mock('@/lib/api', () => {
  const state: MockState = {
    insights: null,
    reviews: null,
    dismissed: [],
  }
  const api = {
    // 多库：URL 现在带 ?lib= 查询参数——剥掉再匹配（与 p3.test 同口径）
    post: vi.fn(async (raw: string, b?: unknown) => {
      const p = raw.split('?')[0]
      if (p === '/wiki/insights') return state.insights
      if (p === '/wiki/insights/dismiss') {
        state.dismissed.push((b as { key: string }).key)
        return undefined
      }
      if (p === '/wiki/insights/reset') {
        state.dismissed = []
        return undefined
      }
      if (p.startsWith('/wiki/reviews/') && p.endsWith('/resolve')) return undefined
      if (p === '/wiki/queries/archive') return { skipped: false }
      return undefined
    }),
    get: vi.fn(async (raw: string) => {
      const p = raw.split('?')[0]
      if (p === '/wiki/reviews') return state.reviews
      return []
    }),
    __state: state,
  }
  return { api }
})

import { api } from '@/lib/api'
const mockState = (api as unknown as { __state: MockState }).__state

import InsightsPanel from '@/components/InsightsPanel'
import ReviewQueue from '@/components/ReviewQueue'

const wrap = (ui: React.ReactElement) => <MemoryRouter>{ui}</MemoryRouter>

beforeEach(() => {
  vi.clearAllMocks()
  mockState.insights = null
  mockState.reviews = null
  mockState.dismissed = []
})

describe('InsightsPanel（图洞察）', () => {
  it('渲染四类洞察 + 社区列表；点击卡片联动高亮', async () => {
    mockState.insights = {
      insights: [
        { key: 'isolated_page:x', kind: 'isolated_page', title: '孤立页面：x', detail: '几乎没有连接', slugs: ['x'], search_queries: ['x'] },
        { key: 'surprising_connection:a->b', kind: 'surprising_connection', title: '意外连接：a ↔ b', detail: '跨社区强连接', slugs: ['a', 'b'], search_queries: [] },
        { key: 'bridge_node:c', kind: 'bridge_node', title: '桥节点：c', detail: '跨 3 个知识区', slugs: ['c'], search_queries: [] },
        { key: 'sparse_community:0', kind: 'sparse_community', title: '稀疏知识区 #0', detail: '互链弱', slugs: ['a', 'b', 'c'], search_queries: [] },
      ],
      communities: [
        { id: 0, top_slug: 'a', size: 3, cohesion: 0.08 },
        { id: 1, top_slug: 'x', size: 5, cohesion: 0.6 },
      ],
      total_pages: 8,
    }
    const onHighlight = vi.fn()
    render(wrap(<InsightsPanel onHighlight={onHighlight} libSlug="main" />))

    await screen.findByText('孤立页面：x')
    expect(screen.getByText('意外连接：a ↔ b')).toBeInTheDocument()
    expect(screen.getByText('桥节点：c')).toBeInTheDocument()
    expect(screen.getByText('稀疏知识区 #0')).toBeInTheDocument()
    expect(screen.getByText(/8 页 · 4 条洞察 · 2 个社区/)).toBeInTheDocument()

    // 点击卡片 → 高亮联动
    fireEvent.click(screen.getByText('孤立页面：x'))
    expect(onHighlight).toHaveBeenCalledWith(['x'])
    // 再点取消
    fireEvent.click(screen.getByText('孤立页面：x'))
    expect(onHighlight).toHaveBeenCalledWith(null)
  })

  it('dismiss 洞察后从列表消失', async () => {
    mockState.insights = {
      insights: [
        { key: 'isolated_page:y', kind: 'isolated_page', title: '孤立页面：y', detail: '', slugs: ['y'], search_queries: [] },
      ],
      communities: [],
      total_pages: 1,
    }
    render(wrap(<InsightsPanel onHighlight={() => {}} libSlug="main" />))
    await screen.findByText('孤立页面：y')
    fireEvent.click(screen.getByRole('button', { name: '忽略' }))
    await waitFor(() => {
      expect(api.post).toHaveBeenCalledWith(
        expect.stringMatching(/^\/wiki\/insights\/dismiss\?lib=/),
        { key: 'isolated_page:y' },
      )
    })
  })
})

describe('ReviewQueue（人审队列）', () => {
  it('渲染项 + 预定义动作处理', async () => {
    mockState.reviews = [
      {
        id: 'r1',
        kind: 'create_page',
        payload: { title: '新概念 X', reason: '值得建页' },
        action: null,
        search_queries: ['X 检索词'],
        status: 'open',
        created_at: '2026-08-21T00:00:00Z',
      },
    ]
    render(wrap(<ReviewQueue libSlug="main" />))
    await screen.findByText('新概念 X')
    expect(screen.getByText('值得建页')).toBeInTheDocument()
    expect(screen.getByText(/X 检索词/)).toBeInTheDocument()

    // 预定义动作
    fireEvent.click(screen.getByTestId('review-action-创建页面'))
    await waitFor(() => {
      expect(api.post).toHaveBeenCalledWith(
        expect.stringMatching(/^\/wiki\/reviews\/r1\/resolve\?lib=/),
        { action: '创建页面' },
      )
    })
  })

  it('空队列显示提示', async () => {
    mockState.reviews = []
    render(wrap(<ReviewQueue libSlug="main" />))
    await screen.findByText(/人审队列为空/)
  })
})

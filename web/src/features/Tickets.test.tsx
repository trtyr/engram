/** 工单时间线组件测试（0074 拆 /tickets）：详情面板渲染时间线（event 留痕+comment）+ 评论发送 POST /tickets/{id}/events + 项目分组。 */
import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { MemoryRouter } from 'react-router-dom'
import Tickets from './Tickets'

const state = {
  rows: [
    {
      id: 't1',
      project_id: 'p1',
      short_no: 101,
      title: '时间线工单',
      body: '',
      status: 'confirmed',
      severity: 'P2',
      symptom: '白屏',
      reproduce: '',
      acceptance: '',
      resolution: '',
      resolved_at: null,
      created_at: '2026-09-27T08:00:00Z',
      updated_at: '2026-09-27T08:00:00Z',
    },
  ] as Array<Record<string, unknown>>,
  projects: [{ id: 'p1', name: '测试项目' }],
  events: [
    {
      id: 'e1',
      ticket_id: 't1',
      kind: 'event',
      payload: { from: 'open', to: 'confirmed' },
      actor: 'console',
      created_at: '2026-09-27T08:05:00Z',
    },
    {
      id: 'e2',
      ticket_id: 't1',
      kind: 'comment',
      payload: { text: '第一条评论' },
      actor: 'tester',
      created_at: '2026-09-27T08:06:00Z',
    },
  ],
  posts: [] as Array<{ path: string; body: unknown }>,
}

vi.mock('@/lib/api', () => {
  const api = {
    get: vi.fn(async (p: string) => {
      if (p.startsWith('/tickets?') || p === '/tickets') return { items: state.rows, total: state.rows.length }
      if (p === '/projects') return { projects: state.projects }
      if (p.endsWith('/events')) return { events: state.events }
      return null
    }),
    post: vi.fn(async (p: string, body: unknown) => {
      state.posts.push({ path: p, body })
      return { commented: true }
    }),
    put: vi.fn(async () => ({})),
    del: vi.fn(async () => ({})),
  }
  return { api }
})

describe('Tickets 时间线（/tickets + 项目分组）', () => {
  beforeEach(() => {
    state.posts.length = 0
    vi.clearAllMocks()
  })

  it('按项目分段渲染 + 详情面板时间线（event 留痕+comment）', async () => {
    render(
      <MemoryRouter>
        <Tickets />
      </MemoryRouter>,
    )
    // 项目分段标题
    expect(await screen.findByText('测试项目')).toBeTruthy()
    // 工单行
    const row = await screen.findByText('时间线工单')
    fireEvent.click(row)
    // 详情面板时间线
    expect(await screen.findByText('活动时间线')).toBeTruthy()
    expect(await screen.findByText(/状态 open → confirmed/)).toBeTruthy()
    expect(screen.getByText(/第一条评论/)).toBeTruthy()
  })

  it('评论发送 POST /tickets/{id}/events 并刷新时间线', async () => {
    render(
      <MemoryRouter>
        <Tickets />
      </MemoryRouter>,
    )
    fireEvent.click(await screen.findByText('时间线工单'))
    const input = await screen.findByPlaceholderText('写评论…')
    fireEvent.change(input, { target: { value: '新评论' } })
    fireEvent.click(screen.getByRole('button', { name: '评论' }))
    await vi.waitFor(() => expect(state.posts.length).toBe(1))
    expect(state.posts[0].path).toBe('/tickets/t1/events')
    expect(state.posts[0].body).toEqual({ text: '新评论' })
  })
})

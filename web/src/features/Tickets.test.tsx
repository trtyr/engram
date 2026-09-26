/** 工单时间线组件测试：详情面板渲染时间线（event 留痕+comment）+ 评论发送 POST /todos/{id}/events。 */
import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import type { Todo } from '@/lib/api'
import Tickets from './Tickets'

const state = {
  rows: [
    {
      id: 't1',
      short_no: 101,
      title: '时间线工单',
      body: '',
      kind: 'ticket',
      status: 'confirmed',
      priority: 'normal',
      severity: 'P2',
      symptom: '',
      reproduce: '',
      acceptance: '',
      resolution: '',
      tags: [],
      due_at: null,
      project_hint: null,
      done_at: null,
      resolved_at: null,
      created_at: '2026-09-27T08:00:00Z',
      updated_at: '2026-09-27T08:00:00Z',
    },
  ] as unknown as Todo[],
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
      if (p.startsWith('/todos?')) return state.rows
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

import { api } from '@/lib/api'

describe('Tickets 时间线', () => {
  beforeEach(() => {
    state.posts.length = 0
    vi.clearAllMocks()
  })

  it('详情面板渲染时间线并', async () => {
    render(<Tickets />)
    const row = await screen.findByText('时间线工单')
    fireEvent.click(row)
    expect(await screen.findByText('活动时间线')).toBeTruthy()
    expect(await screen.findByText(/状态 open → confirmed/)).toBeTruthy()
    expect(screen.getByText(/第一条评论/)).toBeTruthy()
  })

  it('评论发送 POST 并刷新时间线', async () => {
    render(<Tickets />)
    fireEvent.click(await screen.findByText('时间线工单'))
    const input = await screen.findByPlaceholderText('写评论…')
    fireEvent.change(input, { target: { value: '新评论' } })
    fireEvent.click(screen.getByRole('button', { name: '评论' }))
    await vi.waitFor(() => expect(state.posts.length).toBe(1))
    expect(state.posts[0].path).toBe('/todos/t1/events')
    expect(state.posts[0].body).toEqual({ text: '新评论' })
    expect(api.post).toHaveBeenCalledTimes(1)
  })
})

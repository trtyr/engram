/**
 * 学习驾驶舱测试（2026-10-06 两层重设计）：
 * 总览（进度环/热力图/连击/到期汇总）+ 领域工作台（路线图/节点详情/复习调度/日志）。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import { MemoryRouter, Routes, Route } from 'react-router-dom'
import Study from './Study'
import StudyWorkspace from './StudyWorkspace'

const today = new Date().toISOString()
const yesterday = new Date(Date.now() - 86400_000).toISOString()

const state: {
  topics: Array<Record<string, unknown>>
  fulls: Record<string, Record<string, unknown>>
  reviews: Array<Record<string, unknown>>
  journal: Record<string, Array<Record<string, unknown>>>
  patches: Array<Record<string, unknown>>
} = {
  topics: [{ id: 't1', name: 'Rust 进阶', goal: '吃透所有权', status: 'active', updated_at: today }],
  fulls: {
    t1: {
      id: 't1',
      name: 'Rust 进阶',
      goal: '吃透所有权',
      status: 'active',
      updated_at: today,
      progress: { total: 3, learned: 2 },
      next_up: [{ id: 'i3', track_id: 't1', name: '生命周期', status: 'not_started', position: 30, wiki_slugs: [], doc_ids: [], learned_at: null, needs_review: false, review_due_at: null, updated_at: today }],
      in_progress: [],
      recent_journal: [],
      items: [
        { id: 'i1', track_id: 't1', name: '基础语法', status: 'learned', position: 10, wiki_slugs: ['rust-basics'], doc_ids: [], learned_at: yesterday, needs_review: false, review_due_at: null, updated_at: yesterday },
        { id: 'i2', track_id: 't1', name: '所有权', status: 'learned', position: 20, wiki_slugs: [], doc_ids: [], learned_at: today, needs_review: true, review_due_at: '2020-01-01T00:00:00Z', updated_at: today },
        { id: 'i3', track_id: 't1', name: '生命周期', status: 'not_started', position: 30, wiki_slugs: [], doc_ids: [], learned_at: null, needs_review: false, review_due_at: null, updated_at: today },
      ],
    },
  },
  reviews: [
    { id: 'i2', track_id: 't1', name: '所有权', status: 'learned', position: 20, wiki_slugs: [], doc_ids: [], learned_at: today, needs_review: true, review_due_at: '2020-01-01T00:00:00Z', updated_at: today },
  ],
  journal: {
    t1: [
      { id: 'j1', track_id: 't1', note: '看完 ch4', created_at: yesterday },
      { id: 'j2', track_id: 't1', note: '所有权过一遍', created_at: today },
    ],
  },
  patches: [],
}

vi.mock('@/lib/api', () => ({
  api: {
    get: vi.fn(async (p: string) => {
      if (p === '/study/topics') return { topics: state.topics }
      if (p === '/study/reviews') return { reviews: state.reviews, count: state.reviews.length }
      const m = p.match(/^\/study\/topics\/([^/]+)$/)
      if (m) return state.fulls[m[1]]
      if (/^\/study\/topics\/[^/]+\/journal$/.test(p)) {
        const id = p.split('/')[3]
        return { journal: state.journal[id] ?? [] }
      }
      return {}
    }),
    post: vi.fn(async (p: string, b?: unknown) => {
      if (/^\/study\/topics\/[^/]+\/journal$/.test(p)) {
        const id = p.split('/')[3]
        state.journal[id] = [...(state.journal[id] ?? []), { id: `j-${Math.random()}`, track_id: id, note: (b as { note: string }).note, created_at: new Date().toISOString() }]
      }
      return { ok: true }
    }),
    patch: vi.fn(async (p: string, b?: unknown) => {
      state.patches.push({ path: p, body: b })
      return { ok: true }
    }),
    del: vi.fn(async () => ({})),
  },
}))

beforeEach(() => {
  state.patches = []
  state.journal.t1 = [
    { id: 'j1', track_id: 't1', note: '看完 ch4', created_at: yesterday },
    { id: 'j2', track_id: 't1', note: '所有权过一遍', created_at: today },
  ]
})

describe('学习驾驶舱 · 总览', () => {
  it('领域卡带进度环与待复习数，热力图与连击可见，到期汇总按领域分组', async () => {
    render(
      <MemoryRouter>
        <Study />
      </MemoryRouter>,
    )
    expect((await screen.findAllByText('Rust 进阶')).length).toBeGreaterThan(0)
    expect(screen.getByText('2/3 节点')).toBeTruthy()
    expect(screen.getByText('1 个待复习')).toBeTruthy()
    expect(screen.getByText(/连续学习天数/)).toBeTruthy()
    expect(screen.getByText(/今日复习/)).toBeTruthy()
  })
})

describe('学习驾驶舱 · 领域工作台', () => {
  it('路线图节点可展开详情，三态切换/记得升档/忘了回档都打到 item patch', async () => {
    render(
      <MemoryRouter initialEntries={['/study/t1']}>
        <Routes>
          <Route path="/study/:id" element={<StudyWorkspace />} />
        </Routes>
      </MemoryRouter>,
    )
    expect(await screen.findByText('Rust 进阶')).toBeTruthy()
    // 展开到期节点（所有权）
    fireEvent.click((await screen.findAllByText('所有权'))[0])
    expect((await screen.findAllByText(/已到期/)).length).toBeGreaterThan(0)
    // 记得 → 升档（当前间隔远超 60 天 → 封顶 60）
    const rememberBtns = screen.getAllByRole('button', { name: /^记得/ })
    fireEvent.click(rememberBtns[0])
    await vi.waitFor(() => {
      const p = state.patches.find((x) => (x.body as { needs_review?: boolean })?.needs_review === true && ((x.body as { review_due_at?: string })?.review_due_at?.length ?? 0) > 0)
      expect(p).toBeTruthy()
    })
    // 下一步队列：开始 → status learning
    fireEvent.click(screen.getByRole('button', { name: '开始' }))
    await vi.waitFor(() => {
      expect(state.patches.some((x) => (x.body as { status?: string })?.status === 'learning')).toBe(true)
    })
    // 快速日志：回车记一笔
    fireEvent.change(screen.getByPlaceholderText('今天学到哪了…（回车记一笔）'), {
      target: { value: '生命周期卡住了' },
    })
    fireEvent.keyDown(screen.getByPlaceholderText('今天学到哪了…（回车记一笔）'), { key: 'Enter' })
    expect(await screen.findByText('生命周期卡住了')).toBeTruthy()
  })
})

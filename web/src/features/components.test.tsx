/**
 * 关键组件交互测试（Phase 6 出口标准）：
 * atoms 表格操作 / persona 版本历史与回滚 / Wiki 编辑器保存。
 * 以 Memory Atoms、PersonaView、Wiki PagesPane 的行为面为对象。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'

// ---- api mock ----
vi.mock('@/lib/api', () => {
  interface WikiPageM {
    id: string; slug: string; title: string; page_type: string; content: string
    frontmatter: Record<string, unknown>; origin: string; version: number; updated_at: string
  }
  const state: { atoms: unknown[]; persona: PersonaLike[]; history: PersonaLike[]; pages: WikiPageM[]; wikiPage?: WikiPageM; draftContent?: string } = {
    atoms: [],
    persona: [],
    history: [],
    pages: [],
  }
  const api = {
    get: vi.fn(async (p: string) => {
      if (p.startsWith('/memory/atoms')) return state.atoms
      if (p.startsWith('/memory/persona/history')) return state.history
      if (p.startsWith('/memory/persona')) return state.persona
      if (p.startsWith('/memory/scenarios')) return []
      if (p.startsWith('/wiki/pages/')) return state.wikiPage as unknown as Record<string, unknown>
      if (p.startsWith('/wiki/pages')) return state.pages
      return []
    }),
    post: vi.fn(async (p: string, _b?: unknown) => {
      if (p === '/memory/persona/rollback') {
        state.persona = state.history.slice(0, 1)
        return state.persona[0]
      }
      if (p === '/memory/atoms') return {}
      return {}
    }),
    patch: vi.fn(async (p: string) => {
      if (p.includes('/memory/atoms/')) {
        const id = p.split('/').pop()!
        state.atoms = (state.atoms as { id: string }[]).filter((a) => a.id !== id)
      }
      return {}
    }),
    put: vi.fn(async (p: string) => {
      if (p.startsWith('/wiki/pages/')) {
        const slug = decodeURIComponent(p.split('/').pop()!)
        const cur = state.pages as unknown as { slug: string; content: string; version: number }[]
        const next = cur.map((pg) =>
          pg.slug === slug ? { ...pg, content: state.draftContent ?? pg.content, version: pg.version + 1 } : pg,
        )
        state.pages = next as unknown as typeof state.pages
        const updated = next.find((pg) => pg.slug === slug)
        state.wikiPage = updated as unknown as typeof state.wikiPage
        return updated
      }
      return {}
    }),
    __state: state,
  }
  return { api }
})

interface PersonaLike {
  id: string
  aspect: string
  content: string
  version: number
  evidence_refs: unknown
  prompt_version: string | null
  created_at: string
}
interface AtomLike {
  id: string
  kind: string
  content: string
  confidence: number
  status: string
  superseded_by: string | null
  needs_review: boolean
  hit_count: number
  scenario_id: string | null
  source_refs: unknown[]
  created_at: string
}
interface WikiPageLike {
  id: string
  slug: string
  title: string
  page_type: string
  content: string
  frontmatter: Record<string, unknown>
  origin: string
  version: number
  updated_at: string
}

// 拿到 mock state 的引用（模块级单例）
import { api } from '@/lib/api'
interface MockState {
  atoms: AtomLike[]
  persona: PersonaLike[]
  history: PersonaLike[]
  pages: WikiPageLike[]
  wikiPage?: WikiPageLike
  draftContent?: string
}
const mockState = (api as unknown as { __state: MockState }).__state

import Memory from '@/features/Memory'
import Wiki from '@/features/Wiki'

const wrap = (ui: React.ReactElement) => <MemoryRouter initialEntries={['/']}>{ui}</MemoryRouter>

beforeEach(() => {
  vi.clearAllMocks()
  mockState.atoms = []
  mockState.persona = []
  mockState.history = []
  mockState.pages = []
  mockState.wikiPage = undefined
})

describe('Atoms 表格操作', () => {
  it('归档 active 原子后从列表消失（PATCH + 重取）', async () => {
    mockState.atoms = [
      { id: 'a1', kind: 'preference', content: '用户偏好简洁', confidence: 0.9, status: 'active', superseded_by: null, needs_review: false, hit_count: 2, scenario_id: null, source_refs: [], created_at: '2026-08-20T00:00:00Z' },
      { id: 'a2', kind: 'fact', content: '用户住上海', confidence: 0.85, status: 'active', superseded_by: null, needs_review: false, hit_count: 0, scenario_id: null, source_refs: [], created_at: '2026-08-20T00:01:00Z' },
    ]
    render(wrap(<Memory />))
    fireEvent.click(screen.getByRole('button', { name: '原子' }))
    const rows = await screen.findAllByTestId(/^atom-content-/)
    expect(rows).toHaveLength(2)
    fireEvent.click(screen.getByTestId('atom-archive-a2'))
    await waitFor(() => {
      expect(screen.queryByTestId('atom-content-a2')).toBeNull()
      expect(screen.getByTestId('atom-content-a1')).toBeInTheDocument()
    })
    expect(api.patch).toHaveBeenCalledWith('/memory/atoms/a2', { status: 'archived' })
  })

  it('双击进入行内编辑，Enter 保存 PATCH 新内容', async () => {
    mockState.atoms = [
      { id: 'a1', kind: 'fact', content: '旧内容', confidence: 0.9, status: 'active', superseded_by: null, needs_review: false, hit_count: 0, scenario_id: null, source_refs: [], created_at: '2026-08-20T00:00:00Z' },
    ]
    render(wrap(<Memory />))
    fireEvent.click(screen.getByRole('button', { name: '原子' }))
    const cell = await screen.findByTestId('atom-content-a1')
    fireEvent.doubleClick(cell)
    const input = screen.getByTestId('atom-edit-a1')
    fireEvent.change(input, { target: { value: '新内容' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() => {
      expect(api.patch).toHaveBeenCalledWith('/memory/atoms/a1', { content: '新内容' })
    })
  })

  it('supersede 面板：新增新事实 + 归档旧条', async () => {
    mockState.atoms = [
      { id: 'old', kind: 'fact', content: '用户住在上海', confidence: 0.9, status: 'active', superseded_by: null, needs_review: false, hit_count: 0, scenario_id: null, source_refs: [], created_at: '2026-08-20T00:00:00Z' },
    ]
    render(wrap(<Memory />))
    fireEvent.click(screen.getByRole('button', { name: '原子' }))
    await screen.findByTestId('atom-content-old')
    fireEvent.click(screen.getByTestId('atom-supersede-old'))
    const panel = screen.getByTestId('supersede-panel')
    expect(panel).toBeInTheDocument()
    fireEvent.change(screen.getByTestId('supersede-input'), { target: { value: '用户已搬到北京' } })
    fireEvent.click(screen.getByTestId('supersede-submit'))
    await waitFor(() => {
      expect(api.post).toHaveBeenCalledWith('/memory/atoms', expect.objectContaining({ content: '用户已搬到北京' }))
      expect(api.patch).toHaveBeenCalledWith('/memory/atoms/old', { status: 'archived' })
    })
  })
})

describe('Persona 版本历史与回滚', () => {
  it('展示分面，查历史，回滚到上一版本', async () => {
    mockState.persona = [
      { id: 'p2', aspect: 'identity', content: '用户居住在北京。v2', version: 2, evidence_refs: {}, prompt_version: '1', created_at: '2026-08-20T02:00:00Z' },
    ]
    mockState.history = [
      { id: 'p2', aspect: 'identity', content: '用户居住在北京。v2', version: 2, evidence_refs: {}, prompt_version: '1', created_at: '2026-08-20T02:00:00Z' },
      { id: 'p1', aspect: 'identity', content: '用户居住在上海。v1', version: 1, evidence_refs: {}, prompt_version: '1', created_at: '2026-08-20T01:00:00Z' },
    ]
    render(wrap(<Memory />))
    fireEvent.click(screen.getByRole('button', { name: '画像' }))
    await screen.findByText('用户居住在北京。v2')
    // 历史
    fireEvent.click(screen.getByRole('button', { name: '历史' }))
    await screen.findByText('用户居住在上海。v1')
    expect(screen.getByText('用户居住在上海。v1')).toBeInTheDocument()
    // 回滚（v2 > 1 显示回滚按钮；新加了 confirm 守卫）
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    fireEvent.click(screen.getByRole('button', { name: /回滚 v1/ }))
    await waitFor(() => {
      expect(api.post).toHaveBeenCalledWith('/memory/persona/rollback', { aspect: 'identity', to_version: 1 })
    })
    await screen.findByText('用户居住在上海。v1')
  })
})

describe('Wiki 编辑器保存', () => {
  it('进入编辑、改内容、保存为 human 版本（PUT + 版本递增）', async () => {
    mockState.pages = [
      { id: 'w1', slug: '向量检索', title: '向量检索', page_type: 'concept', content: '# 向量检索\n\n旧内容', frontmatter: {}, origin: 'llm', version: 1, updated_at: '2026-08-20T00:00:00Z' },
    ]
    mockState.wikiPage = mockState.pages[0]
    mockState.draftContent = '# 向量检索\n\n人工编辑后的新内容'
    render(wrap(<Wiki />))
    fireEvent.click(screen.getByRole('button', { name: '页面' }))
    await screen.findByText('向量检索')
    // 先选中页面（点卡片）再进入编辑
    fireEvent.click(screen.getByText('向量检索'))
    await screen.findByText(/v1 ·/)
    fireEvent.click(screen.getByRole('button', { name: '编辑' }))
    const tas = await screen.findAllByRole('textbox')
    const ta = tas.find((el) => (el as HTMLTextAreaElement).value.includes('旧内容')) as HTMLTextAreaElement
    expect(ta).toBeTruthy()
    expect(ta).toHaveValue('# 向量检索\n\n旧内容')
    fireEvent.change(ta, { target: { value: mockState.draftContent } })
    fireEvent.click(screen.getByRole('button', { name: /保存（人工版）/ }))
    await waitFor(() => {
      expect(api.put).toHaveBeenCalledWith(
        '/wiki/pages/%E5%90%91%E9%87%8F%E6%A3%80%E7%B4%A2',
        expect.objectContaining({ title: '向量检索', content: mockState.draftContent }),
      )
    })
  })
})

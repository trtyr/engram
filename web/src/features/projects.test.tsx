/**
 * 项目记忆列表页测试：渲染列表 / 类型筛选 / 新建。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'

// ---- api mock ----
vi.mock('@/lib/api', () => {
  interface P {
    id: string
    name: string
    type: string
    status: string
    description: string | null
    categories: string[]
    frontmatter: Record<string, unknown>
    created_at: string
    updated_at: string
  }
  const state: { types: unknown[]; rows: P[] } = {
    types: [
      { type: 'dev', label: '开发', default_categories: ['后端', '前端', '测试', '规划'] },
      { type: 'research', label: '调研', default_categories: ['待查', '线索', '资料', '结论', '疑点', '证伪'] },
    ],
    rows: [
      {
        id: 'p1',
        name: 'engram',
        type: 'dev',
        status: 'active',
        description: null,
        categories: ['后端', '前端', '测试', '规划'],
        frontmatter: {},
        created_at: '2026-09-04T00:00:00Z',
        updated_at: '2026-09-04T00:00:00Z',
      },
    ],
  }
  const api = {
    get: vi.fn(async (p: string) => {
      if (p === '/projects/types') return state.types
      if (p.startsWith('/projects')) return state.rows
      return []
    }),
    post: vi.fn(async (p: string, b?: unknown) => {
      if (p === '/projects') {
        const body = b as { name: string; type: string; description: string | null }
        const np: P = {
          id: 'p2',
          name: body.name,
          type: body.type,
          status: 'active',
          description: body.description,
          categories: ['后端', '前端', '测试', '规划'],
          frontmatter: {},
          created_at: '2026-09-04T00:00:00Z',
          updated_at: '2026-09-04T00:00:00Z',
        }
        state.rows.push(np)
        return np
      }
      return {}
    }),
    put: vi.fn(async () => ({})),
    del: vi.fn(async () => ({})),
    patch: vi.fn(async () => ({})),
    upload: vi.fn(async () => ({})),
  }
  return { api }
})

import { api } from '@/lib/api'
import Projects from '@/features/Projects'

function renderPage() {
  return render(
    <MemoryRouter>
      <Projects />
    </MemoryRouter>,
  )
}

describe('Projects 列表页', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('渲染项目列表与类型徽章', async () => {
    renderPage()
    await waitFor(() => {
      expect(screen.getByText('engram')).toBeTruthy()
    })
    expect(screen.getAllByText('开发').length).toBeGreaterThan(0)
    expect(screen.getByText('后端')).toBeTruthy()
  })

  it('新建项目调用 POST /projects', async () => {
    renderPage()
    await waitFor(() => expect(screen.getByText('engram')).toBeTruthy())

    fireEvent.change(screen.getByPlaceholderText('项目名'), { target: { value: '新项目' } })
    fireEvent.click(screen.getByRole('button', { name: '新建' }))

    await waitFor(() => {
      expect(api.post).toHaveBeenCalledWith(
        '/projects',
        expect.objectContaining({ name: '新项目', type: 'dev' }),
      )
    })
  })

  it('类型筛选触发带 ?type= 的请求', async () => {
    renderPage()
    await waitFor(() => expect(screen.getByText('engram')).toBeTruthy())

    const filter = screen.getAllByRole('combobox')[0]
    fireEvent.change(filter, { target: { value: 'dev' } })

    await waitFor(() => {
      expect(api.get).toHaveBeenCalledWith('/projects?type=dev')
    })
  })
})

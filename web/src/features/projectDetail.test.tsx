/**
 * 项目详情页测试：左树（位置 + 分类文档）+ 右内容（markdown 阅读）。
 */
import { describe, expect, it, vi } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { MemoryRouter, Route, Routes } from 'react-router-dom'

// ---- api mock ----
vi.mock('@/lib/api', () => {
  const detail = {
    id: 'p1',
    name: 'agent-memory',
    type: 'dev',
    status: 'active',
    description: null,
    categories: ['后端', '前端', '测试', '规划'],
    frontmatter: {},
    created_at: '2026-09-04T00:00:00Z',
    updated_at: '2026-09-04T00:00:00Z',
    locations: [
      {
        id: 'l1',
        project_id: 'p1',
        host: 'MacBook Pro',
        path: '~/Documents/Code/Rust/agent-memory',
        purpose: '开发',
        sort_order: 0,
        created_at: '2026-09-04T00:00:00Z',
        updated_at: '2026-09-04T00:00:00Z',
      },
    ],
    docs: [
      {
        id: 'd1',
        project_id: 'p1',
        category: '后端',
        title: 'api.md',
        content: '# API 设计',
        frontmatter: {},
        created_at: '2026-09-04T00:00:00Z',
        updated_at: '2026-09-04T00:00:00Z',
      },
      {
        id: 'd2',
        project_id: 'p1',
        category: '规划',
        title: '路线图.md',
        content: '# 路线图',
        frontmatter: {},
        created_at: '2026-09-04T00:00:00Z',
        updated_at: '2026-09-04T00:00:00Z',
      },
    ],
  }
  const api = {
    get: vi.fn(async (p: string) => {
      if (p === '/projects/p1') return detail
      return []
    }),
    post: vi.fn(async () => ({})),
    put: vi.fn(async () => ({})),
    del: vi.fn(async () => ({})),
    patch: vi.fn(async () => ({})),
    upload: vi.fn(async () => ({})),
  }
  return { api }
})

import ProjectDetail from '@/features/ProjectDetail'

function renderPage() {
  return render(
    <MemoryRouter initialEntries={['/projects/p1']}>
      <Routes>
        <Route path="/projects/:id" element={<ProjectDetail />} />
      </Routes>
    </MemoryRouter>,
  )
}

describe('ProjectDetail 详情页', () => {
  it('左树渲染位置主机与分类文档', async () => {
    renderPage()
    await waitFor(() => {
      expect(screen.getByText('agent-memory')).toBeTruthy()
    })
    // 位置主机（左树 + 概览各一处）
    expect(screen.getAllByText('MacBook Pro').length).toBeGreaterThan(0)
    // 分类（后端/规划 出现在树里，带计数前缀）
    expect(screen.getAllByText(/后端/).length).toBeGreaterThan(0)
    expect(screen.getAllByText(/规划/).length).toBeGreaterThan(0)
    // 文档标题
    expect(screen.getAllByText('api.md').length).toBeGreaterThan(0)
    expect(screen.getAllByText('路线图.md').length).toBeGreaterThan(0)
  })

  it('点击文档在右侧渲染 markdown 内容', async () => {
    renderPage()
    await waitFor(() => expect(screen.getAllByText('api.md').length).toBeGreaterThan(0))

    fireEvent.click(screen.getAllByText('api.md')[0])

    await waitFor(() => {
      expect(screen.getByText('API 设计')).toBeTruthy()
    })
  })
})

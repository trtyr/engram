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
    name: 'engram',
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
        ip: '100.74.134.42',
        host: 'MacBook Pro',
        os: 'macOS',
        path: '~/Documents/Code/Rust/engram',
        purpose: '开发',
        sort_order: 0,
        asset_id: 'a1',
        created_at: '2026-09-04T00:00:00Z',
        updated_at: '2026-09-04T00:00:00Z',
      },
    ],
    assets: [
      {
        asset_id: 'a1',
        kind: 'host',
        name: 'MacBook Air M1',
        ip: '100.74.134.42',
        os: 'macOS 26.3',
        location_id: 'l1',
        host: 'MacBook Pro',
        path: '~/Documents/Code/Rust/engram',
        purpose: '开发',
      },
    ],
    links: [
      {
        id: 'k1',
        from_project: 'p1',
        from_name: 'engram',
        to_project: 'p9',
        to_name: '母项目',
        kind: 'part_of',
        note: '属于大项目',
        created_at: '2026-09-04T00:00:00Z',
      },
    ],
    docs: [
      {
        id: 'd1',
        project_id: 'p1',
        category: '后端',
        folder: '',
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
        folder: '',
        title: '路线图.md',
        content: '# 路线图',
        frontmatter: {},
        created_at: '2026-09-04T00:00:00Z',
        updated_at: '2026-09-04T00:00:00Z',
      },
      {
        id: 'd3',
        project_id: 'p1',
        category: '后端',
        folder: '审计/wiki',
        title: 'wiki-audit.md',
        content: '# 审计',
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
      expect(screen.getByText('engram')).toBeTruthy()
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

  it('folder 树：多级子文件夹默认展开可折叠，文档挂对应层级', async () => {
    renderPage()
    await waitFor(() => expect(screen.getAllByText('api.md').length).toBeGreaterThan(0))

    // folder="审计/wiki" 默认展开：📁 审计 与子级 📁 wiki 都可见，文档挂对应层级
    expect(screen.getAllByText(/📁 审计/).length).toBeGreaterThan(0)
    await waitFor(() => expect(screen.getAllByText(/📁 wiki/).length).toBeGreaterThan(0))
    expect(screen.getAllByText('wiki-audit.md').length).toBeGreaterThan(0)

    // 点击 📁 审计 → 折叠：子级 📁 wiki 隐藏（概览页同名列表不受影响）
    fireEvent.click(screen.getAllByText(/📁 审计/)[0])
    await waitFor(() => expect(screen.queryAllByText(/📁 wiki/).length).toBe(0))

    // 再点 → 重新展开
    fireEvent.click(screen.getAllByText(/📁 审计/)[0])
    await waitFor(() => expect(screen.getAllByText(/📁 wiki/).length).toBeGreaterThan(0))
  })

  it('关系区渲染用到的资产与隶属边', async () => {
    renderPage()
    await waitFor(() => expect(screen.getByText('🔗 关系')).toBeTruthy())

    // 用到的资产（台账名来自 assets 行，不是项目里重抄的 host 文本）
    expect(screen.getByText('用到的资产（1）')).toBeTruthy()
    expect(screen.getByText('MacBook Air M1')).toBeTruthy()

    // 隶属（我属于）→ 指向母项目
    expect(screen.getByText('隶属（我属于）（1）')).toBeTruthy()
    expect(screen.getByText('母项目')).toBeTruthy()
    expect(screen.getByText(/属于大项目/)).toBeTruthy()
  })
})

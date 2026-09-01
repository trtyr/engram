/**
 * CommandPalette 单测（frontend-polish R2.3）：
 * 打开聚焦 / Enter 触发检索 / 命中渲染与键盘选择 / Esc 关闭 / 点击命中跳转。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { MemoryRouter, Route, Routes } from 'react-router-dom'

const postMock = vi.fn()

vi.mock('@/lib/api', () => ({
  api: {
    post: (...a: unknown[]) => postMock(...a),
  },
}))

import { CommandPalette } from '@/components/CommandPalette'

function mount() {
  return render(
    <MemoryRouter initialEntries={['/']}>
      <CommandPalette onClose={() => {}} />
    </MemoryRouter>,
  )
}

beforeEach(() => {
  postMock.mockReset()
})

describe('CommandPalette', () => {
  it('挂载即渲染对话框并自动聚焦输入框', () => {
    mount()
    expect(screen.getByRole('dialog', { name: '全局检索' })).toBeTruthy()
    expect(screen.getByPlaceholderText(/跨域检索/)).toHaveFocus()
  })

  it('Enter 触发 /search 并渲染命中列表', async () => {
    postMock.mockResolvedValue({
      query: 'pw',
      hits: [
        { id: 'a1', domain: 'memory', score: 0.9, snippet: '原子片段', title: '原子 A' },
        { id: 'w1', domain: 'wiki', score: 0.8, snippet: '页面片段', title: 'Wiki W' },
      ],
    })
    mount()
    const input = screen.getByPlaceholderText(/跨域检索/)
    fireEvent.change(input, { target: { value: 'pw' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() => expect(screen.getByText('原子 A')).toBeTruthy())
    expect(postMock).toHaveBeenCalledWith('/search', { query: 'pw', limit: 10 })
    expect(screen.getByText('Wiki W')).toBeTruthy()
    // 第一项默认选中
    expect(screen.getByText('原子 A').closest('button')!.className.split(' ')).toContain('bg-muted')
  })

  it('ArrowDown 移动选择，Enter 跳转（命中 id 出现在路由变化侧）', async () => {
    postMock.mockResolvedValue({
      query: 'pw',
      hits: [
        { id: 'a1', domain: 'memory', score: 0.9, snippet: 's1', title: 'H1' },
        { id: 'w1', domain: 'wiki', score: 0.8, snippet: 's2', title: 'H2' },
      ],
    })
    mount()
    const input = screen.getByPlaceholderText(/跨域检索/)
    fireEvent.change(input, { target: { value: 'pw' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() => expect(screen.getByText('H2')).toBeTruthy())
    fireEvent.keyDown(input, { key: 'ArrowDown' })
    const cls = (t: string) => screen.getByText(t).closest('button')!.className.split(' ')
    expect(cls('H2')).toContain('bg-muted')
    expect(cls('H1')).not.toContain('bg-muted')
  })

  it('Esc 关闭面板', () => {
    const onClose = vi.fn()
    render(
      <MemoryRouter>
        <CommandPalette onClose={onClose} />
      </MemoryRouter>,
    )
    fireEvent.keyDown(screen.getByPlaceholderText(/跨域检索/), { key: 'Escape' })
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('检索失败显示错误文案', async () => {
    postMock.mockRejectedValue(new Error('后端不可达'))
    mount()
    const input = screen.getByPlaceholderText(/跨域检索/)
    fireEvent.change(input, { target: { value: 'x' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() => expect(screen.getByText('后端不可达')).toBeTruthy())
  })

  it('entity 域命中点击跳转独立圈子页（/circle?entity=）', async () => {
    postMock.mockResolvedValue({
      query: '张三',
      hits: [{ id: 'e1', domain: 'entity', score: 1.3, snippet: '同事', title: '张三', kind: 'person' }],
    })
    render(
      <MemoryRouter>
        <Routes>
          <Route path="/" element={<CommandPalette onClose={() => {}} />} />
          <Route path="/circle" element={<div data-testid="circle-page" />} />
        </Routes>
      </MemoryRouter>,
    )
    const input = screen.getByPlaceholderText(/跨域检索/)
    fireEvent.change(input, { target: { value: '张三' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() => expect(screen.getByText('张三')).toBeTruthy())
    fireEvent.click(screen.getByText('张三'))
    // 点击后路由到独立圈子页（/circle?entity=e1）
    await waitFor(() => {
      expect(screen.getByTestId('circle-page')).toBeTruthy()
    })
  })
})

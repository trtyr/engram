/** 页头建库对话框（R 入口显性化）：slug 校验 / 创建调用 / 成功回调切换。 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { NewLibDialog } from './Wiki'

const mocked = vi.hoisted(() => ({ post: vi.fn() }))
vi.mock('@/lib/api', () => ({
  api: { post: mocked.post, get: vi.fn(), put: vi.fn(), del: vi.fn() },
}))

const renderDlg = (onCreated = vi.fn()) => {
  const onClose = vi.fn()
  render(<NewLibDialog onClose={onClose} onCreated={onCreated} />)
  return { onClose }
}

describe('NewLibDialog', () => {
  beforeEach(() => {
    mocked.post.mockReset()
  })

  it('非法 slug 显示校验提示且不发请求', () => {
    renderDlg()
    const slug = screen.getByLabelText('新库 slug')
    fireEvent.change(slug, { target: { value: 'Bad_Slug!' } })
    fireEvent.click(screen.getByText('创建'))
    expect(screen.getByText(/slug 需小写字母/)).toBeTruthy()
    expect(mocked.post).not.toHaveBeenCalled()
  })

  it('合法 slug 创建成功→回调带回 slug', async () => {
    mocked.post.mockResolvedValue({ slug: 'notes', name: '笔记' })
    const onCreated = vi.fn()
    renderDlg(onCreated)
    fireEvent.change(screen.getByLabelText('新库 slug'), { target: { value: 'notes' } })
    fireEvent.change(screen.getByLabelText('新库名称'), { target: { value: '笔记库' } })
    fireEvent.click(screen.getByText('创建'))
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith('notes'))
    expect(mocked.post).toHaveBeenCalledWith('/wiki/libraries', { slug: 'notes', name: '笔记库' })
  })

  it('创建失败显示错误且不回调', async () => {
    mocked.post.mockRejectedValue(new Error('slug 已存在'))
    const onCreated = vi.fn()
    renderDlg(onCreated)
    fireEvent.change(screen.getByLabelText('新库 slug'), { target: { value: 'main' } })
    fireEvent.click(screen.getByText('创建'))
    await waitFor(() => expect(screen.getByText('slug 已存在')).toBeTruthy())
    expect(onCreated).not.toHaveBeenCalled()
  })
})

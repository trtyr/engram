/**
 * ErrorBoundary 单测（P019-M3）：渲染期异常兜底出错误卡片（含重试出口），
 * 正常子树不受影响。
 */
import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import { fireEvent } from '@testing-library/react'

import { ErrorBoundary } from '@/components/ErrorBoundary'

function Boom(): never {
  throw new Error('boom-chunk-404')
}

describe('ErrorBoundary', () => {
  it('子树抛错时渲染错误卡片而非白屏', () => {
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {})
    render(
      <ErrorBoundary>
        <Boom />
      </ErrorBoundary>,
    )
    expect(screen.getByText('页面出错了')).toBeTruthy()
    expect(screen.getByText(/boom-chunk-404/)).toBeTruthy()
    expect(screen.getByRole('button', { name: '重新加载' })).toBeTruthy()
    spy.mockRestore()
  })

  it('重试渲染可清除错误态', () => {
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {})
    let boom = true
    const { rerender } = render(
      <ErrorBoundary>
        {boom ? <Boom /> : <p>恢复正常</p>}
      </ErrorBoundary>,
    )
    expect(screen.getByText('页面出错了')).toBeTruthy()
    // 先换好子树（此时仍在错误态），再点重试渲染清错误
    boom = false
    rerender(
      <ErrorBoundary>
        {boom ? <Boom /> : <p>恢复正常</p>}
      </ErrorBoundary>,
    )
    fireEvent.click(screen.getByRole('button', { name: '重试渲染' }))
    expect(screen.getByText('恢复正常')).toBeTruthy()
    spy.mockRestore()
  })
})

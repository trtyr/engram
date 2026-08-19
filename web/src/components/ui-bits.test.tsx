/** 组件测试：状态徽章 / 时间格式 / API 错误体。 */
import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import { StatusBadge, Empty, fmtTime } from '@/components/ui-bits'
import { ApiError } from '@/lib/api'

describe('StatusBadge', () => {
  it('renders known status with tone class', () => {
    render(<StatusBadge status="ready" />)
    expect(screen.getByText('ready')).toHaveClass('text-green-400')
  })
  it('unknown status falls back to gray', () => {
    render(<StatusBadge status="weird" />)
    expect(screen.getByText('weird')).toHaveClass('text-gray-400')
  })
})

describe('Empty', () => {
  it('shows hint text', () => {
    render(<Empty text="暂无数据" />)
    expect(screen.getByText('暂无数据')).toBeInTheDocument()
  })
})

describe('fmtTime', () => {
  it('formats ISO to local string', () => {
    const out = fmtTime('2026-08-19T12:00:00Z')
    expect(out).toMatch(/2026/)
  })
})

describe('ApiError', () => {
  it('carries code and retryable', () => {
    const e = new ApiError(503, 'storage_unavailable', '存储层暂时不可用', true)
    expect(e.code).toBe('storage_unavailable')
    expect(e.retryable).toBe(true)
    expect(e.message).toContain('存储层')
  })
})

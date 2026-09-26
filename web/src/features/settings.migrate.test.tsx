/**
 * 数据迁移面板测试：导出下载 / 迁移包上传导入（报告展示）/ 远程拉取表单校验。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import Settings from './Settings'

const calls: { get: string[]; post: [string, unknown][] } = { get: [], post: [] }
const downloadBlob = new Blob(['{"format":"engram-transfer"}'])

vi.mock('@/lib/api', () => {
  const api = {
    get: vi.fn(async (p: string) => {
      calls.get.push(p)
      if (p === '/settings/llm/providers') return []
      if (p === '/settings/llm/routing') return { routes: {} }
      if (p === '/settings/api-keys') return []
      if (p === '/jobs?limit=8') return []
      return {}
    }),
    post: vi.fn(async (p: string, b?: unknown) => {
      calls.post.push([p, b])
      if (p === '/migrate/import') {
        return {
          memory: { sessions: { imported: 1, skipped: 0 } },
        }
      }
      if (p === '/migrate/pull') {
        return {
          source: 'http://a-host:8080',
          imported: { memory: { sessions: { imported: 1, skipped: 0 } } },
        }
      }
      return {}
    }),
    put: vi.fn(async () => ({})),
    del: vi.fn(async () => ({})),
    download: vi.fn(async () => downloadBlob),
    upload: vi.fn(async () => ({})),
    patch: vi.fn(async () => ({})),
  }
  return { api }
})

import { api } from '@/lib/api'

describe('设置页 · 数据迁移', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    calls.get.length = 0
    calls.post.length = 0
  })

  it('进入迁移 tab：三个卡片（导出/导入/远程拉取）齐备', async () => {
    render(<Settings />)
    fireEvent.click(screen.getByRole('button', { name: '数据迁移' }))
    await waitFor(() => expect(screen.getByText('① 全系统导出')).toBeTruthy())
    expect(screen.getByText('② 导入迁移包')).toBeTruthy()
    expect(screen.getByText('③ 远程拉取（A → 本机）')).toBeTruthy()
  })

  it('导出：调用 api.download 并触发 blob 保存', async () => {
    const url = URL.createObjectURL
    const revoked: string[] = []
    URL.createObjectURL = vi.fn(() => 'blob:mock') as typeof URL.createObjectURL
    URL.revokeObjectURL = ((u: string) => revoked.push(u)) as typeof URL.revokeObjectURL
    render(<Settings />)
    fireEvent.click(screen.getByRole('button', { name: '数据迁移' }))
    fireEvent.click(await screen.findByRole('button', { name: '导出迁移包' }))
    await waitFor(() => expect(api.download).toHaveBeenCalledWith('/migrate/export'))
    expect(revoked.length).toBeGreaterThan(0)
    URL.createObjectURL = url
  })

  it('导入迁移包：解析 JSON → POST /migrate/import → 分域报告展示', async () => {
    render(<Settings />)
    fireEvent.click(screen.getByRole('button', { name: '数据迁移' }))
    const input = await screen.findByLabelText('迁移包文件')
    fireEvent.change(input, {
      target: {
        files: [new File(['{"format":"engram-transfer"}'], 't.json', { type: 'application/json' })],
      },
    })
    await waitFor(() => {
      expect(api.post).toHaveBeenCalledWith('/migrate/import', { format: 'engram-transfer' })
    })
    await waitFor(() => expect(screen.getByText('导入报告')).toBeTruthy())
    expect(screen.getByText(/"imported": 1/)).toBeTruthy()
  })

  it('远程拉取：空表单按钮禁用；填写后 POST /migrate/pull 并展示报告', async () => {
    render(<Settings />)
    fireEvent.click(screen.getByRole('button', { name: '数据迁移' }))
    const btn = await screen.findByRole('button', { name: '拉取并导入' })
    expect(btn.hasAttribute('disabled')).toBe(true)

    fireEvent.change(screen.getByLabelText('源机地址'), { target: { value: 'http://a-host:8080' } })
    fireEvent.change(screen.getByLabelText('源机管理员密码'), { target: { value: 'pw' } })
    fireEvent.click(btn)
    await waitFor(() => {
      expect(api.post).toHaveBeenCalledWith('/migrate/pull', {
        source_url: 'http://a-host:8080',
        source_admin_password: 'pw',
      })
    })
    await waitFor(() => expect(screen.getByText('导入报告')).toBeTruthy())
  })
})

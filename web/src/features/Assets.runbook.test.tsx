/**
 * 资产运维手册前端测试：Markdown 渲染 → 手册编辑保存（PUT）→ 版本史/回滚（POST restore）
 * → fields 运维字段表单（编辑+保存整体替换）。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import Assets from './Assets'

const detail = {
  id: 'a1',
  kind: 'host',
  name: 'rb-host',
  aliases: ['mac'],
  ip: '10.0.0.9',
  os: 'macOS',
  note: '',
  fields: { 规格: 'M4 Max' },
  created_at: '2026-09-26T10:00:00Z',
  updated_at: '2026-09-26T10:00:00Z',
  runbook_md: '# 主机手册\n- 32G 内存',
  used_by: [],
}

const calls: { put: [string, unknown][]; post: [string, unknown][] } = { put: [], post: [] }

vi.mock('@/lib/api', () => {
  const api = {
    get: vi.fn(async (p: string) => {
      if (p === '/assets/types') return [{ kind: 'host', label: '主机' }]
      if (p === '/assets') return [detail]
      if (p === '/assets/a1') return detail
      if (p === '/assets/a1/runbook/versions')
        return {
          versions: [
            {
              id: 'v1',
              asset_id: 'a1',
              old_runbook_md: '# 旧版',
              edited_by: 'console',
              created_at: '2026-09-26T11:00:00Z',
            },
          ],
        }
      return {}
    }),
    put: vi.fn(async (p: string, b?: unknown) => {
      calls.put.push([p, b])
      return b
    }),
    post: vi.fn(async (p: string, b?: unknown) => {
      calls.post.push([p, b])
      return {}
    }),
    del: vi.fn(async () => ({})),
  }
  return { api }
})

beforeEach(() => {
  calls.put = []
  calls.post = []
})

describe('资产详情 · 运行手册与运维字段', () => {
  it('详情渲染 Markdown 手册（标题与列表项）', async () => {
    render(<Assets />)
    fireEvent.click(await screen.findByText('rb-host'))
    expect(await screen.findByText('主机手册')).toBeTruthy()
    expect(screen.getByText(/32G 内存/)).toBeTruthy()
  })

  it('编辑手册→保存走 PUT /assets/{id}/runbook', async () => {
    render(<Assets />)
    fireEvent.click(await screen.findByText('rb-host'))
    fireEvent.click(await screen.findByRole('button', { name: '编辑手册' }))
    const ta = await screen.findByPlaceholderText(
      '# 主机手册（Markdown）——硬件 / 网络 / 服务 / 端口 / 变更 / 踩坑',
    )
    fireEvent.change(ta, { target: { value: '# 新版手册' } })
    fireEvent.click(screen.getByRole('button', { name: '保存手册' }))
    await waitFor(() =>
      expect(calls.put.some(([p, b]) => p === '/assets/a1/runbook' && (b as { md: string }).md === '# 新版手册')).toBe(
        true,
      ),
    )
  })

  it('版本史可见且可回滚（POST restore）', async () => {
    render(<Assets />)
    fireEvent.click(await screen.findByText('rb-host'))
    fireEvent.click(await screen.findByRole('button', { name: '版本史' }))
    expect(await screen.findByText(/旧版/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '滚到这版' }))
    await waitFor(() =>
      expect(calls.post.some(([p]) => p === '/assets/a1/runbook/restore')).toBe(true),
    )
  })

  it('fields 表单：编辑值后保存整体替换', async () => {
    render(<Assets />)
    fireEvent.click(await screen.findByText('rb-host'))
    fireEvent.click(await screen.findByRole('button', { name: '编辑' }))
    const v = await screen.findByDisplayValue('M4 Max')
    fireEvent.change(v, { target: { value: 'M4 Ultra' } })
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    await waitFor(() =>
      expect(
        calls.put.some(
          ([p, b]) =>
            p === '/assets/a1' && (b as { fields: { 规格: string } }).fields['规格'] === 'M4 Ultra',
        ),
      ).toBe(true),
    )
  })
})

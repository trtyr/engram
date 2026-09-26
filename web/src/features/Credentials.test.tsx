/**
 * 凭据台账页测试（EN-234 控制台治理面）：
 * 台账两条存量凭据（不含值）→ 取用流水可见 → put 换值后流水清零语义可复现。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import Credentials from './Credentials'

const state = {
  rows: [
    {
      id: 'c1',
      name: 'newapi/api_key',
      sensitive: true,
      description: 'NewAPI 网关',
      created_by: 'agent',
      created_at: '2026-09-26T08:00:00Z',
      updated_at: '2026-09-26T08:00:00Z',
      last_read_at: null,
      read_count: 1,
    },
    {
      id: 'c2',
      name: 'helm/admin-login',
      sensitive: true,
      description: null,
      created_by: 'agent',
      created_at: '2026-09-26T08:00:00Z',
      updated_at: '2026-09-26T08:00:00Z',
      last_read_at: null,
      read_count: 0,
    },
  ],
  reads: { 'newapi/api_key': [{ id: 'r1', credential_id: 'c1', reader: 'agent', read_at: '2026-09-26T09:00:00Z' }] },
  revealed: '',
}

vi.mock('@/lib/api', () => {
  const api = {
    get: vi.fn(async (p: string) => {
      if (p === '/credentials') return { items: state.rows }
      if (p.endsWith('/reads')) {
        const name = decodeURIComponent(p.split('/')[2])
        return { name, reads: state.reads[name] ?? [] }
      }
      if (p.endsWith('/value')) {
        const name = decodeURIComponent(p.split('/')[2])
        state.reads[name] = [
          ...(state.reads[name] ?? []),
          { id: `r-${Math.random()}`, credential_id: 'x', reader: 'console', read_at: '2026-09-26T09:30:00Z' },
        ]
        state.revealed = `sk-value-of-${name}`
        return { name, value: state.revealed, read_count: (state.rows.find((r) => r.name === name)?.read_count ?? 0) + 1 }
      }
      return {}
    }),
    post: vi.fn(async (p: string, b?: unknown) => {
      if (p === '/credentials') {
        const body = b as { name: string }
        state.reads[body.name] = [] // 同名换值/新写：流水清零
        return { credential: { id: 'cx', name: body.name }, hint: '同名换值：旧取用流水已清零（值变了旧痕作废）。' }
      }
      return {}
    }),
    del: vi.fn(async () => ({})),
  }
  return { api }
})

beforeEach(() => {
  state.revealed = ''
  state.reads['newapi/api_key'] = [{ id: 'r1', credential_id: 'c1', reader: 'agent', read_at: '2026-09-26T09:00:00Z' }]
})

describe('凭据台账页', () => {
  it('台账两条存量凭据可见，且列表不含值明文', async () => {
    render(<Credentials />)
    expect(await screen.findByText('newapi/api_key')).toBeTruthy()
    expect(screen.getByText('helm/admin-login')).toBeTruthy()
    expect(screen.queryByText(/sk-value-of/)).toBeNull() // 值明文不进台账
  })

  it('取用流水可见（谁/何时）', async () => {
    render(<Credentials />)
    const rows = await screen.findAllByText('取用流水')
    fireEvent.click(rows[0])
    expect(await screen.findByText(/agent/)).toBeTruthy()
    expect(screen.getByText(/console|agent/)).toBeTruthy()
  })

  it('put 换值 → 流水清零语义可复现；揭示留痕且明文仅揭示框可见', async () => {
    render(<Credentials />)
    fireEvent.change(await screen.findByPlaceholderText('名称（如 newapi/api_key）'), {
      target: { value: 'helm/admin-login' },
    })
    fireEvent.change(screen.getByPlaceholderText('值（写入即加密，永不回显于列表）'), {
      target: { value: 'new-secret' },
    })
    fireEvent.click(screen.getByRole('button', { name: '写入' }))
    expect(await screen.findByText(/旧取用流水已清零/)).toBeTruthy()

    // 换值后再开该行流水：空态文案 = 清零语义
    const toggles = screen.getAllByRole('button', { name: '取用流水' })
    fireEvent.click(toggles[1])
    expect(await screen.findByText(/无取用记录/)).toBeTruthy()

    // 揭示：明文只在揭示框，且留痕（mock 端 reads +1）
    const reveals = screen.getAllByRole('button', { name: '揭示值' })
    fireEvent.click(reveals[1])
    expect(await screen.findByText('sk-value-of-helm/admin-login')).toBeTruthy()
  })
})

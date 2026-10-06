/**
 * 凭据页测试（EN-234 治理面 · 2026-10-06 保险箱式双栏重设计）：
 * 台账两条存量凭据（不含值）→ 选中进详情 → 揭示 30s 自动遮蔽 → 复制留痕 →
 * 换值折叠区清零流水语义 → 危险区输名字删除。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import Credentials from './Credentials'

vi.mock('@/lib/utils', async (importOriginal) => {
  const orig = await importOriginal<typeof import('@/lib/utils')>()
  return { ...orig, copyText: vi.fn(async () => true) }
})

const state: {
  rows: Array<Record<string, unknown>>
  reads: Record<string, Array<{ id: string; credential_id: string; reader: string; read_at: string }>>
} = {
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
      tags: ['newapi', 'prod'],
      expires_at: null,
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
      tags: ['helm'],
      expires_at: '2020-01-01T00:00:00Z',
    },
  ],
  reads: { 'newapi/api_key': [{ id: 'r1', credential_id: 'c1', reader: 'agent', read_at: '2026-09-26T09:00:00Z' }] },
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
        return { name, value: `sk-value-of-${name}` }
      }
      return {}
    }),
    post: vi.fn(async (p: string, b?: unknown) => {
      if (p === '/credentials') {
        const body = b as { name: string }
        state.reads[body.name] = [] // 同名换值/新写：流水清零
        return { credential: { id: 'cx', name: body.name }, hint: '' }
      }
      return {}
    }),
    del: vi.fn(async () => ({})),
  }
  return { api }
})

beforeEach(() => {
  state.reads['newapi/api_key'] = [{ id: 'r1', credential_id: 'c1', reader: 'agent', read_at: '2026-09-26T09:00:00Z' }]
  state.reads['helm/admin-login'] = []
})

describe('凭据页（保险箱式双栏）', () => {
  it('台账两条存量凭据可见、含过期健康徽章与态势条，且列表不含值明文', async () => {
    render(<Credentials />)
    expect(await screen.findByText('newapi/api_key')).toBeTruthy()
    expect(screen.getByText('helm/admin-login')).toBeTruthy()
    expect(screen.getByText('已过期')).toBeTruthy() // helm 2020 到期 → 过期徽章
    expect(screen.getByText(/共/)).toBeTruthy() // 态势条
    expect(screen.queryByText(/sk-value-of/)).toBeNull() // 值明文不进台账
  })

  it('选中进详情：值默认遮蔽，揭示后明文只在保险箱内且留痕，复制走剪贴板', async () => {
    render(<Credentials />)
    fireEvent.click(await screen.findByText('newapi/api_key'))
    // 保险箱：默认遮蔽
    expect(screen.getByText('••••••••••••')).toBeTruthy()
    // 揭示 → 明文 + 自动遮蔽倒计时
    fireEvent.click(screen.getByRole('button', { name: '揭示' }))
    expect(await screen.findByText('sk-value-of-newapi/api_key')).toBeTruthy()
    expect(screen.getByText(/自动遮蔽/)).toBeTruthy()
    // 取用流水时间线：agent 的历史留痕可见
    expect((await screen.findAllByText(/agent/)).length).toBeGreaterThan(0)
    // 遮蔽按钮可收回明文
    fireEvent.click(screen.getByRole('button', { name: '遮蔽' }))
    expect(screen.getByText('••••••••••••')).toBeTruthy()
    // 复制：取值 + 剪贴板
    fireEvent.click(screen.getByRole('button', { name: '复制' }))
    await waitFor(() => {
      expect(screen.getByText(/已复制/)).toBeTruthy()
    })
  })

  it('新建弹窗写入 → 提示；换值折叠区清零流水语义可复现；危险区输名字才可删', async () => {
    render(<Credentials />)
    await screen.findByText('newapi/api_key') // 等台账加载完
    // 新建弹窗
    fireEvent.click(screen.getByRole('button', { name: /新建凭据/ }))
    fireEvent.change(screen.getByPlaceholderText('名称（如 newapi/api_key）'), {
      target: { value: 'helm/admin-login' },
    })
    fireEvent.change(screen.getByPlaceholderText('值（写入即加密，永不回显于列表）'), {
      target: { value: 'new-secret' },
    })
    fireEvent.click(screen.getByRole('button', { name: '写入' }))
    expect(await screen.findByText(/已写入「helm\/admin-login」/)).toBeTruthy()

    // 选中 helm → 换值折叠区写入新值 → 流水清零提示
    fireEvent.click(screen.getByText('helm/admin-login'))
    fireEvent.click(screen.getByText('换值（旧取用流水清零）'))
    fireEvent.change(screen.getByPlaceholderText('新值（写入即加密）'), {
      target: { value: 'another-secret' },
    })
    fireEvent.click(screen.getByRole('button', { name: '写入新值' }))
    expect(await screen.findByText(/旧取用流水已清零/)).toBeTruthy()
    expect(await screen.findByText('从未取用——流水在「揭示 / 复制」时生成。')).toBeTruthy() // 清零后的流水空态

    // 危险区：未输名字时删除禁用，输名字后解锁
    expect(screen.getByRole('button', { name: '删除' }).hasAttribute('disabled')).toBe(true)
    fireEvent.change(screen.getByPlaceholderText('helm/admin-login'), {
      target: { value: 'helm/admin-login' },
    })
    expect(screen.getByRole('button', { name: '删除' }).hasAttribute('disabled')).toBe(false)
    fireEvent.click(screen.getByRole('button', { name: '删除' }))
    expect(await screen.findByText(/已删除「helm\/admin-login」/)).toBeTruthy()
  })
})

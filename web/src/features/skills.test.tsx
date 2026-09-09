/**
 * 技能页测试（Wiki 式双栏）：目录渲染 / 新建 / 启停 / 删除确认 / 导入 / 搜索 / 版本 / 附属文件。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import Skills from './Skills'
import { GlobalConfirm } from '@/components/confirm'

// ---- api mock ----
const state = {
  rows: [
    {
      id: 's1',
      slug: 'review-pr',
      name: 'PR 审查',
      description: '审查 Rust PR 的固定流程',
      tags: ['rust', 'review'],
      enabled: true,
      source: 'manual',
      kind: 'text',
      origin: 'self',
      local_path: null,
      repo_url: null,
      created_at: '2026-09-05T00:00:00Z',
      updated_at: '2026-09-05T00:00:00Z',
    },
    {
      id: 's2',
      slug: 'deploy-check',
      name: 'Deploy Check',
      description: '部署前检查清单',
      tags: ['ops'],
      enabled: false,
      source: 'import',
      kind: 'text',
      origin: 'self',
      local_path: null,
      repo_url: null,
      created_at: '2026-09-05T00:00:00Z',
      updated_at: '2026-09-05T00:00:00Z',
    },
    {
      id: 's3',
      slug: 'local-tool',
      name: '本地工具',
      description: '脚本型：真身在本地',
      tags: [],
      enabled: true,
      source: 'mcp',
      kind: 'script',
      origin: 'both',
      local_path: '/opt/skills/local-tool',
      repo_url: 'https://github.com/x/local-tool',
      created_at: '2026-09-05T00:00:00Z',
      updated_at: '2026-09-05T00:00:00Z',
    },
  ],
  files: [
    { path: 'references/api.md', size: 128 },
    { path: 'scripts/check.py', size: 64 },
  ],
}
const calls: { get: string[]; post: [string, unknown][]; put: [string, unknown][]; del: string[] } = {
  get: [],
  post: [],
  put: [],
  del: [],
}

vi.mock('@/lib/api', () => {
  const api = {
    get: vi.fn(async (p: string) => {
      calls.get.push(p)
      if (p === '/skills/review-pr') {
        return { ...state.rows[0], content: '# 审查步骤\n1. 读 diff' }
      }
      if (p === '/skills/review-pr/files') return state.files
      if (p === '/skills/review-pr/file?path=scripts%2Fcheck.py') {
        return { path: 'scripts/check.py', content: "print('hi')" }
      }
      if (p === '/skills/review-pr/revisions') {
        return [
          {
            id: 'r1',
            skill_id: 's1',
            rev: 1,
            name: 'PR 审查',
            description: '',
            content: '初始版',
            tags: [],
            origin: 'create',
            created_at: '2026-09-05T00:00:00Z',
          },
        ]
      }
      if (p === '/skills/deploy-check') return { ...state.rows[1], content: '# 清单' }
      if (p === '/skills/deploy-check/files') return []
      if (p === '/skills/local-tool') return { ...state.rows[2], content: '# 本地真身\n由指针现读。' }
      if (p === '/skills/local-tool/files') return []
      if (p === '/skills/new-skill') {
        return { slug: 'new-skill', name: '新技能', description: '', content: '', tags: [], enabled: true, source: 'manual' }
      }
      if (p === '/skills/new-skill/files') return []
      return state.rows
    }),
    post: vi.fn(async (p: string, b?: unknown) => {
      calls.post.push([p, b])
      if (p === '/skills') return { slug: 'new-skill' }
      if (p === '/skills/import') {
        return { imported: 1, updated: 0, failed: 0, items: [{ index: 0, slug: 'new-skill', status: 'imported', error: null }] }
      }
      return {}
    }),
    put: vi.fn(async (p: string, b?: unknown) => {
      calls.put.push([p, b])
      return {}
    }),
    del: vi.fn(async (p: string) => {
      calls.del.push(p)
      return {}
    }),
    patch: vi.fn(async () => ({})),
    upload: vi.fn(async () => ({})),
  }
  return { api }
})

import { api } from '@/lib/api'

describe('Skills 技能页（双栏）', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    calls.get.length = 0
    calls.post.length = 0
    calls.put.length = 0
    calls.del.length = 0
  })

  it('左目录 + 右阅读：自动选中第一个技能并渲染 Markdown 正文', async () => {
    render(<MemoryRouter><Skills /></MemoryRouter>)
    await waitFor(() => expect(screen.getByText('Deploy Check')).toBeTruthy())
    expect(screen.getByText('PR 审查')).toBeTruthy()
    expect(screen.getByText('review-pr')).toBeTruthy()
    expect(screen.getByText('技能目录')).toBeTruthy()
    // 自动选中第一个 → 详情加载 + 正文渲染
    await waitFor(() => expect(screen.getByRole('heading', { name: 'PR 审查' })).toBeTruthy())
    expect(screen.getByText('● 启用')).toBeTruthy()
    await waitFor(() => expect(screen.getByText('读 diff')).toBeTruthy())
  })

  it('新建技能调用 POST /skills（标签拆分、空 slug 置 null）', async () => {
    render(<MemoryRouter><Skills /></MemoryRouter>)
    await waitFor(() => expect(screen.getByText('PR 审查')).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: '新建' }))
    fireEvent.change(screen.getByLabelText('技能名'), { target: { value: '新技能' } })
    fireEvent.change(screen.getByLabelText('技能 slug'), { target: { value: '' } })
    fireEvent.change(screen.getByLabelText('技能标签'), { target: { value: 'a, b' } })
    fireEvent.click(screen.getByRole('button', { name: '创建' }))

    await waitFor(() => {
      expect(api.post).toHaveBeenCalledWith(
        '/skills',
        expect.objectContaining({ name: '新技能', slug: null, tags: ['a', 'b'] }),
      )
    })
  })

  it('启停切换调用 PUT /skills/{slug}', async () => {
    render(<MemoryRouter><Skills /></MemoryRouter>)
    await waitFor(() => expect(screen.getByRole('button', { name: '停用' })).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: '停用' }))
    await waitFor(() => {
      expect(api.put).toHaveBeenCalledWith('/skills/review-pr', { enabled: false })
    })
  })

  it('目录切换选中：点 Deploy Check 后删除带应用内确认弹窗', async () => {
    render(<MemoryRouter><><Skills /><GlobalConfirm /></></MemoryRouter>)
    await waitFor(() => expect(screen.getByText('Deploy Check')).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: /Deploy Check/ }))
    await waitFor(() => expect(screen.getByRole('button', { name: '删除' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: '删除' }))
    // 应用内确认弹窗：点弹窗内的确认键才执行
    const dlg = await screen.findByRole('alertdialog')
    fireEvent.click(within(dlg).getByRole('button', { name: '删除' }))
    await waitFor(() => {
      expect(api.del).toHaveBeenCalledWith('/skills/deploy-check')
    })
  })

  it('粘贴 SKILL.md 导入调用 POST /skills/import 并展示报告', async () => {
    render(<MemoryRouter><Skills /></MemoryRouter>)
    await waitFor(() => expect(screen.getByText('PR 审查')).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: '导入' }))
    fireEvent.change(screen.getByLabelText('导入内容'), {
      target: { value: '---\nname: X\n---\nbody' },
    })
    // 第二个「导入」按钮是面板内提交（第一个是 header 的面板开关）
    fireEvent.click(screen.getAllByRole('button', { name: '导入' })[1])

    await waitFor(() => {
      expect(api.post).toHaveBeenCalledWith(
        '/skills/import',
        expect.objectContaining({ documents: [{ content: '---\nname: X\n---\nbody' }] }),
      )
    })
    await waitFor(() => expect(screen.getByText(/新建 1 · 覆盖 0 · 失败 0/)).toBeTruthy())
  })

  it('搜索框输入触发带 ?q= 的请求', async () => {
    render(<MemoryRouter><Skills /></MemoryRouter>)
    await waitFor(() => expect(screen.getByText('PR 审查')).toBeTruthy())

    fireEvent.change(screen.getByLabelText('搜索技能'), { target: { value: '审查' } })
    await waitFor(() => {
      expect(api.get).toHaveBeenCalledWith('/skills?q=%E5%AE%A1%E6%9F%A5')
    })
  })

  it('版本面板加载 revisions 并可回滚', async () => {
    render(<MemoryRouter><Skills /></MemoryRouter>)
    await waitFor(() => expect(screen.getByRole('button', { name: '版本' })).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: '版本' }))
    await waitFor(() => expect(screen.getByText('v1')).toBeTruthy())
    expect(api.get).toHaveBeenCalledWith('/skills/review-pr/revisions')

    fireEvent.click(screen.getByRole('button', { name: '回滚' }))
    await waitFor(() => {
      expect(api.post).toHaveBeenCalledWith('/skills/review-pr/revisions/r1/restore', {})
    })
  })

  it('附属文件区：索引展示 + 点击查看脚本内容 + 删除带应用内确认弹窗', async () => {
    render(<MemoryRouter><><Skills /><GlobalConfirm /></></MemoryRouter>)
    await waitFor(() => expect(screen.getByText('附属文件')).toBeTruthy())
    await waitFor(() => expect(screen.getByText('scripts/check.py')).toBeTruthy())
    expect(screen.getByText('references/api.md')).toBeTruthy()
    expect(screen.getByText('128 B')).toBeTruthy()

    // 删除文件带应用内确认弹窗（索引按 path 排序，references/api.md 是第一个）
    fireEvent.click(screen.getAllByRole('button', { name: '删除文件' })[0])
    const dlg = await screen.findByRole('alertdialog')
    fireEvent.click(within(dlg).getByRole('button', { name: '删除' }))
    await waitFor(() => {
      expect(api.del).toHaveBeenCalledWith('/skills/review-pr/file?path=references%2Fapi.md')
    })

    // 查看脚本（GET file?path=…；索引按 path 排序，scripts/check.py 是第二个）
    fireEvent.click(screen.getAllByRole('button', { name: '查看' })[1])
    await waitFor(() => expect(screen.getByText("print('hi')")).toBeTruthy())
    expect(screen.getByText('← 返回 SKILL.md')).toBeTruthy()
  })

  it('二态展示：text 型带「文本」徽标，script 型带「脚本」徽标且详情显示本地指针', async () => {
    render(<MemoryRouter><Skills /></MemoryRouter>)
    await waitFor(() => expect(screen.getByText('PR 审查')).toBeTruthy())
    // 列表徽标：两种形态都在目录里
    expect(screen.getAllByText('文本').length).toBeGreaterThanOrEqual(2)
    expect(screen.getAllByText('脚本').length).toBeGreaterThanOrEqual(1)

    // 切到 script 型技能：详情头显示本地路径 + 仓库链接，版本按钮隐藏
    fireEvent.click(screen.getByRole('button', { name: /本地工具/ }))
    await waitFor(() => expect(screen.getByRole('heading', { name: '本地工具' })).toBeTruthy())
    expect(screen.getAllByText(/\/opt\/skills\/local-tool/).length).toBeGreaterThanOrEqual(1)
    expect(screen.getByText('https://github.com/x/local-tool')).toBeTruthy()
    expect(screen.getByText('脚本型技能（本地指针）')).toBeTruthy()
    // script 型：无「版本」按钮、无附属文件管理
    expect(screen.queryByRole('button', { name: '版本' })).toBeNull()
    expect(screen.queryByRole('button', { name: '添加文件' })).toBeNull()
    // 现读正文渲染
    await waitFor(() => expect(screen.getByText('由指针现读。')).toBeTruthy())

    // 切回 text 型：版本按钮回来、附属文件区正常
    fireEvent.click(screen.getByRole('button', { name: /PR 审查/ }))
    await waitFor(() => expect(screen.getByRole('button', { name: '版本' })).toBeTruthy())
    await waitFor(() => expect(screen.getByText('附属文件')).toBeTruthy())
  })

  it('新建表单：切 script 型要求本地路径，POST body 携带 kind/origin/local_path/repo_url', async () => {
    render(<MemoryRouter><Skills /></MemoryRouter>)
    await waitFor(() => expect(screen.getByText('PR 审查')).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: '新建' }))
    fireEvent.change(screen.getByLabelText('技能名'), { target: { value: '带脚本技能' } })
    fireEvent.change(screen.getByLabelText('存储形态'), { target: { value: 'script' } })
    fireEvent.change(screen.getByLabelText('来源'), { target: { value: 'both' } })
    fireEvent.change(screen.getByLabelText('本地路径'), { target: { value: '/opt/skills/with-script' } })
    fireEvent.change(screen.getByLabelText('仓库地址'), { target: { value: 'https://github.com/x/y' } })
    fireEvent.click(screen.getByRole('button', { name: '创建' }))

    await waitFor(() => {
      expect(api.post).toHaveBeenCalledWith(
        '/skills',
        expect.objectContaining({
          name: '带脚本技能',
          kind: 'script',
          origin: 'both',
          local_path: '/opt/skills/with-script',
          repo_url: 'https://github.com/x/y',
          content: '',
        }),
      )
    })
  })
})

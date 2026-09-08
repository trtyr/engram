describe('Settings 危险区：清空记忆库确认短语门禁', () => {
  it('错短语禁用执行按钮，正确短语放行', async () => {
    const { default: Settings } = await import('./Settings')
    render(
      <MemoryRouter>
        <Settings />
      </MemoryRouter>,
    )
    fireEvent.click(screen.getByRole('button', { name: '危险操作' }))
    fireEvent.click(screen.getByRole('button', { name: '清空记忆库…' }))
    const input = await screen.findByLabelText('清空确认短语')
    const btn = screen.getByRole('button', { name: '执行清空' })
    expect(btn).toBeDisabled()
    fireEvent.change(input, { target: { value: '清空记忆' } })
    expect(btn).toBeDisabled()
    fireEvent.change(input, { target: { value: '清空记忆库' } })
    expect(btn).toBeEnabled()
  })
})

/**
 * P3 功能测试：跨域 /search、provider 编辑/删除、re-embed。
 * 以 Dashboard GlobalSearch、Settings Providers、Wiki 文档 ChunksPanel 的行为面为对象。
 */
import { GlobalConfirm } from '@/components/confirm'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react'
import { MemoryRouter, Route, Routes } from 'react-router-dom'

type WikiPageM = {
  id: string
  slug: string
  title: string
  page_type: string
  folder: string
  content: string
  frontmatter: Record<string, unknown>
  origin: string
  version: number
  updated_at: string
}

// ---- api mock ----
vi.mock('@/lib/api', () => {
  const state = {
    providers: [] as { id: string; name: string; base_url: string; model_id: string; capability: string; is_default: boolean }[],
    docs: [] as { id: string; title: string; source_uri: string; mime: string | null; status: string; error: string | null; created_at: string }[],
    chunks: [] as { seq: number; content: string; embed_failed: boolean }[],
    searchResult: null as { query: string; hits: { id: string; domain: string; score: number; snippet: string; title?: string | null }[] } | null,
    purpose: { goals: [], key_questions: [], scope: [] } as { goals: string[]; key_questions: string[]; scope: string[] },
    wikiSearchResult: null as { purpose: { goals: string[]; key_questions: string[]; scope: string[] }; pages: WikiPageM[] } | null,
    wikiPages: [] as WikiPageM[],
    reencryptResult: 0 as number,
    sessions: [] as { id: string; agent: string; content: { speaker: string; text: string; ts?: string }[]; distill_status: string; created_at: string }[],
    atoms: [] as { id: string; kind: string; content: string; confidence: number; status: string; superseded_by: string | null; needs_review: boolean; hit_count: number; scenario_id: string | null; source_refs: { session_id?: string; erased?: boolean }[]; created_at: string }[],
    usage: [] as { id: number; provider: string; model: string; purpose: string; input_tokens: number; output_tokens: number; latency_ms: number; job_id: number | null; ts: string }[],
    memorySearch: null as { entities: { id: string; title: string | null; snippet: string; score: number; kind: string | null }[]; l1: { id: string; snippet: string; score: number }[]; l2: { id: string; title: string | null; snippet: string }[]; l3: unknown[] } | null,
  }
  const api = {
    get: vi.fn(async (p: string) => {
      if (p.startsWith('/settings/llm/providers')) return state.providers
      if (p.startsWith('/wiki/documents/') && p.endsWith('/chunks')) return state.chunks
      if (p.startsWith('/wiki/documents')) return state.docs
      if (p.startsWith('/memory/sessions')) return state.sessions
      if (p.startsWith('/memory/atoms')) return state.atoms
      if (p.startsWith('/memory/entities/graph')) return { nodes: [], edges: [] }
      if (p.startsWith('/memory/entities/')) {
        return { entity: { id: 'e1', name: '张三', kind: 'person', summary: '同事，负责后端', atom_count: 0, updated_at: '2026-08-30T00:00:00Z' }, atoms: [], scenarios: [] }
      }
      if (p.startsWith('/memory/entities')) return []
      if (p.startsWith('/memory/scenarios')) return []
      if (p.startsWith('/wiki/pages/')) return state.wikiPages.find((x) => p.endsWith(`/${x.slug}`)) ?? null
      if (p.startsWith('/wiki/pages')) return state.wikiPages
      if (p === '/wiki/purpose') return state.purpose
      if (p.startsWith('/memory/persona')) return []
      if (p.startsWith('/codegraph/projects')) return []
      if (p.startsWith('/jobs')) return []
      if (p.startsWith('/llm/usage')) return state.usage
      return []
    }),
    post: vi.fn(async (p: string, _b?: unknown) => {
      if (p === '/search') return state.searchResult
      if (p === '/memory/search') return state.memorySearch
      if (p.includes('/re-embed')) return undefined
      if (p === '/wiki/search') return state.wikiSearchResult
      if (p.endsWith('/re-encrypt')) return { re_encrypted: state.reencryptResult }
      if (p === '/settings/llm/providers') {
        const b = _b as { name: string; base_url: string; model_id: string; capability: string }
        return { id: 'new1', name: b.name, base_url: b.base_url, model_id: b.model_id, capability: b.capability, is_default: false }
      }
      if (p === '/settings/llm/routing/suggest') return {}
      return {}
    }),
    patch: vi.fn(async (_p: string, _b?: unknown) => ({})),
    put: vi.fn(async (p: string) => {
      if (p === '/wiki/purpose') return undefined
      return {}
    }),
    del: vi.fn(async () => undefined),
    __state: state,
  }
  return { api }
})

import { api } from '@/lib/api'

interface MockState {
  providers: { id: string; name: string; base_url: string; model_id: string; capability: string; is_default: boolean }[]
  docs: { id: string; title: string; source_uri: string; mime: string | null; status: string; error: string | null; created_at: string }[]
  chunks: { seq: number; content: string; embed_failed: boolean }[]
  searchResult: { query: string; hits: { id: string; domain: string; score: number; snippet: string; title?: string | null }[] } | null
  purpose: { goals: string[]; key_questions: string[]; scope: string[] }
  wikiSearchResult: { purpose: { goals: string[]; key_questions: string[]; scope: string[] }; pages: WikiPageM[] } | null
  wikiPages: WikiPageM[]
  reencryptResult: number
  sessions: { id: string; agent: string; content: { speaker: string; text: string; ts?: string }[]; distill_status: string; created_at: string }[]
  atoms: { id: string; kind: string; content: string; confidence: number; status: string; superseded_by: string | null; needs_review: boolean; hit_count: number; scenario_id: string | null; source_refs: { session_id?: string; erased?: boolean }[]; created_at: string }[]
  usage: { id: number; provider: string; model: string; purpose: string; input_tokens: number; output_tokens: number; latency_ms: number; job_id: number | null; ts: string }[]
  memorySearch: { entities: { id: string; title: string | null; snippet: string; score: number; kind: string | null }[]; l1: { id: string; snippet: string; score: number }[]; l2: { id: string; title: string | null; snippet: string }[]; l3: unknown[] } | null
}

const mockState = (api as unknown as { __state: MockState }).__state

import Dashboard from '@/features/Dashboard'
import Memory from '@/features/Memory'
import Settings from '@/features/Settings'
import { DocumentsPane } from '@/features/DocumentsPane'
import Wiki from '@/features/Wiki'

const wrap = (ui: React.ReactElement) => <MemoryRouter initialEntries={['/']}>{ui}</MemoryRouter>

beforeEach(() => {
  vi.clearAllMocks()
  mockState.providers = []
  mockState.docs = []
  mockState.chunks = []
  mockState.searchResult = null
  mockState.purpose = { goals: [], key_questions: [], scope: [] }
  mockState.wikiSearchResult = null
  mockState.wikiPages = []
  mockState.reencryptResult = 0
  mockState.sessions = []
  mockState.atoms = []
  mockState.usage = []
  mockState.memorySearch = null
})

describe('Dashboard 概览：管线主视觉 + 用量图', () => {
  it('L0-L3 大数字与近7天增量来自 sessions/atoms 数据', async () => {
    const now = new Date().toISOString()
    mockState.sessions = [
      { id: 's1', agent: 'a', content: [], distill_status: 'completed', created_at: now },
      { id: 's2', agent: 'a', content: [], distill_status: 'completed', created_at: now },
      { id: 's3', agent: 'a', content: [], distill_status: 'completed', created_at: '2026-01-01T00:00:00Z' },
    ]
    mockState.atoms = [
      { id: 'a1', kind: 'preference', content: 'x', confidence: 1, status: 'active', superseded_by: null, needs_review: false, hit_count: 0, scenario_id: null, source_refs: [], created_at: now },
      { id: 'a2', kind: 'preference', content: 'x', confidence: 1, status: 'superseded', superseded_by: 'a1', needs_review: false, hit_count: 0, scenario_id: null, source_refs: [], created_at: '2026-01-01T00:00:00Z' },
    ]
    render(wrap(<Dashboard />))
    await waitFor(() => {
      // L0 = 3 会话，增量 +2（一条在 7 天外）；L1 = 1 活跃原子（superseded 不计），增量 +1
      expect(screen.getByText('3').closest('button')).toHaveAttribute('aria-label', '会话 3，跳转到记忆 会话')
      expect(screen.getByText('1').closest('button')).toHaveAttribute('aria-label', '原子 1，跳转到记忆 原子')
      expect(screen.getAllByText('+2 近7天').length).toBeGreaterThanOrEqual(1)
      expect(screen.getAllByText('+1 近7天').length).toBeGreaterThanOrEqual(1)
    })
  })

  it('用量行按日分桶渲染 30 根柱，今日柱有明细 title', async () => {
    const today = new Date().toISOString().slice(0, 10)
    mockState.usage = [
      { id: 1, provider: 'p', model: 'm', purpose: 'extract', input_tokens: 800, output_tokens: 200, latency_ms: 100, job_id: null, ts: `${today}T10:00:00Z` },
    ]
    render(wrap(<Dashboard />))
    await waitFor(() => {
      const rects = document.querySelectorAll('svg[role="img"] rect')
      expect(rects.length).toBe(30)
    })
    const todayBar = document.querySelector('svg[role="img"] rect:last-of-type title')
    expect(todayBar?.textContent).toContain('1,000 tokens')
    expect(todayBar?.textContent).toContain('1 次调用')
  })
})

describe('Memory 检索面板（?tab=search 深链直达，tab 条已收敛入全局 palette）', () => {
  it('实体段先于原子段渲染，点击直达圈子页', async () => {
    mockState.memorySearch = {
      entities: [{ id: 'e1', title: '张三', snippet: '同事，负责后端', score: 1.3, kind: 'person' }],
      l1: [],
      l2: [],
      l3: [],
    }
    window.history.pushState({}, '', '/memory?tab=search')
    render(
      <MemoryRouter initialEntries={['/memory?tab=search']}>
        <Routes>
          <Route path="/memory" element={<Memory />} />
          <Route path="/circle" element={<div data-testid="circle-page" />} />
        </Routes>
      </MemoryRouter>,
    )
    await screen.findByPlaceholderText('中文检索记忆…')
    fireEvent.change(screen.getByPlaceholderText('中文检索记忆…'), { target: { value: '张三' } })
    fireEvent.click(screen.getByRole('button', { name: '检索' }))
    await waitFor(() => {
      expect(api.post).toHaveBeenCalledWith('/memory/search', { query: '张三', max_items: 10 })
      expect(screen.getByText('张三')).toBeInTheDocument()
    })
    fireEvent.click(screen.getByText('张三'))
    // 实体命中 → 路由到独立圈子页（/circle?entity=），MemoryRouter 内以标记路由断言
    await waitFor(() => {
      expect(screen.getByTestId('circle-page')).toBeInTheDocument()
    })
  })

  it('L1 命中「查看」跳原子 tab', async () => {
    mockState.memorySearch = {
      entities: [],
      l1: [{ id: 'a1', snippet: '命中片段', score: 0.912 }],
      l2: [],
      l3: [],
    }
    window.history.pushState({}, '', '/memory?tab=search')
    render(wrap(<Memory />))
    await screen.findByPlaceholderText('中文检索记忆…')
    fireEvent.change(screen.getByPlaceholderText('中文检索记忆…'), { target: { value: '命中' } })
    fireEvent.click(screen.getByRole('button', { name: '检索' }))
    await waitFor(() => {
      expect(screen.getByText('查看')).toBeInTheDocument()
    })
    fireEvent.click(screen.getByText('查看'))
    await waitFor(() => {
      expect(screen.getByRole('button', { name: '原子' }).getAttribute('aria-pressed')).toBe('true')
    })
  })
})

describe('人审队列', () => {
  it('通过清 needs_review 后行消失；批量通过逐条 PATCH', async () => {
    mockState.atoms = [
      { id: 'r1', kind: 'fact', content: '低置信事实甲', confidence: 0.5, status: 'candidate', superseded_by: null, needs_review: true, hit_count: 0, scenario_id: null, source_refs: [], created_at: new Date().toISOString() },
      { id: 'r2', kind: 'fact', content: '低置信事实乙', confidence: 0.52, status: 'candidate', superseded_by: null, needs_review: true, hit_count: 0, scenario_id: null, source_refs: [], created_at: new Date().toISOString() },
    ]
    render(wrap(<Memory />))
    fireEvent.click(screen.getByRole('button', { name: '人审' }))
    await screen.findByText('2 待审')
    // 单条通过
    fireEvent.click(screen.getAllByRole('button', { name: '通过' })[0])
    await waitFor(() => {
      expect(api.patch).toHaveBeenCalledWith('/memory/atoms/r1', { needs_review: false })
      expect(screen.getByText('1 待审')).toBeInTheDocument()
    })
    // 勾选剩余一条 → 批量通过
    fireEvent.click(screen.getByRole('checkbox', { name: /选中 低置信事实乙/ }))
    fireEvent.click(screen.getByRole('button', { name: '批量通过' }))
    await waitFor(() => {
      expect(api.patch).toHaveBeenCalledWith('/memory/atoms/r2', { needs_review: false })
    })
  })
})

describe('Provider 编辑 / 删除', () => {
  const p1 = {
    id: 'p1',
    name: 'openai',
    base_url: 'https://api.openai.com/v1',
    model_id: 'gpt-4',
    capability: 'chat',
    is_default: true,
  }

  it('编辑：填表单后 PUT（name 不可改，key 留空不传）', async () => {
    mockState.providers = [p1]
    render(wrap(<Settings />))
    fireEvent.click(screen.getByRole('button', { name: '供应商' }))
    await screen.findByText('openai')
    fireEvent.click(screen.getByRole('button', { name: '编辑' }))
    const baseInput = screen.getByLabelText('Base URL（OpenAI 兼容）') as HTMLInputElement
    expect(baseInput.value).toBe('https://api.openai.com/v1')
    fireEvent.click(screen.getByRole('button', { name: '保存修改' }))
    await waitFor(() => {
      expect(api.put).toHaveBeenCalledWith(
        '/settings/llm/providers/p1',
        expect.objectContaining({ base_url: 'https://api.openai.com/v1' }),
      )
      // key 留空 → 不应出现在 body
      const body = (api.put as ReturnType<typeof vi.fn>).mock.calls[0][1] as Record<string, unknown>
      expect(body).not.toHaveProperty('api_key')
    })
  })

  it('删除：确认后 DELETE', async () => {
    mockState.providers = [p1]
    render(wrap(<><Settings /><GlobalConfirm /></>))
    fireEvent.click(screen.getByRole('button', { name: '供应商' }))
    await screen.findByText('openai')
    fireEvent.click(screen.getByRole('button', { name: '删除' }))
    // 应用内确认弹窗（非浏览器原生 confirm）：点弹窗内的确认键才执行
    const dlg = await screen.findByRole('alertdialog')
    fireEvent.click(within(dlg).getByRole('button', { name: '删除' }))
    await waitFor(() => {
      expect(api.del).toHaveBeenCalledWith('/settings/llm/providers/p1')
    })
  })
})

describe('AI 功能页全景', () => {
  const p1 = {
    id: 'p1',
    name: 'openai',
    base_url: 'https://api.openai.com/v1',
    model_id: 'gpt-4',
    capability: 'chat',
    is_default: true,
  }

  it('默认 tab 展示 8 个 AI 功能点，嵌入功能点无向量供应商时提示去注册', async () => {
    mockState.providers = [p1]
    render(wrap(<Settings />))
    // 默认 tab 即「AI 功能」，8 个功能点都在
    await screen.findByText('抽取')
    expect(screen.getByText('嵌入')).toBeInTheDocument()
    expect(screen.getByText('画像')).toBeInTheDocument()
    expect(screen.getByText('Wiki 生成')).toBeInTheDocument()
    // 只有 chat provider → 嵌入功能点无匹配，提示去「供应商」注册
    expect(screen.getByText('无匹配供应商（去「供应商」注册）')).toBeInTheDocument()
  })

  it('供应商 tab 类型下拉可注册向量供应商（capability=embedding）', async () => {
    mockState.providers = []
    render(wrap(<Settings />))
    fireEvent.click(screen.getByRole('button', { name: '供应商' }))
    fireEvent.click(screen.getByRole('button', { name: '注册供应商' }))
    fireEvent.change(screen.getByLabelText('名称'), { target: { value: 'embed-gw' } })
    fireEvent.change(screen.getByLabelText('Base URL（OpenAI 兼容）'), { target: { value: 'https://embed.example.com' } })
    fireEvent.change(screen.getByLabelText(/API Key/), { target: { value: 'sk-embed' } })
    fireEvent.change(screen.getByLabelText('模型 ID'), { target: { value: 'bge-m3' } })
    fireEvent.change(screen.getByLabelText('类型'), { target: { value: 'embedding' } })
    fireEvent.click(screen.getByRole('button', { name: '注册 Provider' }))
    await waitFor(() => {
      expect(api.post).toHaveBeenCalledWith(
        '/settings/llm/providers',
        expect.objectContaining({ capability: 'embedding', model_id: 'bge-m3' }),
      )
    })
  })
})

describe('Wiki 文档 re-embed', () => {
  it('有嵌入失败块时显示重嵌按钮并 POST re-embed', async () => {
    mockState.docs = [
      { id: 'd1', title: 'doc', source_uri: '', mime: null, status: 'ready', error: null, created_at: '2026-08-20T00:00:00Z' },
    ]
    mockState.chunks = [
      { seq: 1, content: '块1', embed_failed: true },
      { seq: 2, content: '块2', embed_failed: false },
    ]
    render(wrap(<DocumentsPane />))
    // 主从版式：目录项 + 阅读区标题都显示文档名
    await screen.findAllByText('doc')
    // 主从版式：首篇自动选中，阅读区直接可见
    await screen.findByText('1 个分块嵌入失败（FTS 降级）')
    fireEvent.click(screen.getByRole('button', { name: '重嵌缺失块' }))
    await waitFor(() => {
      expect(api.post).toHaveBeenCalledWith('/wiki/documents/d1/re-embed')
    })
  })
})

describe('Wiki 目标（purpose）', () => {
  it('读 purpose 展示三栏，保存调 PUT', async () => {
    mockState.purpose = { goals: ['构建知识库'], key_questions: ['什么？'], scope: ['Rust'] }
    render(wrap(<Wiki />))
    fireEvent.click(screen.getByRole('button', { name: '运维' }))
    fireEvent.click(screen.getByRole('button', { name: '目标' }))
    await screen.findByText('构建知识库')
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    await waitFor(() => {
      expect(api.put).toHaveBeenCalledWith(
        '/wiki/purpose',
        expect.objectContaining({ goals: ['构建知识库'], key_questions: ['什么？'], scope: ['Rust'] }),
      )
    })
  })
})

describe('Wiki 目录树', () => {
  it('按 folder 分组渲染页面，根页面 + 多级文件夹可见', async () => {
    mockState.wikiPages = [
      { id: 'w1', slug: 'tokio', title: 'Tokio', page_type: 'concept', folder: '技术/Rust', content: '# Tokio', frontmatter: {}, origin: 'llm', version: 1, updated_at: '2026-08-20T00:00:00Z' },
      { id: 'w2', slug: 'root-page', title: '根页面', page_type: 'concept', folder: '', content: '', frontmatter: {}, origin: 'llm', version: 1, updated_at: '2026-08-20T00:00:00Z' },
    ]
    render(wrap(<Wiki />))
    // 目录树：folder 层级（技术 → Rust）+ 页面标题 + 根页面
    await screen.findByText('Tokio')
    expect(screen.getByText('技术')).toBeInTheDocument()
    expect(screen.getByText('Rust')).toBeInTheDocument()
    expect(screen.getByText('根页面')).toBeInTheDocument()
  })
})

describe('主密钥重加密', () => {
  it('输入旧密钥 POST re-encrypt 并展示结果', async () => {
    mockState.reencryptResult = 2
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    render(wrap(<Settings />))
    fireEvent.click(screen.getByRole('button', { name: '危险操作' }))
    const input = await screen.findByPlaceholderText('openssl rand -hex 32 的旧值')
    fireEvent.change(input, { target: { value: 'aabbccdd' } })
    fireEvent.click(screen.getByRole('button', { name: '执行重加密' }))
    await waitFor(() => {
      expect(api.post).toHaveBeenCalledWith('/settings/llm/providers/re-encrypt', { old_master_key: 'aabbccdd' })
      expect(screen.getByText('已重加密 2 个 provider')).toBeInTheDocument()
    })
  })
})

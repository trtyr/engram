/**
 * API client：认证、错误体统一、token 管理。
 * token 三处可能：admin 会话（ams_）/api key（amk_，供开发调试）。
 */

const BASE = import.meta.env.DEV ? '' : ''

export class ApiError extends Error {
  code: string
  retryable: boolean
  status: number
  constructor(status: number, code: string, message: string, retryable: boolean) {
    super(message)
    this.status = status
    this.code = code
    this.retryable = retryable
  }
}

export function getToken(): string | null {
  return localStorage.getItem('am_token')
}
export function setToken(t: string) {
  localStorage.setItem('am_token', t)
}
export function clearToken() {
  localStorage.removeItem('am_token')
}

/** 登出：删掉当前后端会话（失败不阻塞本地登出——token 照旧清除）。 */
export function logoutSession(): void {
  const t = getToken()
  if (!t) return
  fetch(`${BASE}/auth/logout`, {
    method: 'POST',
    headers: { authorization: `Bearer ${t}` },
  }).catch(() => {})
}

async function req<T>(method: string, path: string, body?: unknown, raw = false): Promise<T> {
  const headers: Record<string, string> = {}
  if (getToken()) headers.authorization = `Bearer ${getToken()}`
  if (body !== undefined) headers['content-type'] = 'application/json'
  const resp = await fetch(`${BASE}${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  if (resp.status === 401) {
    // 会话中途失效 → 广播全局事件（App 监听后回登录页）。登录接口的密码错误
    // 不带 token，不会触发广播。
    const hadToken = !!getToken()
    clearToken()
    if (hadToken && typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent('engram-auth-expired'))
    }
    throw new ApiError(401, 'unauthorized', '未认证', false)
  }
  if (!resp.ok) {
    let msg = `HTTP ${resp.status}`
    let code = 'unknown'
    let retryable = false
    try {
      const e = await resp.json()
      msg = e.error?.message ?? msg
      code = e.error?.code ?? code
      retryable = e.error?.retryable ?? false
    } catch { /* 保底 */ }
    throw new ApiError(resp.status, code, msg, retryable)
  }
  if (raw) return (await resp.blob()) as T
  if (resp.status === 204) return undefined as T
  // 202 Accepted 常带实体 body（如 /memory/distill 返回入队的 Job[]）——有则解析，无则 undefined
  if (resp.status === 202) {
    try {
      return (await resp.json()) as T
    } catch {
      return undefined as T
    }
  }
  return (await resp.json()) as T
}

export const api = {
  get: <T>(p: string) => req<T>('GET', p),
  post: <T>(p: string, b?: unknown) => req<T>('POST', p, b),
  put: <T>(p: string, b?: unknown) => req<T>('PUT', p, b),
  patch: <T>(p: string, b?: unknown) => req<T>('PATCH', p, b),
  del: <T>(p: string) => req<T>('DELETE', p),
  /** 二进制下载（带鉴权；401 走统一过期事件）。返回 blob 交调用方触发保存。 */
  download: async (p: string): Promise<Blob> => {
    const headers: Record<string, string> = {}
    if (getToken()) headers.authorization = `Bearer ${getToken()}`
    const resp = await fetch(`${BASE}${p}`, { headers })
    if (resp.status === 401) {
      const hadToken = !!getToken()
      clearToken()
      if (hadToken) window.dispatchEvent(new CustomEvent('engram-auth-expired'))
      throw new ApiError(401, 'unauthorized', '未认证', false)
    }
    if (!resp.ok) {
      let msg = `HTTP ${resp.status}`
      try {
        const e = await resp.json()
        msg = e.error?.message ?? msg
      } catch { /* 保底 */ }
      throw new ApiError(resp.status, 'download_failed', msg, false)
    }
    return resp.blob()
  },
  upload: async <T>(p: string, file: File): Promise<T> => {
    const fd = new FormData()
    fd.append('file', file)
    const headers: Record<string, string> = {}
    if (getToken()) headers.authorization = `Bearer ${getToken()}`
    const resp = await fetch(`${BASE}${p}`, { method: 'POST', headers, body: fd })
    if (resp.status === 401) {
      const hadToken = !!getToken()
      clearToken()
      if (hadToken) {
        window.dispatchEvent(new CustomEvent('engram-auth-expired'))
      }
      throw new ApiError(401, 'unauthorized', '未认证', false)
    }
    if (!resp.ok) {
      let msg = `HTTP ${resp.status}`
      let code = 'upload_failed'
      let retryable = false
      try {
        const e = await resp.json()
        msg = e.error?.message ?? msg
        code = e.error?.code ?? code
        retryable = e.error?.retryable ?? false
      } catch { /* 保底 */ }
      throw new ApiError(resp.status, code, msg, retryable)
    }
    return (await resp.json()) as T
  },
  // 覆盖 BASE（测试注入）
  setBase(b: string) {
    ;(req as unknown as { base: string }).base = b
  },
}

// ---- 域类型（与后端 DTO 对齐；OpenAPI 生成流水线在 Phase 6a CI 落地后切换） ----

export interface Session {
  id: string
  agent: string
  content: { speaker: string; text: string; ts?: string }[]
  distill_status: string
  sensitive: boolean
  created_at: string
}
export interface Atom {
  id: string
  kind: string
  content: string
  confidence: number
  status: string
  superseded_by: string | null
  needs_review: boolean
  sensitive?: boolean
  hit_count: number
  scenario_id: string | null
  source_refs: { session_id?: string; erased?: boolean }[]
  created_at: string
}
export interface Scenario {
  id: string
  topic: string
  summary: string
  atom_refs: string[]
  version: number
  updated_at: string
}
export interface Persona {
  id: string
  aspect: string
  content: string
  evidence_refs: unknown
  version: number
  prompt_version: string | null
  manually_edited?: boolean
  created_at: string
}
export interface Job {
  id: string
  kind: string
  status: string
  attempts: number
  error: string | null
  progress: unknown
  payload?: Record<string, unknown>
  created_at: string
}
export interface JobEvent {
  id: number
  job_id: string
  ts: string
  level: string
  message: string
  data: unknown
}
export interface Document {
  id: string
  title: string
  source_uri: string
  mime: string | null
  status: string
  error: string | null
  created_at: string
  updated_at: string
}
export interface ChunkHit {
  chunk_id: string
  document_id: string
  document_title: string
  seq: number
  snippet: string
  score: number
  embed_failed: boolean
}
export interface EntityRevision {
  id: string
  entity_id: string
  old_summary: string
  edited_by: string
  created_at: string
}
export interface TimelineEvent {
  id: string
  at: string
  kind: string
  content: string
}
export interface SearchHit {
  id: string
  score: number
  title?: string | null
  snippet: string
  kind?: string | null
  needs_review?: boolean | null
}
export interface EntityNode {
  id: string
  name: string
  kind: string
  summary: string
  atom_count: number
  manually_edited?: boolean
  updated_at: string
}
export interface GraphEdge {
  a: string
  b: string
  weight: number
}
export interface EntityGraph {
  nodes: EntityNode[]
  edges: GraphEdge[]
  relations: EntityRelation[]
}
export interface EntityRelation {
  id: string
  from_id: string
  to_id: string
  rel_type: string
  weight: number
  source: string
  created_at: string
  updated_at: string
}
export interface EntityDetail {
  entity: EntityNode
  atoms: Atom[]
  scenarios: Scenario[]
  neighbors: EntityNode[]
  relations: EntityRelation[]
}
export interface WikiPage {
  id: string
  slug: string
  title: string
  page_type: string
  /** 目录树层级（/ 分隔多级，Obsidian 式文件夹） */
  folder: string
  content: string
  frontmatter: Record<string, unknown>
  origin: string
  version: number
  updated_at: string
}
export interface GraphDto {
  nodes: { slug: string; title: string; page_type: string; community?: number }[]
  edges: { from_slug: string; to_slug: string; weight: number }[]
  communities?: { id: number; top_slug: string; size: number; cohesion: number; sparse: boolean }[]
}
export interface LintReport {
  issues: { rule: string; slug: string; detail: string }[]
  checked_pages: number
}
export interface CgProject {
  id: string
  name: string
  path: string
  source_uri: string
  status: string
  stats: CgStats | null
  error: string | null
  created_at: string
  last_synced_at: string | null
}
export interface CgStats {
  files?: number
  symbols?: number
  edges?: number
  by_kind?: Record<string, number>
  last_indexed?: string | null
}
/** CLI 可用性（GET /codegraph/status） */
export interface CgCliStatus {
  available: boolean
  version: string | null
  pin: string
}
/** 待办（第七域，GET /todos） */
export interface Todo {
  id: string
  /** 全局单调短号（显示为 EN-<n>，人类可读引用） */
  short_no: number
  title: string
  body: string
  /** todo=行动项 / ticket=工单（0041） */
  kind: 'todo' | 'ticket'
  status: string
  priority: 'low' | 'normal' | 'high'
  /** 工单严重度（仅 kind=ticket） */
  severity: 'P0' | 'P1' | 'P2' | 'P3' | null
  symptom: string
  reproduce: string
  acceptance: string
  resolution: string
  tags: string[]
  due_at: string | null
  project_hint: string | null
  done_at: string | null
  resolved_at: string | null
  created_at: string
  updated_at: string
}
/** 管理员活跃会话（GET /auth/sessions） */
export interface AdminSessionDto {
  id: string
  created_at: string
  expires_at: string
  last_used_at: string | null
  ip: string | null
  user_agent: string | null
  current: boolean
}
/** 调用图归一结果（GET /codegraph/projects/{id}/graph）。mode: symbol=符号子图 / files=文件级全图 */
export interface CgGraph {
  mode?: 'symbol' | 'files'
  symbol?: string
  center?: string
  callers?: number
  callees?: number
  files?: number
  nodes: { id: string; name: string; kind: string; role: string; filePath?: string; line?: number }[]
  edges: { from: string; to: string; rel: string; weight?: number }[]
}
export interface Provider {
  id: string
  name: string
  base_url: string
  model_id: string
  capability: string
  is_default: boolean
  warning?: string | null
}
export interface UsageRow {
  id: number
  job_id?: string | null
  provider: string
  model: string
  purpose: string
  input_tokens: number
  output_tokens: number
  latency_ms: number
  ts: string
}
export interface ApiKey {
  id: string
  name: string
  key_prefix: string
  scopes: string[]
  created_at: string
  last_used_at: string | null
  revoked_at: string | null
  expires_at: string | null
}
export interface McpActionInfo {
  action: string
  summary: string
  destructive: boolean
  /** 参数 JSON Schema（与 help 手册同源） */
  parameters: Record<string, unknown>
  /** 是否已被停用（disabled_tools 里的 域.action） */
  disabled: boolean
}
export interface McpToolInfo {
  name: string
  domain: string
  description: string
  read_only: boolean | null
  destructive: boolean | null
  /** 参数 JSON Schema（与 tools/list 的 inputSchema 同源） */
  parameters: Record<string, unknown>
  /** 域内操作（渐进式发现；非域工具为空表） */
  actions: McpActionInfo[]
}
export interface McpInfo {
  endpoint: string
  protocol_version: string
  server_name: string
  server_version: string
  enabled: boolean
  disabled_tools: string[]
  instructions: string
  tools: McpToolInfo[]
}
export interface UnifiedHit {
  id: string
  domain: string
  score: number
  snippet: string
  title?: string | null
  extra?: Record<string, unknown>
}
export interface SearchResponse {
  query: string
  hits: UnifiedHit[]
}
export interface Purpose {
  goals: string[]
  key_questions: string[]
  scope: string[]
}
export interface WikiSearchResponse {
  purpose: Purpose
  pages: WikiPage[]
}

// ---- 项目记忆域（第五域） ----
export interface ProjectDto {
  id: string
  name: string
  type: string
  status: string
  description: string | null
  categories: string[]
  frontmatter: Record<string, unknown>
  created_at: string
  updated_at: string
}
export interface ProjectLocationDto {
  id: string
  project_id: string
  ip: string
  host: string
  os: string
  path: string
  purpose: string | null
  sort_order: number
  created_at: string
  updated_at: string
}
export interface ProjectDocDto {
  id: string
  project_id: string
  category: string
  /** 子文件夹相对路径（/ 分隔，'' = 分类根下；树形呈现 = category → folder → 文档） */
  folder: string
  title: string
  content: string
  frontmatter: Record<string, unknown>
  created_at: string
  updated_at: string
}
export interface ProjectDetailDto extends ProjectDto {
  locations: ProjectLocationDto[]
  docs: ProjectDocDto[]
}
/** 项目文件（非 markdown 制品：架构图 HTML/配置等；0045）。 */
export interface ProjectFileDto {
  id: string
  project_id: string
  name: string
  /** 渲染契约：text/html → iframe sandbox 查看器，text/markdown → WikiMarkdown，其余 <pre> */
  mime: string
  content: string
  version: number
  created_at: string
  updated_at: string
}

export interface ProjectTypeDto {
  type: string
  label: string
  default_categories: string[]
}

// ---- 技能域（第六域） ----

/** 技能附属文件索引条目（folder 形态：scripts/ / references/…，不含内容） */
export interface SkillFileInfoDto {
  path: string
  size: number
}
/** 技能附属文件（含内容） */
export interface SkillFileEntryDto {
  path: string
  content: string
}
export interface SkillSummaryDto {
  id: string
  slug: string
  name: string
  description: string
  tags: string[]
  enabled: boolean
  source: string
  /** 存储形态：text=整体入库 / script=脚本存本地、库中只存指针 */
  kind: string
  /** 来源：self=自建 / github=源自 GitHub / both=自建且已发布 */
  origin: string
  /** script 型指针：本地技能文件夹路径 */
  local_path: string | null
  /** GitHub 仓库地址（origin 含 github 时） */
  repo_url: string | null
  created_at: string
  updated_at: string
}
export interface SkillDto extends SkillSummaryDto {
  content: string
}
export interface SkillRevisionDto {
  id: string
  skill_id: string
  rev: number
  name: string
  description: string
  content: string
  tags: string[]
  origin: string
  created_at: string
}
export interface SkillImportItem {
  index: number
  slug: string | null
  status: string
  error: string | null
}
export interface SkillImportReport {
  imported: number
  updated: number
  failed: number
  items: SkillImportItem[]
}

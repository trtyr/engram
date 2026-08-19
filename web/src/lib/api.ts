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
    clearToken()
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
  return (await resp.json()) as T
}

export const api = {
  get: <T>(p: string) => req<T>('GET', p),
  post: <T>(p: string, b?: unknown) => req<T>('POST', p, b),
  put: <T>(p: string, b?: unknown) => req<T>('PUT', p, b),
  patch: <T>(p: string, b?: unknown) => req<T>('PATCH', p, b),
  del: <T>(p: string) => req<T>('DELETE', p),
  upload: async <T>(p: string, file: File): Promise<T> => {
    const fd = new FormData()
    fd.append('file', file)
    const headers: Record<string, string> = {}
    if (getToken()) headers.authorization = `Bearer ${getToken()}`
    const resp = await fetch(`${BASE}${p}`, { method: 'POST', headers, body: fd })
    if (!resp.ok) {
      let msg = `HTTP ${resp.status}`
      try {
        const e = await resp.json()
        msg = e.error?.message ?? msg
      } catch { /* 保底 */ }
      throw new ApiError(resp.status, 'upload_failed', msg, false)
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
  created_at: string
}
export interface Job {
  id: string
  kind: string
  status: string
  attempts: number
  error: string | null
  progress: unknown
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
export interface WikiPage {
  id: string
  slug: string
  title: string
  page_type: string
  content: string
  frontmatter: Record<string, unknown>
  origin: string
  version: number
  updated_at: string
}
export interface GraphDto {
  nodes: { slug: string; title: string; page_type: string }[]
  edges: { from_slug: string; to_slug: string; weight: number }[]
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
  stats: Record<string, number> | null
  error: string | null
  last_synced_at: string | null
}
export interface Provider {
  id: string
  name: string
  base_url: string
  models: { id: string; capabilities: string[] }[]
  is_default: boolean
}
export interface UsageRow {
  id: number
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
}

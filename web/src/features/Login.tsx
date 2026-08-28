/**
 * 登录页 ——「记忆星座」主题。
 * 深色画布 + 稀疏记忆节点 + 品牌星座标记 + 单字段登录表单。
 */
import { useState } from 'react'
import type { FormEvent } from 'react'
import { Eye, EyeOff, KeyRound, Loader2 } from 'lucide-react'
import { ApiError, api, setToken } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { BrandMark } from '@/components/ui-bits'

/** 背景星座：稀疏的记忆节点与连接线，纯氛围，中心留白给表单。 */
function Constellation() {
  const nodes: [number, number, number][] = [
    [118, 138, 3], [248, 92, 2], [388, 206, 2.5], [538, 124, 2], [700, 268, 2],
    [858, 146, 3], [1010, 88, 2], [1180, 186, 2.5], [1332, 116, 2],
    [104, 436, 2], [216, 534, 2.5], [1262, 416, 2], [1362, 576, 3],
    [176, 776, 2], [576, 826, 2], [938, 788, 2.5], [1242, 738, 2], [700, 660, 2],
  ]
  const edges: [number, number][] = [
    [0, 1], [1, 2], [2, 3], [4, 5], [5, 6], [6, 7], [7, 8],
    [9, 10], [11, 12], [13, 14], [14, 15], [15, 16],
  ]
  return (
    <svg
      viewBox="0 0 1440 900"
      preserveAspectRatio="xMidYMid slice"
      aria-hidden="true"
      className="pointer-events-none absolute inset-0 size-full text-brand-strong"
    >
      {edges.map(([a, b], i) => (
        <line
          key={`e${i}`}
          x1={nodes[a][0]}
          y1={nodes[a][1]}
          x2={nodes[b][0]}
          y2={nodes[b][1]}
          stroke="currentColor"
          strokeWidth="1"
          strokeLinecap="round"
          opacity="0.12"
        />
      ))}
      {nodes.map(([x, y, r], i) => (
        <circle key={`n${i}`} cx={x} cy={y} r={r} fill="currentColor" opacity="0.26" />
      ))}
    </svg>
  )
}

export default function Login({ onAuthed }: { onAuthed: () => void }) {
  const [pw, setPw] = useState('')
  const [show, setShow] = useState(false)
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)

  async function submit(e: FormEvent) {
    e.preventDefault()
    if (busy) return
    if (!pw.trim()) {
      setErr('请输入密码')
      return
    }
    setBusy(true)
    setErr('')
    try {
      const r = await api.post<{ token: string }>('/auth/login', { password: pw })
      setToken(r.token)
      onAuthed()
    } catch (ex) {
      if (ex instanceof ApiError && ex.status === 401) setErr('密码错误，请重试')
      else if (ex instanceof TypeError) setErr('无法连接服务，请稍后重试')
      else setErr(ex instanceof ApiError ? ex.message : '登录失败，请重试')
    } finally {
      setBusy(false)
    }
  }

  return (
    <main className="relative min-h-screen overflow-hidden bg-background text-foreground">
      {/* 顶部品牌辉光 */}
      <div
        aria-hidden="true"
        className="absolute inset-x-0 top-0 h-[34rem]"
        style={{
          background:
            'radial-gradient(60% 55% at 50% 0%, oklch(0.55 0.2 264 / 0.16), transparent 70%)',
        }}
      />
      <Constellation />

      <div className="relative z-10 flex min-h-screen items-center justify-center px-6 py-16">
        <form onSubmit={submit} className="login-enter w-full max-w-sm">
          <div className="flex flex-col items-center text-center">
            <BrandMark className="size-11" />
            <h1 className="mt-5 text-2xl font-semibold tracking-tight">
              agent-memory<span className="text-brand-strong">.</span>
            </h1>
            <p className="mt-2 text-sm text-muted-foreground">AI 长期记忆平台</p>
          </div>

          <div className="mt-10 space-y-4">
            <div>
              <label htmlFor="admin-password" className="mb-2 block text-sm font-medium">
                管理员密码
              </label>
              <div className="relative">
                <KeyRound className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
                <input
                  id="admin-password"
                  type={show ? 'text' : 'password'}
                  value={pw}
                  onChange={(e) => setPw(e.target.value)}
                  autoFocus
                  autoComplete="current-password"
                  placeholder="输入密码"
                  className="h-11 w-full rounded-lg border border-input bg-card pl-10 pr-11 text-sm outline-none transition-colors placeholder:text-muted-foreground/70 focus-visible:border-brand/60 focus-visible:ring-2 focus-visible:ring-brand/30"
                />
                <button
                  type="button"
                  onClick={() => setShow((s) => !s)}
                  aria-label={show ? '隐藏密码' : '显示密码'}
                  className="absolute right-2 top-1/2 -translate-y-1/2 rounded-md p-1.5 text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-brand/40"
                >
                  {show ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
                </button>
              </div>
            </div>

            {err && (
              <p role="alert" className="text-sm text-destructive">
                {err}
              </p>
            )}

            <Button type="submit" disabled={busy} className="h-11 w-full">
              {busy && <Loader2 className="animate-spin" />}
              {busy ? '登录中…' : '登录'}
            </Button>
          </div>

          <p className="mt-8 text-center text-xs tracking-wide text-muted-foreground/70">
            Memory · Knowledge · Wiki · CodeGraph
          </p>
        </form>
      </div>
    </main>
  )
}

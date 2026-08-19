/** 共享 UI 小件：状态徽章 / 空态 / 错误态 / 时间格式化。 */

export function StatusBadge({ status }: { status: string }) {
  const tone: Record<string, string> = {
    ready: 'bg-green-500/15 text-green-400',
    succeeded: 'bg-green-500/15 text-green-400',
    active: 'bg-green-500/15 text-green-400',
    pending: 'bg-yellow-500/15 text-yellow-400',
    processing: 'bg-blue-500/15 text-blue-400',
    parsing: 'bg-blue-500/15 text-blue-400',
    chunking: 'bg-blue-500/15 text-blue-400',
    embedding: 'bg-blue-500/15 text-blue-400',
    running: 'bg-blue-500/15 text-blue-400',
    indexing: 'bg-blue-500/15 text-blue-400',
    failed: 'bg-red-500/15 text-red-400',
    dead: 'bg-red-500/15 text-red-400',
    error: 'bg-red-500/15 text-red-400',
    superseded: 'bg-gray-500/15 text-gray-400',
    archived: 'bg-gray-500/15 text-gray-400',
    candidate: 'bg-purple-500/15 text-purple-400',
    version_mismatch: 'bg-orange-500/15 text-orange-400',
  }
  return (
    <span className={`rounded px-1.5 py-0.5 text-xs ${tone[status] ?? 'bg-gray-500/15 text-gray-400'}`}>
      {status}
    </span>
  )
}

export function Empty({ text }: { text: string }) {
  return <p className="rounded-lg border border-dashed p-8 text-center text-sm text-muted-foreground">{text}</p>
}

export function ErrorBox({ msg }: { msg: string }) {
  return <p className="rounded-lg border border-red-500/30 bg-red-500/10 p-4 text-sm text-red-400">{msg}</p>
}

export function fmtTime(iso: string): string {
  return new Date(iso).toLocaleString('zh-CN', { hour12: false })
}

export function Spinner({ label = '加载中…' }: { label?: string }) {
  return <p className="p-8 text-center text-sm text-muted-foreground">{label}</p>
}

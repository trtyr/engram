/**
 * 代码图谱 · **导入页**（2026-09-21 与查看页拆开）：只放「怎么把代码图谱弄进来」——
 * ① git 仓库地址（服务端 clone + 自动建索引）② 上传本机产出的 `codegraph.db`（multipart）。
 * 两个入口的产物模型见《代码图谱入口收敛 · README》；导入成功给「去查看」直达链接（带上新条目 id）。
 */
import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { ApiError, api, getToken, type CgCliStatus, type CgProject } from '@/lib/api'
import { Card, PageHeader } from '@/components/ui-bits'
import { inputCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import CgTabs from '@/components/CgTabs'

/** 注册响应（POST /codegraph/projects）。 */
interface RegisterResp {
  project: CgProject
  index_job_id: string | null
  warning: string | null
}
/** 上传响应（POST /codegraph/artifacts，multipart）。 */
interface UploadResp {
  project: CgProject
  db_bytes: number
  hint: string
}

/** 从 git 地址推断仓库名（去 .git / query / 尾斜杠）：默认落盘目录与默认项目名都用它。 */
function repoNameFromUri(uri: string): string {
  const t = uri.trim().split(/[?#]/)[0].replace(/\/+$/, '').replace(/\.git$/, '')
  return t.split(/[/:]/).pop() ?? ''
}

export default function CodeGraphImport() {
  const [cli, setCli] = useState<CgCliStatus | null>(null)
  const [importedId, setImportedId] = useState<string | null>(null)

  useEffect(() => {
    api
      .get<CgCliStatus>('/codegraph/status')
      .then(setCli)
      .catch(() => setCli({ available: false, version: null, pin: '1.5.0', hint: null }))
  }, [])

  return (
    <div className="space-y-4">
      <PageHeader
        title="代码图谱 · 导入"
        desc="两个入口：① git 仓库地址（服务端 clone 后自动建索引）② 上传本机 codegraph index 产出的 codegraph.db"
      >
        <CgTabs />
      </PageHeader>

      <CliBar cli={cli} />

      <div className="grid gap-3 lg:grid-cols-2">
        <GitEntryCard onImported={setImportedId} />
        <UploadEntryCard onImported={setImportedId} />
      </div>

      {importedId && (
        <Card className="flex flex-wrap items-center justify-between gap-2 px-3 py-2">
          <p className="text-xs text-muted-foreground">导入完成——去「查看」页看状态与调用图。</p>
          <Link
            to={`/codegraph?sel=${importedId}`}
            className="text-xs text-info underline underline-offset-2"
          >
            去查看 →
          </Link>
        </Card>
      )}
    </div>
  )
}

/** CLI 状态条（单行紧凑）：服务端建索引/同步都依赖它。 */
function CliBar({ cli }: { cli: CgCliStatus | null }) {
  if (!cli) return null
  return (
    <Card
      className={`flex flex-wrap items-center gap-3 px-3 py-2 ${!cli.available ? 'border-destructive/40' : ''}`}
    >
      <span
        aria-hidden="true"
        className={`size-2 rounded-full ${cli.available ? 'bg-success' : 'bg-destructive'}`}
      />
      <p className="text-xs">
        {cli.available
          ? `codegraph CLI 已就绪 · v${cli.version}${cli.hint ? '（与 pin 不符）' : '（pin 匹配）'}`
          : 'codegraph CLI 不可用'}
      </p>
      {cli.available && (
        <span className="font-mono text-[11px] text-muted-foreground">pin {cli.pin}</span>
      )}
      {/* R5（task-5）：装 + 锁版指引由服务端下发，前端不再硬编码命令 */}
      {cli.hint && (
        <p className="basis-full font-mono text-[11px] leading-relaxed text-muted-foreground">
          {cli.hint}
        </p>
      )}
    </Card>
  )
}

/** 入口①：git 仓库地址 → 注册（服务端 clone）+ 自动建索引。 */
function GitEntryCard({ onImported }: { onImported: (id: string) => void }) {
  const [uri, setUri] = useState('')
  const [name, setName] = useState('')
  const [destParent, setDestParent] = useState('')
  const [busy, setBusy] = useState(false)
  const [ok, setOk] = useState('')
  const [err, setErr] = useState('')

  const repoName = repoNameFromUri(uri)
  const useName = name.trim() || repoName
  const defaultDest = `<数据根>/codegraph/${useName || '项目名'}`

  async function submit() {
    if (!uri.trim()) {
      setErr('请填 git 仓库地址（如 https://github.com/you/repo）')
      return
    }
    if (!useName) {
      setErr('无法从地址推断项目名，请手动填写项目名')
      return
    }
    setBusy(true)
    setErr('')
    setOk('')
    try {
      const r = await api.post<RegisterResp>('/codegraph/projects', {
        name: useName,
        source_uri: uri.trim(),
        ...(destParent.trim() ? { dest_parent: destParent.trim() } : {}),
      })
      setUri('')
      setName('')
      setDestParent('')
      setOk(
        r.warning
          ? `已 clone 到 ${r.project.path}，但自动建索引入队失败：${r.warning}`
          : `已 clone 到 ${r.project.path}，已入队建索引（去「查看」看进度）`,
      )
      onImported(r.project.id)
    } catch (e) {
      setErr(e instanceof Error ? e.message : '注册失败')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card className="space-y-2 p-3">
      <div className="flex items-center justify-between gap-2">
        <h3
          className="text-sm font-medium"
          title="服务端 git clone --depth 1 到指定位置，随后自动建索引（无需再点「建索引」）"
        >
          ① git 仓库接入
        </h3>
        <Button size="sm" variant="outline" disabled={busy} onClick={submit}>
          {busy ? 'clone 中…' : '注册并建索引'}
        </Button>
      </div>
      <input
        className={`${inputCls} w-full`}
        placeholder="git 仓库地址，如 https://github.com/you/repo.git"
        aria-label="git 仓库地址"
        value={uri}
        onChange={(e) => setUri(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && !busy) submit()
        }}
      />
      <div className="flex flex-wrap gap-2">
        <input
          className={`${inputCls} w-32`}
          placeholder={repoName ? `项目名（默认 ${repoName}）` : '项目名（可选）'}
          aria-label="项目名"
          title="留空则用仓库名当项目名；它同时是默认落盘目录名"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <input
          className={`${inputCls} min-w-0 flex-1`}
          placeholder="落盘父目录（可选）"
          aria-label="落盘父目录"
          title={`留空 → ${defaultDest}（重名自动加 -2）；填了 → <父目录>/${repoName || '<仓库名>'}。须在服务端白名单根内（默认只允许数据根）。删除条目：默认落盘连目录清、自定义落盘只删注册与产物。`}
          value={destParent}
          onChange={(e) => setDestParent(e.target.value)}
        />
      </div>
      <p className="truncate text-[11px] text-muted-foreground" title={`落盘默认 ${defaultDest}`}>
        落盘：<span className="font-mono">{defaultDest}</span>
        <span className="ml-1">（留空即默认，详见输入框悬停说明）</span>
      </p>
      {ok && <p className="text-xs text-success">{ok}</p>}
      {err && <p className="text-xs text-destructive">{err}</p>}
    </Card>
  )
}

/** 入口②：上传本机产出的 codegraph.db（multipart；head 可留空 = 未声明）。 */
function UploadEntryCard({ onImported }: { onImported: (id: string) => void }) {
  const [file, setFile] = useState<File | null>(null)
  const [name, setName] = useState('')
  const [head, setHead] = useState('')
  const [busy, setBusy] = useState(false)
  const [ok, setOk] = useState('')
  const [err, setErr] = useState('')

  const fileDefault = file ? file.name.replace(/\.db$/i, '') : ''

  async function submit() {
    if (!file) {
      setErr('请选择 codegraph.db 文件（本机 `codegraph index` 的产物）')
      return
    }
    const useName = name.trim() || fileDefault
    if (!useName) {
      setErr('请填写项目名（同名再传 = 覆盖该条目的产物）')
      return
    }
    setBusy(true)
    setErr('')
    setOk('')
    try {
      const r = await uploadArtifact(useName, file, head.trim())
      setOk(
        `已入库「${r.project.name}」：${(r.db_bytes / 1024 / 1024).toFixed(1)}MB，状态 ${r.project.status}` +
          (r.project.head ? `，head ${r.project.head.slice(0, 10)}` : '（未声明 head）'),
      )
      setFile(null)
      setName('')
      setHead('')
      onImported(r.project.id)
    } catch (e) {
      setErr(e instanceof Error ? e.message : '上传失败')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card className="space-y-2 p-3">
      <div className="flex items-center justify-between gap-2">
        <h3
          className="text-sm font-medium"
          title="本机跑 codegraph index 后，把 .codegraph/codegraph.db 传上来（原始二进制，≤256MB）。同名再传即覆盖该条目产物，旧产物元数据留痕。"
        >
          ② 上传 codegraph.db
        </h3>
        <Button size="sm" variant="outline" disabled={busy} onClick={submit}>
          {busy ? '上传中…' : '上传产物'}
        </Button>
      </div>
      <input
        type="file"
        accept=".db,application/octet-stream"
        aria-label="选择 codegraph.db"
        className="w-full text-xs text-muted-foreground file:mr-3 file:rounded-md file:border file:border-border file:bg-transparent file:px-2.5 file:py-1 file:text-xs"
        onChange={(e) => setFile(e.target.files?.[0] ?? null)}
      />
      <div className="flex flex-wrap gap-2">
        <input
          className={`${inputCls} w-32`}
          placeholder={fileDefault ? `项目名（默认 ${fileDefault}）` : '项目名'}
          aria-label="项目名"
          title="同名再传即覆盖该条目的产物（旧产物元数据压进 stats.previous）"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <input
          className={`${inputCls} min-w-0 flex-1`}
          placeholder="head（可选）"
          aria-label="head"
          title="声明式新鲜度：本机 `git rev-parse HEAD` 的结果。留空 = 未声明——服务端不编造 head，界面会如实标注「无法比对」。"
          value={head}
          onChange={(e) => setHead(e.target.value)}
        />
      </div>
      <p className="truncate text-[11px] text-muted-foreground">
        传的是 <span className="font-mono">.codegraph/codegraph.db</span> 本体（不要压缩/文本化）
      </p>
      {ok && <p className="text-xs text-success">{ok}</p>}
      {err && <p className="text-xs text-destructive">{err}</p>}
    </Card>
  )
}

/** multipart 上传（绕开 JSON 封装）：错误体与 api 同口径解析，失败文案可行动。 */
async function uploadArtifact(name: string, file: File, head: string): Promise<UploadResp> {
  const fd = new FormData()
  fd.append('name', name)
  if (head) fd.append('head', head)
  fd.append('file', file, 'codegraph.db')
  const t = getToken()
  const resp = await fetch('/codegraph/artifacts', {
    method: 'POST',
    headers: t ? { authorization: `Bearer ${t}` } : {},
    body: fd,
  })
  if (!resp.ok) {
    let msg = `HTTP ${resp.status}`
    let code = 'unknown'
    let retryable = false
    try {
      const e = await resp.json()
      if (e?.error) {
        msg = e.error.message ?? msg
        code = e.error.code ?? code
        retryable = e.error.retryable ?? false
      }
    } catch {
      /* 非 JSON 错误体（如 413）：保底用状态码 */
    }
    throw new ApiError(resp.status, code, msg, retryable)
  }
  return (await resp.json()) as UploadResp
}

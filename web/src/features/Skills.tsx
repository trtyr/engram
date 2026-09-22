/**
 * 技能域：Wiki 式双栏浏览——左侧技能目录（独立滚动）+ 右侧 Markdown 阅读与治理。
 * 顶部菜单栏：搜索 / 状态筛选 / 导出 / 导入 / 新建。
 * folder 形态：skill = 文件夹（SKILL.md 本体 + scripts/ references/ 等附属文件），
 * 云部署语义——文件按路径寻址随库走，客户端取走后本地执行。
 */
import { useCallback, useEffect, useState } from 'react'
import { appConfirm } from '@/components/confirm'
import {
  api,
  type SkillFileInfoDto,
  type SkillImportReport,
  type SkillRevisionDto,
  type SkillSummaryDto,
} from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner } from '@/components/ui-bits'
import { inputCls, selectCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import WikiMarkdown from '@/components/WikiMarkdown'
import { cn } from '@/lib/utils'

const SOURCE_LABEL: Record<string, string> = {
  manual: '手建',
  import: '导入',
  mcp: 'MCP',
}

/** 存储形态标签：text=整体入库 / script=本地指针。 */
const KIND_LABEL: Record<string, string> = {
  text: '文本',
  script: '脚本',
}

/** 来源标签。 */
const ORIGIN_LABEL: Record<string, string> = {
  self: '自建',
  github: 'GitHub',
  both: '自建+GitHub',
}

type PaneMode = 'read' | 'edit' | 'revs'

/** 完整详情（含正文；script 型正文由后端从 local_path 现读）。 */
interface SkillDetail {
  slug: string
  name: string
  description: string
  content: string
  tags: string[]
  enabled: boolean
  source: string
  kind: string
  origin: string
  local_path: string | null
  repo_url: string | null
}

const isScript = (d: { kind: string } | null) => d?.kind === 'script'

export default function Skills() {
  const [rows, setRows] = useState<SkillSummaryDto[] | null>(null)
  const [q, setQ] = useState('')
  const [enabledFilter, setEnabledFilter] = useState('')
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)
  const [panel, setPanel] = useState<null | 'create' | 'import'>(null)
  const [importReport, setImportReport] = useState<SkillImportReport | null>(null)

  // 双栏：左侧目录选中项 → 右侧阅读
  const [selected, setSelected] = useState<string | null>(null)
  const [detail, setDetail] = useState<SkillDetail | null>(null)
  const [files, setFiles] = useState<SkillFileInfoDto[] | null>(null)
  const [mode, setMode] = useState<PaneMode>('read')
  const [viewFile, setViewFile] = useState<string | null>(null)
  const [fileContent, setFileContent] = useState<string | null>(null)
  const [detailErr, setDetailErr] = useState('')

  // 新建表单
  const [name, setName] = useState('')
  const [slug, setSlug] = useState('')
  const [description, setDescription] = useState('')
  const [tags, setTags] = useState('')
  const [content, setContent] = useState('')
  // 二态（0038）：text=入库 / script=本地指针
  const [kind, setKind] = useState('text')
  const [localPath, setLocalPath] = useState('')
  const [origin, setOrigin] = useState('self')
  const [repoUrl, setRepoUrl] = useState('')

  // 导入表单
  const [importText, setImportText] = useState('')
  const [overwrite, setOverwrite] = useState(false)

  // 编辑表单
  const [editName, setEditName] = useState('')
  const [editDescription, setEditDescription] = useState('')
  const [editTags, setEditTags] = useState('')
  const [editContent, setEditContent] = useState('')

  // 版本列表（按 slug 缓存）
  const [revs, setRevs] = useState<Record<string, SkillRevisionDto[]>>({})

  // 附属文件上传表单
  const [filePath, setFilePath] = useState('')
  const [fileText, setFileText] = useState('')
  const [fileForm, setFileForm] = useState(false)

  const query = [
    q.trim() && `q=${encodeURIComponent(q.trim())}`,
    enabledFilter && `enabled=${enabledFilter}`,
  ]
    .filter(Boolean)
    .join('&')

  const load = useCallback(
    (keepSelection = true) =>
      api
        .get<SkillSummaryDto[]>(`/skills${query ? `?${query}` : ''}`)
        .then((r) => {
          setRows(r)
          setErr('')
          // 选中项被删/被过滤 → 清空；空列表 → 清空
          setSelected((cur) => {
            if (!keepSelection) return cur
            if (cur && r.some((s) => s.slug === cur)) return cur
            return r[0]?.slug ?? null
          })
        })
        .catch((e) => setErr(e.message)),
    [query],
  )

  useEffect(() => {
    load()
  }, [load])

  // 选中变化 → 拉详情；text 型顺带拉附属文件索引（script 型无入库文件）
  useEffect(() => {
    if (!selected) {
      setDetail(null)
      setFiles(null)
      return
    }
    let alive = true
    setDetailErr('')
    api
      .get<SkillDetail>(`/skills/${selected}`)
      .then((s) => {
        if (!alive) return
        setDetail(s)
        if (s.kind === 'script') {
          setFiles([])
          return Promise.resolve()
        }
        return api
          .get<SkillFileInfoDto[]>(`/skills/${selected}/files`)
          .then((fs) => {
            if (alive) setFiles(fs)
          })
      })
      .catch((e) => {
        if (!alive) return
        setDetailErr(e instanceof Error ? e.message : '详情加载失败')
      })
    return () => {
      alive = false
    }
  }, [selected])

  function startEdit(slug: string) {
    if (!detail || detail.slug !== slug) return
    setEditName(detail.name)
    setEditDescription(detail.description)
    setEditTags(detail.tags.join(', '))
    setEditContent(detail.content)
    setMode('edit')
  }

  async function doEdit(slug: string) {
    if (!editName.trim()) return
    setBusy(true)
    try {
      const body: Record<string, unknown> = {
        name: editName.trim(),
        description: editDescription.trim(),
        tags: editTags.split(/[,，]/).map((s) => s.trim()).filter(Boolean),
      }
      // script 型正文不入库（SKILL.md 在本地）——不发 content
      if (!isScript(detail)) body.content = editContent
      await api.put(`/skills/${slug}`, body)
      setMode('read')
      setErr('')
      await load()
      // 重拉详情
      const s = await api.get<SkillDetail>(`/skills/${slug}`)
      setDetail(s)
    } catch (e) {
      setErr(e instanceof Error ? e.message : '保存失败')
    } finally {
      setBusy(false)
    }
  }

  async function doToggle(slug: string, enabled: boolean) {
    setBusy(true)
    try {
      await api.put(`/skills/${slug}`, { enabled })
      setErr('')
      await load()
      if (detail?.slug === slug) setDetail({ ...detail, enabled })
    } catch (e) {
      setErr(e instanceof Error ? e.message : '操作失败')
    } finally {
      setBusy(false)
    }
  }

  async function doDelete(slug: string) {
    if (
      !(await appConfirm({
        title: `删除技能「${slug}」？`,
        description: '附属文件与版本快照一并删除，不可恢复。',
        destructive: true,
        confirmLabel: '删除',
      }))
    )
      return
    setBusy(true)
    try {
      await api.del(`/skills/${slug}`)
      setSelected(null)
      setDetail(null)
      setMode('read')
      setErr('')
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '删除失败')
    } finally {
      setBusy(false)
    }
  }

  async function doCreate() {
    if (!name.trim()) return
    setBusy(true)
    try {
      const created = await api.post<{ slug: string }>('/skills', {
        name: name.trim(),
        slug: slug.trim() || null,
        description: description.trim(),
        content: kind === 'script' ? '' : content,
        tags: tags.split(/[,，]/).map((s) => s.trim()).filter(Boolean),
        kind,
        origin,
        local_path: kind === 'script' ? localPath.trim() || null : null,
        repo_url: origin === 'self' ? null : repoUrl.trim() || null,
      })
      setName('')
      setSlug('')
      setDescription('')
      setTags('')
      setContent('')
      setKind('text')
      setLocalPath('')
      setOrigin('self')
      setRepoUrl('')
      setPanel(null)
      setErr('')
      await load(false)
      if (created?.slug) setSelected(created.slug)
      setMode('read')
    } catch (e) {
      setErr(e instanceof Error ? e.message : '新建失败')
    } finally {
      setBusy(false)
    }
  }

  async function doImport() {
    if (!importText.trim()) return
    setBusy(true)
    try {
      const report = await api.post<SkillImportReport>('/skills/import', {
        documents: [{ content: importText }],
        overwrite,
      })
      setImportReport(report)
      setImportText('')
      setErr('')
      await load(false)
      if (report.items.length > 0) {
        const ok = report.items.find((i) => i.slug)
        if (ok?.slug) setSelected(ok.slug)
      }
    } catch (e) {
      setErr(e instanceof Error ? e.message : '导入失败')
    } finally {
      setBusy(false)
    }
  }

  async function doExport() {
    setBusy(true)
    try {
      const all = await api.get<unknown[]>('/skills/export')
      const blob = new Blob([JSON.stringify(all, null, 2)], { type: 'application/json' })
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      a.download = 'engram-skills-export.json'
      a.click()
      URL.revokeObjectURL(url)
    } catch (e) {
      setErr(e instanceof Error ? e.message : '导出失败')
    } finally {
      setBusy(false)
    }
  }

  async function showRevs(slug: string) {
    setMode('revs')
    try {
      const list = await api.get<SkillRevisionDto[]>(`/skills/${slug}/revisions`)
      setRevs((prev) => ({ ...prev, [slug]: list }))
    } catch (e) {
      setErr(e instanceof Error ? e.message : '版本加载失败')
    }
  }

  async function doRestore(slug: string, revId: string) {
    setBusy(true)
    try {
      await api.post(`/skills/${slug}/revisions/${revId}/restore`, {})
      setErr('')
      await load()
      const s = await api.get<SkillDetail>(`/skills/${slug}`)
      setDetail(s)
      showRevs(slug)
    } catch (e) {
      setErr(e instanceof Error ? e.message : '回滚失败')
    } finally {
      setBusy(false)
    }
  }

  // ---------- 附属文件操作 ----------

  async function reloadFiles(slug: string) {
    try {
      setFiles(await api.get<SkillFileInfoDto[]>(`/skills/${slug}/files`))
    } catch (e) {
      setErr(e instanceof Error ? e.message : '文件索引加载失败')
    }
  }

  async function saveBlob(blob: Blob, filename: string) {
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = filename
    a.click()
    URL.revokeObjectURL(url)
  }

  /** 单文件直下（raw）。 */
  async function downloadFile(slug: string, path: string) {
    setBusy(true)
    try {
      const blob = await api.download(`/skills/${slug}/file?path=${encodeURIComponent(path)}&raw=1`)
      await saveBlob(blob, path.split('/').pop() ?? 'file')
      setErr('')
    } catch (e) {
      setErr(e instanceof Error ? e.message : '下载失败')
    } finally {
      setBusy(false)
    }
  }

  async function openFile(slug: string, path: string) {
    try {
      const f = await api.get<SkillFileEntryLike>(`/skills/${slug}/file?path=${encodeURIComponent(path)}`)
      setFileContent(f.content)
      setViewFile(path)
    } catch (e) {
      setErr(e instanceof Error ? e.message : '文件读取失败')
    }
  }

  async function doPutFile(slug: string) {
    const p = filePath.trim()
    if (!p) return
    setBusy(true)
    try {
      await api.put(`/skills/${slug}/file`, { path: p, content: fileText })
      setFilePath('')
      setFileText('')
      setFileForm(false)
      setErr('')
      await reloadFiles(slug)
    } catch (e) {
      setErr(e instanceof Error ? e.message : '文件保存失败')
    } finally {
      setBusy(false)
    }
  }

  async function doDeleteFile(slug: string, path: string) {
    if (!(await appConfirm({ title: `删除附属文件「${path}」？`, destructive: true, confirmLabel: '删除' })))
      return
    setBusy(true)
    try {
      await api.del(`/skills/${slug}/file?path=${encodeURIComponent(path)}`)
      setErr('')
      if (viewFile === path) {
        setViewFile(null)
        setFileContent(null)
      }
      await reloadFiles(slug)
    } catch (e) {
      setErr(e instanceof Error ? e.message : '文件删除失败')
    } finally {
      setBusy(false)
    }
  }

  if (err && !rows) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  const bounded = panel === null

  return (
    <div className={cn('flex flex-col gap-4', bounded && 'lg:h-[calc(100vh-3rem)]')}>
      <div className="shrink-0">
        <PageHeader title="技能" desc="技能域：文件夹形态的可复用指令包（SKILL.md + 脚本/参考资料），MCP 工具面对 AI 开放。">
          <input
            className={`${inputCls} w-48`}
            placeholder="搜名称/描述…"
            aria-label="搜索技能"
            value={q}
            onChange={(e) => setQ(e.target.value)}
          />
          <select
            className={selectCls}
            aria-label="启用状态筛选"
            value={enabledFilter}
            onChange={(e) => setEnabledFilter(e.target.value)}
          >
            <option value="">全部状态</option>
            <option value="true">已启用</option>
            <option value="false">已停用</option>
          </select>
          <Button size="sm" onClick={() => doExport()} disabled={busy}>
            导出
          </Button>
          <Button size="sm" variant="outline" onClick={() => setPanel(panel === 'import' ? null : 'import')}>
            导入
          </Button>
          <Button size="sm" variant="outline" onClick={() => setPanel(panel === 'create' ? null : 'create')}>
            新建
          </Button>
        </PageHeader>
      </div>

      {/* 新建面板 */}
      {panel === 'create' && (
        <Card className="shrink-0 space-y-2 p-4">
          <div className="flex flex-wrap gap-2">
            <input
              className={`${inputCls} w-56`}
              placeholder="技能名（必填）"
              aria-label="技能名"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
            <input
              className={`${inputCls} w-48 font-mono`}
              placeholder="slug（可选，kebab-case）"
              aria-label="技能 slug"
              value={slug}
              onChange={(e) => setSlug(e.target.value)}
            />
            <input
              className={`${inputCls} flex-1`}
              placeholder="一句话描述"
              aria-label="技能描述"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
            />
            <input
              className={`${inputCls} w-48`}
              placeholder="标签（逗号分隔）"
              aria-label="技能标签"
              value={tags}
              onChange={(e) => setTags(e.target.value)}
            />
            <select
              className={selectCls}
              aria-label="存储形态"
              title="text=整体入库；script=带脚本，真身存本地文件夹，系统只存指针"
              value={kind}
              onChange={(e) => setKind(e.target.value)}
            >
              <option value="text">文本（入库）</option>
              <option value="script">脚本（本地指针）</option>
            </select>
            <select
              className={selectCls}
              aria-label="来源"
              value={origin}
              onChange={(e) => setOrigin(e.target.value)}
            >
              <option value="self">自建</option>
              <option value="github">GitHub</option>
              <option value="both">自建+GitHub</option>
            </select>
          </div>
          {kind === 'script' && (
            <div className="flex flex-wrap gap-2">
              <input
                className={`${inputCls} flex-1 font-mono`}
                placeholder="本地技能文件夹路径（必填，含 SKILL.md，如 /opt/skills/my-tool）"
                aria-label="本地路径"
                value={localPath}
                onChange={(e) => setLocalPath(e.target.value)}
              />
              {origin !== 'self' && (
                <input
                  className={`${inputCls} w-72 font-mono`}
                  placeholder="https://github.com/user/repo（可选）"
                  aria-label="仓库地址"
                  value={repoUrl}
                  onChange={(e) => setRepoUrl(e.target.value)}
                />
              )}
            </div>
          )}
          {kind === 'script' ? (
            <p className="text-[11px] leading-4 text-muted-foreground">
              脚本型：SKILL.md 与脚本（scripts/*.py 等）放本地文件夹，系统只存指针——正文不入库、文件/版本操作走本地，get 时现读。
            </p>
          ) : (
            <textarea
              className={`${inputCls} h-40 w-full font-mono`}
              placeholder="SKILL.md 正文（markdown）——写清这个技能做什么、怎么做、何时用；纯文本参考资料创建后从右栏添加文件（不允许 .py/.sh 等脚本文件）"
              aria-label="技能正文"
              value={content}
              onChange={(e) => setContent(e.target.value)}
            />
          )}
          <div className="flex gap-2">
            <Button
              size="sm"
              disabled={busy || !name.trim() || (kind === 'script' && !localPath.trim())}
              onClick={doCreate}
            >
              创建
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setPanel(null)}>
              取消
            </Button>
          </div>
        </Card>
      )}

      {/* 导入面板 */}
      {panel === 'import' && (
        <Card className="shrink-0 space-y-2 p-4">
          <p className="text-xs text-muted-foreground">
            粘贴 SKILL.md 全文（frontmatter 容错解析：name / description / slug / tags）。批量导入走 API POST /skills/import。
          </p>
          <textarea
            className={`${inputCls} h-40 w-full font-mono`}
            placeholder={'---\nname: 示例技能\ndescription: 做什么\ntags: a, b\n---\n正文…'}
            aria-label="导入内容"
            value={importText}
            onChange={(e) => setImportText(e.target.value)}
          />
          <label className="flex items-center gap-2 text-xs text-muted-foreground">
            <input type="checkbox" checked={overwrite} onChange={(e) => setOverwrite(e.target.checked)} />
            slug 冲突时覆盖更新
          </label>
          <div className="flex gap-2">
            <Button size="sm" disabled={busy || !importText.trim()} onClick={doImport}>
              导入
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setPanel(null)}>
              取消
            </Button>
          </div>
          {importReport && (
            <div className="rounded-md border border-border p-2 text-xs">
              <p className="text-muted-foreground">
                新建 {importReport.imported} · 覆盖 {importReport.updated} · 失败 {importReport.failed}
              </p>
              {importReport.items
                .filter((i) => i.status === 'failed')
                .map((i) => (
                  <p key={i.index} className="mt-1 text-destructive">
                    #{i.index}：{i.error}
                  </p>
                ))}
            </div>
          )}
        </Card>
      )}

      {err && <ErrorBox msg={err} />}

      {/* 双栏：目录 + 阅读（有面板打开时退到自然流） */}
      <div className={cn('flex min-h-0 min-w-0 flex-1 flex-col gap-4 lg:flex-row', !bounded && 'min-h-[60vh]')}>
        {/* 左栏：技能目录（独立滚动） */}
        <Card className={cn('w-full shrink-0 overflow-hidden lg:w-72', bounded && 'min-h-0')}>
          <div className="flex items-center justify-between border-b border-border px-3 py-2">
            <h3 className="text-sm font-semibold">技能目录</h3>
            <p className="font-mono text-xs text-muted-foreground">{rows.length}</p>
          </div>
          {rows.length === 0 ? (
            <p className="px-3 py-6 text-center text-xs text-muted-foreground">
              暂无技能——粘贴 SKILL.md 导入，或用 MCP skills_create 沉淀
            </p>
          ) : (
            <nav aria-label="技能目录" className={cn('overflow-y-auto', bounded && 'max-h-[calc(100vh-12rem)]')}>
              <ul role="list">
                {rows.map((s) => (
                  <li key={s.slug}>
                    <button
                      type="button"
                      aria-current={selected === s.slug}
                      onClick={() => {
                        setSelected(s.slug)
                        setMode('read')
                        setViewFile(null)
                        setFileContent(null)
                      }}
                      className={cn(
                        'w-full border-b border-border/60 px-3 py-2 text-left transition-colors last:border-b-0 hover:bg-muted/40',
                        selected === s.slug && 'bg-muted',
                      )}
                    >
                      <p className="flex items-center gap-2">
                        <span
                          aria-hidden="true"
                          className={cn(
                            'size-1.5 shrink-0 rounded-full',
                            s.enabled ? 'bg-success' : 'bg-muted-foreground/40',
                          )}
                        />
                        <span
                          className={cn(
                            'min-w-0 flex-1 truncate text-sm',
                            !s.enabled && 'text-muted-foreground line-through',
                          )}
                        >
                          {s.name}
                        </span>
                      </p>
                      <p className="mt-0.5 flex items-center gap-1.5 pl-3.5">
                        <code className="font-mono text-[11px] text-muted-foreground">{s.slug}</code>
                        <span
                          className={cn(
                            'rounded border px-1 text-[10px] leading-3.5',
                            s.kind === 'script'
                              ? 'border-amber-500/40 bg-amber-500/10 text-amber-700 dark:text-amber-400'
                              : 'border-border text-muted-foreground',
                          )}
                        >
                          {KIND_LABEL[s.kind] ?? s.kind}
                        </span>
                        <span className="rounded border border-border px-1 text-[10px] leading-3.5 text-muted-foreground">
                          {SOURCE_LABEL[s.source] ?? s.source}
                        </span>
                      </p>
                    </button>
                  </li>
                ))}
              </ul>
            </nav>
          )}
        </Card>

        {/* 右栏：阅读 / 编辑 / 版本 */}
        <div className={cn('flex min-h-[45vh] min-w-0 flex-1 flex-col lg:min-h-0', !bounded && 'min-h-[60vh]')}>
          {!selected || !detail ? (
            <Card className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3">
              {detailErr ? (
                <ErrorBox msg={detailErr} />
              ) : selected && !detail ? (
                <Spinner label="加载详情…" />
              ) : (
                <Empty text="从左侧目录选一个技能开始阅读；SKILL.md 里引用的脚本/参考资料在右下「附属文件」区" />
              )}
            </Card>
          ) : (
            <Card className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
              {/* 阅读头：名称 + 元信息 + 治理动作 */}
              <div className="shrink-0 border-b border-border px-4 py-2.5 md:px-6">
                <div className="flex flex-wrap items-start justify-between gap-2">
                  <div className="min-w-0">
                    <h2
                      className={cn(
                        'text-base font-semibold',
                        !detail.enabled && 'text-muted-foreground line-through',
                      )}
                    >
                      {detail.name}
                    </h2>
                    <p className="mt-0.5 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                      <code className="font-mono">{detail.slug}</code>
                      <span className="rounded border border-border px-1.5 py-0.5">
                        {SOURCE_LABEL[detail.source] ?? detail.source}
                      </span>
                      <span
                        className={cn(
                          'rounded border px-1.5 py-0.5',
                          isScript(detail)
                            ? 'border-amber-500/40 bg-amber-500/10 text-amber-700 dark:text-amber-400'
                            : 'border-border',
                        )}
                        title={isScript(detail) ? '脚本存本地文件夹，系统只存指针' : '整体入库'}
                      >
                        {KIND_LABEL[detail.kind] ?? detail.kind}
                      </span>
                      <span className="rounded border border-border px-1.5 py-0.5">
                        {ORIGIN_LABEL[detail.origin] ?? detail.origin}
                      </span>
                      <span className={detail.enabled ? 'text-emerald-600' : ''}>● {detail.enabled ? '启用' : '停用'}</span>
                      {(detail.tags ?? []).map((t) => (
                        <span key={t} className="rounded border border-border px-1.5 py-0.5">
                          {t}
                        </span>
                      ))}
                    </p>
                    {isScript(detail) && (
                      <p className="mt-1 flex flex-wrap items-center gap-1 text-[11px] text-muted-foreground">
                        <span>📁 本地：</span>
                        <code className="max-w-full break-all font-mono">{detail.local_path ?? '（未设置）'}</code>
                        {detail.repo_url && (
                          <>
                            <span>·</span>
                            <a
                              className="max-w-full break-all underline decoration-dotted hover:text-foreground"
                              href={detail.repo_url}
                              target="_blank"
                              rel="noreferrer"
                            >
                              {detail.repo_url}
                            </a>
                          </>
                        )}
                      </p>
                    )}
                  </div>
                  <div className="flex shrink-0 gap-1">
                    <Button size="sm" variant="ghost" onClick={() => setMode('read')} disabled={mode === 'read'}>
                      阅读
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => startEdit(detail.slug)} disabled={mode === 'edit'}>
                      编辑
                    </Button>
                    {!isScript(detail) && (
                      <Button size="sm" variant="ghost" onClick={() => showRevs(detail.slug)} disabled={mode === 'revs'}>
                        版本
                      </Button>
                    )}
                    <Button size="sm" variant="ghost" disabled={busy} onClick={() => doToggle(detail.slug, !detail.enabled)}>
                      {detail.enabled ? '停用' : '启用'}
                    </Button>
                    <Button size="sm" variant="ghost" disabled={busy} onClick={() => doDelete(detail.slug)}>
                      删除
                    </Button>
                  </div>
                </div>
                {detail.description && <p className="mt-1 text-xs text-muted-foreground">{detail.description}</p>}
              </div>

              {/* 正文区（独立滚动） */}
              <div className="min-h-0 flex-1 overflow-y-auto p-4 [scrollbar-gutter:stable] md:p-6">
                {mode === 'read' && (
                  <div className="mx-auto w-full max-w-4xl">
                    {viewFile !== null ? (
                      <>
                        <button
                          type="button"
                          className="mb-3 text-xs text-muted-foreground hover:text-foreground"
                          onClick={() => {
                            setViewFile(null)
                            setFileContent(null)
                          }}
                        >
                          ← 返回 SKILL.md
                        </button>
                        <h3 className="mb-2 font-mono text-xs text-muted-foreground">{viewFile}</h3>
                        {fileContent === null ? (
                          <Spinner label="加载文件…" />
                        ) : viewFile.endsWith('.md') ? (
                          <WikiMarkdown content={fileContent} />
                        ) : (
                          <pre className="overflow-x-auto rounded-md border border-border bg-muted/40 p-3 font-mono text-xs leading-5">
                            {fileContent}
                          </pre>
                        )}
                      </>
                    ) : (
                      <>
                        {isScript(detail) ? (
                          /* script 型：正文不入库（后端 content 是从 local_path 现读的——
                             那是给 agent 的通道）——Web 阅读视图不渲染正文，展示本地指针卡 */
                          <div className="rounded-md border border-amber-500/40 bg-amber-500/5 p-4 text-sm leading-6">
                            <p className="font-medium text-foreground">脚本型技能（本地指针）</p>
                            <p className="mt-2 text-muted-foreground">
                              正文不入库——SKILL.md 真身在本地文件夹，由 agent 直接读取执行；
                              系统只登记指针与来源，版本由本地 git 管理。
                            </p>
                            <dl className="mt-3 space-y-1.5 font-mono text-xs">
                              <div>
                                <dt className="mr-1 inline text-muted-foreground">local_path:</dt>
                                <dd className="inline break-all text-foreground">
                                  {detail.local_path ?? '（未设置）'}
                                </dd>
                              </div>
                            </dl>
                            <p className="mt-3 text-xs text-muted-foreground">
                              修改脚本请直接编辑本地文件；路径失效时此处会报「指针失效」。
                            </p>
                          </div>
                        ) : (
                          <WikiMarkdown content={detail.content} />
                        )}

                        {!isScript(detail) && (
                          /* text 型：附属文件随库走 */
                          <div className="mt-8 border-t border-border pt-4">
                          <div className="flex items-center justify-between">
                            <h3 className="text-sm font-semibold">
                              附属文件
                              <span className="ml-2 font-mono text-xs text-muted-foreground">
                                {files?.length ?? '…'}
                              </span>
                            </h3>
                            <Button
                              size="sm"
                              variant="ghost"
                              onClick={() => setFileForm((v) => !v)}
                            >
                              {fileForm ? '收起' : '添加文件'}
                            </Button>
                          </div>
                          <p className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
                            skill = 文件夹：脚本（scripts/）、参考资料（references/）按相对路径随技能入库；
                            文件是内容不是执行体——AI 经 MCP 按路径取走后在客户端本地运行。
                          </p>
                          {fileForm && (
                            <div className="mt-2 space-y-2 rounded-md border border-border p-3">
                              <input
                                className={`${inputCls} w-72 font-mono`}
                                placeholder="相对路径，如 scripts/check.py"
                                aria-label="文件路径"
                                value={filePath}
                                onChange={(e) => setFilePath(e.target.value)}
                              />
                              <textarea
                                className={`${inputCls} h-28 w-full font-mono`}
                                placeholder="文件内容（文本）"
                                aria-label="文件内容"
                                value={fileText}
                                onChange={(e) => setFileText(e.target.value)}
                              />
                              <Button size="sm" disabled={busy || !filePath.trim()} onClick={() => doPutFile(detail.slug)}>
                                保存文件
                              </Button>
                            </div>
                          )}
                          {files === null ? (
                            <Spinner label="加载文件…" />
                          ) : files.length === 0 ? (
                            <p className="mt-2 text-xs text-muted-foreground">暂无附属文件</p>
                          ) : (
                            <ul role="list" className="mt-2 space-y-1">
                              {files.map((f) => (
                                <li
                                  key={f.path}
                                  className="flex items-center gap-3 rounded border border-border/60 px-2 py-1.5"
                                >
                                  <code className="min-w-0 flex-1 truncate font-mono text-xs">{f.path}</code>
                                  <span className="shrink-0 font-mono text-[10px] text-muted-foreground">
                                    {f.size} B
                                  </span>
                                  <Button size="sm" variant="ghost" onClick={() => openFile(detail.slug, f.path)}>
                                    查看
                                  </Button>
                                  <Button
                                    size="sm"
                                    variant="ghost"
                                    disabled={busy}
                                    onClick={() => downloadFile(detail.slug, f.path)}
                                  >
                                    下载
                                  </Button>
                                  <Button
                                    size="sm"
                                    variant="ghost"
                                    disabled={busy}
                                    onClick={() => doDeleteFile(detail.slug, f.path)}
                                  >
                                    删除文件
                                  </Button>
                                </li>
                              ))}
                            </ul>
                          )}
                          </div>
                        )}
                      </>
                    )}
                  </div>
                )}

                {mode === 'edit' && (
                  <div className="mx-auto w-full max-w-4xl space-y-2">
                    <div className="flex flex-wrap gap-2">
                      <input
                        className={`${inputCls} w-56`}
                        aria-label="编辑技能名"
                        value={editName}
                        onChange={(e) => setEditName(e.target.value)}
                      />
                      <input
                        className={`${inputCls} flex-1`}
                        placeholder="描述"
                        aria-label="编辑技能描述"
                        value={editDescription}
                        onChange={(e) => setEditDescription(e.target.value)}
                      />
                      <input
                        className={`${inputCls} w-48`}
                        placeholder="标签（逗号分隔）"
                        aria-label="编辑技能标签"
                        value={editTags}
                        onChange={(e) => setEditTags(e.target.value)}
                      />
                    </div>
                    {isScript(detail) ? (
                      <p className="rounded-md border border-amber-500/40 bg-amber-500/5 p-3 text-xs leading-5 text-muted-foreground">
                        脚本型技能正文不入库——SKILL.md 请直接编辑本地文件{' '}
                        <code className="break-all font-mono">{detail.local_path ?? '（未设置）'}</code>
                        ，此处只可改名称/描述/标签。
                      </p>
                    ) : (
                      <textarea
                        className={`${inputCls} h-64 w-full font-mono`}
                        aria-label="编辑技能正文"
                        value={editContent}
                        onChange={(e) => setEditContent(e.target.value)}
                      />
                    )}
                    <div className="flex gap-2">
                      <Button size="sm" disabled={busy} onClick={() => doEdit(detail.slug)}>
                        保存
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => setMode('read')}>
                        取消
                      </Button>
                    </div>
                  </div>
                )}

                {mode === 'revs' && (
                  <div className="mx-auto w-full max-w-4xl space-y-2">
                    {(revs[detail.slug] ?? []).length === 0 ? (
                      <p className="text-xs text-muted-foreground">加载中…</p>
                    ) : (
                      (revs[detail.slug] ?? []).map((r) => (
                        <div key={r.id} className="flex items-center gap-2 text-xs">
                          <span className="font-mono text-muted-foreground">v{r.rev}</span>
                          <span className="rounded bg-muted px-1 text-[11px] text-muted-foreground">{r.origin}</span>
                          <span className="min-w-0 flex-1 truncate text-muted-foreground">{r.description || r.name}</span>
                          <span className="font-mono text-muted-foreground/60">
                            {new Date(r.created_at).toLocaleDateString()}
                          </span>
                          <Button size="sm" variant="ghost" disabled={busy} onClick={() => doRestore(detail.slug, r.id)}>
                            回滚
                          </Button>
                        </div>
                      ))
                    )}
                  </div>
                )}
              </div>
            </Card>
          )}
        </div>
      </div>
    </div>
  )
}

/** 文件读取响应（GET /skills/{slug}/file?path=…）。 */
interface SkillFileEntryLike {
  path: string
  content: string
}

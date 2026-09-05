/** 技能域（第六域）：SKILL.md 形态 AI 技能的资产管理——列表/检索/新建/导入/导出/启停/版本回滚。 */
import { useEffect, useState } from 'react'
import ReactMarkdown from 'react-markdown'
import { api, type SkillImportReport, type SkillRevisionDto, type SkillSummaryDto } from '@/lib/api'
import { Card, Empty, ErrorBox, PageHeader, Spinner } from '@/components/ui-bits'
import { inputCls, selectCls } from '@/lib/ui'
import { Button } from '@/components/ui/button'

const SOURCE_LABEL: Record<string, string> = {
  manual: '手建',
  import: '导入',
  mcp: 'MCP',
}

type Panel = null | 'create' | 'import' | { view: string } | { edit: string } | { revs: string }

export default function Skills() {
  const [rows, setRows] = useState<SkillSummaryDto[] | null>(null)
  const [q, setQ] = useState('')
  const [enabledFilter, setEnabledFilter] = useState('')
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)
  const [panel, setPanel] = useState<Panel>(null)
  const [importReport, setImportReport] = useState<SkillImportReport | null>(null)

  // 新建表单
  const [name, setName] = useState('')
  const [slug, setSlug] = useState('')
  const [description, setDescription] = useState('')
  const [tags, setTags] = useState('')
  const [content, setContent] = useState('')

  // 导入表单
  const [importText, setImportText] = useState('')
  const [overwrite, setOverwrite] = useState(false)

  // 编辑表单（按 slug 展开）
  const [editName, setEditName] = useState('')
  const [editDescription, setEditDescription] = useState('')
  const [editTags, setEditTags] = useState('')
  const [editContent, setEditContent] = useState('')

  // 版本列表（按 slug 缓存）
  const [revs, setRevs] = useState<Record<string, SkillRevisionDto[]>>({})

  const query = [
    q.trim() && `q=${encodeURIComponent(q.trim())}`,
    enabledFilter && `enabled=${enabledFilter}`,
  ]
    .filter(Boolean)
    .join('&')

  const load = () =>
    api
      .get<SkillSummaryDto[]>(`/skills${query ? `?${query}` : ''}`)
      .then((r) => {
        setRows(r)
        setErr('')
      })
      .catch((e) => setErr(e.message))

  useEffect(() => {
    load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query])

  async function doCreate() {
    if (!name.trim()) return
    setBusy(true)
    try {
      await api.post('/skills', {
        name: name.trim(),
        slug: slug.trim() || null,
        description: description.trim(),
        content,
        tags: tags.split(/[,，]/).map((s) => s.trim()).filter(Boolean),
      })
      setName('')
      setSlug('')
      setDescription('')
      setTags('')
      setContent('')
      setPanel(null)
      setErr('')
      load()
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
      load()
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

  async function doToggle(slug: string, enabled: boolean) {
    setBusy(true)
    try {
      await api.put(`/skills/${slug}`, { enabled })
      setErr('')
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '操作失败')
    } finally {
      setBusy(false)
    }
  }

  async function doDelete(slug: string) {
    if (!confirm(`删除技能「${slug}」？版本快照一并删除，不可恢复。`)) return
    setBusy(true)
    try {
      await api.del(`/skills/${slug}`)
      setPanel(null)
      setErr('')
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '删除失败')
    } finally {
      setBusy(false)
    }
  }

  function startEdit(slug: string) {
    // 编辑需要正文——先取详情再展开
    api.get<Required<Pick<SkillSummaryDto, 'slug'>> & { name: string; description: string; content: string; tags: string[] }>(
      `/skills/${slug}`,
    ).then((s) => {
      setEditName(s.name)
      setEditDescription(s.description)
      setEditTags(s.tags.join(', '))
      setEditContent(s.content)
      setPanel({ edit: slug })
    }).catch((e) => setErr(e.message))
  }

  async function doEdit(slug: string) {
    if (!editName.trim()) return
    setBusy(true)
    try {
      await api.put(`/skills/${slug}`, {
        name: editName.trim(),
        description: editDescription.trim(),
        content: editContent,
        tags: editTags.split(/[,，]/).map((s) => s.trim()).filter(Boolean),
      })
      setPanel(null)
      setErr('')
      load()
    } catch (e) {
      setErr(e instanceof Error ? e.message : '保存失败')
    } finally {
      setBusy(false)
    }
  }

  async function showRevs(slug: string) {
    setPanel({ revs: slug })
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
      load()
      showRevs(slug)
    } catch (e) {
      setErr(e instanceof Error ? e.message : '回滚失败')
    } finally {
      setBusy(false)
    }
  }

  async function showView(slug: string) {
    setPanel({ view: slug })
  }

  if (err && !rows) return <ErrorBox msg={err} />
  if (!rows) return <Spinner />

  return (
    <div className="space-y-5">
      <PageHeader title="技能" desc="技能第六域：SKILL.md 形态的可复用指令包——可导入、可检索、可版本回滚，MCP 六工具对 AI 开放。">
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

      {/* 新建面板 */}
      {panel === 'create' && (
        <Card className="space-y-2 p-4">
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
          </div>
          <textarea
            className={`${inputCls} h-40 w-full font-mono`}
            placeholder="技能正文（markdown）——写清这个技能做什么、怎么做、何时用"
            aria-label="技能正文"
            value={content}
            onChange={(e) => setContent(e.target.value)}
          />
          <div className="flex gap-2">
            <Button size="sm" disabled={busy || !name.trim()} onClick={doCreate}>
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
        <Card className="space-y-2 p-4">
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

      {rows.length === 0 ? (
        <Empty text="暂无技能——粘贴 SKILL.md 导入，或用 MCP tools/call skills_create 沉淀" />
      ) : (
        <div className="grid gap-3 md:grid-cols-2">
          {rows.map((s) => (
            <Card key={s.slug} className="p-4">
              <div className="flex items-start gap-2">
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className={`truncate font-medium ${s.enabled ? '' : 'text-muted-foreground line-through'}`}>
                      {s.name}
                    </span>
                    <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground">
                      {s.slug}
                    </span>
                    <span className="shrink-0 rounded border border-border px-1.5 py-0.5 text-[11px] text-muted-foreground">
                      {SOURCE_LABEL[s.source] ?? s.source}
                    </span>
                    <span
                      className={`shrink-0 whitespace-nowrap text-xs ${s.enabled ? 'text-emerald-600' : 'text-muted-foreground'}`}
                    >
                      ● {s.enabled ? '启用' : '停用'}
                    </span>
                  </div>
                  {s.description && (
                    <p className="mt-1 line-clamp-2 text-xs text-muted-foreground">{s.description}</p>
                  )}
                  {s.tags.length > 0 && (
                    <div className="mt-2 flex flex-wrap gap-1">
                      {s.tags.map((t) => (
                        <span
                          key={t}
                          className="rounded border border-border px-1.5 py-0.5 text-[11px] text-muted-foreground"
                        >
                          {t}
                        </span>
                      ))}
                    </div>
                  )}
                </div>
                <div className="flex shrink-0 flex-col gap-1">
                  <div className="flex gap-1">
                    <Button size="sm" variant="ghost" onClick={() => (panel && typeof panel === 'object' && 'view' in panel && panel.view === s.slug ? setPanel(null) : showView(s.slug))}>
                      查看
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => startEdit(s.slug)}>
                      编辑
                    </Button>
                  </div>
                  <div className="flex gap-1">
                    <Button size="sm" variant="ghost" onClick={() => doToggle(s.slug, !s.enabled)} disabled={busy}>
                      {s.enabled ? '停用' : '启用'}
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => (panel && typeof panel === 'object' && 'revs' in panel && panel.revs === s.slug ? setPanel(null) : showRevs(s.slug))}>
                      版本
                    </Button>
                    <Button size="sm" variant="ghost" disabled={busy} onClick={() => doDelete(s.slug)}>
                      删除
                    </Button>
                  </div>
                </div>
              </div>

              {/* 正文预览 */}
              {panel && typeof panel === 'object' && 'view' in panel && panel.view === s.slug && (
                <ViewPane slug={s.slug} />
              )}

              {/* 编辑面板 */}
              {panel && typeof panel === 'object' && 'edit' in panel && panel.edit === s.slug && (
                <div className="mt-3 space-y-2 border-t border-border/60 pt-3">
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
                  <textarea
                    className={`${inputCls} h-40 w-full font-mono`}
                    aria-label="编辑技能正文"
                    value={editContent}
                    onChange={(e) => setEditContent(e.target.value)}
                  />
                  <div className="flex gap-2">
                    <Button size="sm" disabled={busy} onClick={() => doEdit(s.slug)}>
                      保存
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => setPanel(null)}>
                      取消
                    </Button>
                  </div>
                </div>
              )}

              {/* 版本面板 */}
              {panel && typeof panel === 'object' && 'revs' in panel && panel.revs === s.slug && (
                <div className="mt-3 space-y-2 border-t border-border/60 pt-3">
                  {(revs[s.slug] ?? []).length === 0 ? (
                    <p className="text-xs text-muted-foreground">加载中…</p>
                  ) : (
                    (revs[s.slug] ?? []).map((r) => (
                      <div key={r.id} className="flex items-center gap-2 text-xs">
                        <span className="font-mono text-muted-foreground">v{r.rev}</span>
                        <span className="rounded bg-muted px-1 text-[11px] text-muted-foreground">{r.origin}</span>
                        <span className="min-w-0 flex-1 truncate text-muted-foreground">{r.description || r.name}</span>
                        <span className="font-mono text-muted-foreground/60">
                          {new Date(r.created_at).toLocaleDateString()}
                        </span>
                        <Button size="sm" variant="ghost" disabled={busy} onClick={() => doRestore(s.slug, r.id)}>
                          回滚
                        </Button>
                      </div>
                    ))
                  )}
                </div>
              )}
            </Card>
          ))}
        </div>
      )}
    </div>
  )
}

/** 正文预览（react-markdown，与项目详情同一排版约定）。 */
function ViewPane({ slug }: { slug: string }) {
  const [content, setContent] = useState<string | null>(null)
  const [err, setErr] = useState('')
  useEffect(() => {
    api
      .get<{ content: string }>(`/skills/${slug}`)
      .then((s) => setContent(s.content))
      .catch((e) => setErr(e.message))
  }, [slug])
  if (err) return <ErrorBox msg={err} />
  if (content === null) return <Spinner label="加载正文…" />
  return (
    <div className="mt-3 border-t border-border/60 pt-3">
      <div className="prose prose-sm max-w-none dark:prose-invert">
        <ReactMarkdown>{content}</ReactMarkdown>
      </div>
    </div>
  )
}

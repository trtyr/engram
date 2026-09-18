/** 独立全屏文件查看页：/file-view?project=<id>&name=<name> —— 从项目页「全屏打开」以新标签页进入，
 *  绕开 Shell 侧边栏，整个视口交给内容（架构图等全屏设计的 HTML 制品专用查看窗口）。 */
import { useEffect, useState } from 'react'
import { useSearchParams } from 'react-router-dom'
import { api, type ProjectFileDto } from '@/lib/api'
import { ErrorBox, Spinner } from '@/components/ui-bits'
import { fmtTime } from '@/lib/ui'
import { Button } from '@/components/ui/button'
import WikiMarkdown from '@/components/WikiMarkdown'

export default function FileView() {
  const [params] = useSearchParams()
  const projectId = params.get('project') ?? ''
  const name = params.get('name') ?? ''
  const [file, setFile] = useState<ProjectFileDto | null>(null)
  const [err, setErr] = useState('')

  useEffect(() => {
    document.title = name ? `${name} — engram` : 'engram'
    if (!projectId || !name) {
      setErr('缺少 project 或 name 参数')
      return
    }
    api
      .get<ProjectFileDto>(`/projects/${projectId}/files/${encodeURIComponent(name)}`)
      .then(setFile)
      .catch((e) => setErr(e instanceof Error ? e.message : '加载失败'))
  }, [projectId, name])

  if (err) {
    return (
      <div className="flex min-h-screen flex-col items-start gap-3 bg-background p-6">
        <ErrorBox msg={err} />
        <Button variant="outline" size="sm" onClick={() => window.close()}>
          关闭
        </Button>
      </div>
    )
  }
  if (!file) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-background">
        <Spinner />
      </div>
    )
  }

  function toggleFullscreen() {
    if (document.fullscreenElement) {
      void document.exitFullscreen()
    } else {
      void document.documentElement.requestFullscreen()
    }
  }

  return (
    <div className="flex h-screen flex-col bg-background">
      <div className="flex h-9 shrink-0 items-center justify-between border-b border-border px-3">
        <div className="flex items-baseline gap-2 font-mono text-xs">
          <span className="font-semibold">{file.name}</span>
          <span className="text-muted-foreground">v{file.version}</span>
          <span className="text-muted-foreground">· {file.mime}</span>
          <span className="text-muted-foreground">· {fmtTime(file.updated_at)}</span>
        </div>
        <div className="flex gap-2">
          <Button variant="ghost" size="sm" onClick={toggleFullscreen}>
            浏览器全屏
          </Button>
          <Button variant="outline" size="sm" onClick={() => window.close()}>
            关闭
          </Button>
        </div>
      </div>
      <div className="min-h-0 flex-1">
        {file.mime === 'text/html' ? (
          <iframe title={file.name} sandbox="allow-scripts" srcDoc={file.content} className="h-full w-full bg-white" />
        ) : file.mime === 'text/markdown' ? (
          <div className="h-full overflow-y-auto p-4 [scrollbar-gutter:stable]">
            <WikiMarkdown content={file.content} />
          </div>
        ) : (
          <pre className="h-full overflow-auto bg-muted/40 p-4 font-mono text-xs leading-5">{file.content}</pre>
        )}
      </div>
    </div>
  )
}

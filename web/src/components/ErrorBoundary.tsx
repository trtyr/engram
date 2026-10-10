/**
 * 全局错误边界（P019-M3）：兜住渲染期异常与 lazy chunk 加载失败——
 * 旧实现无任何边界，chunk 404（部署窗口）或渲染崩溃直接整树白屏。
 * fallback 给「重新加载」出口（location.reload 拉新 chunk）。
 */
import { Component, type ReactNode } from 'react'
import { Button } from '@/components/ui/button'

type Props = { children: ReactNode }
type State = { error: Error | null }

export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null }

  static getDerivedStateFromError(error: Error): State {
    return { error }
  }

  componentDidCatch(error: Error) {
    // 浏览器端无日志上报通道（宏观 logging 审查已知盲区）——console 留痕供现场排查
    console.error('[ErrorBoundary]', error)
  }

  render() {
    if (this.state.error) {
      return (
        <div className="flex min-h-[60vh] flex-col items-center justify-center gap-3 p-8 text-center">
          <h2 className="text-base font-semibold">页面出错了</h2>
          <p className="max-w-md text-sm text-muted-foreground">
            {this.state.error.message || '渲染时发生未知异常'}
            ——可能是版本部署窗口期资源已更新。
          </p>
          <div className="flex gap-2">
            <Button size="sm" onClick={() => window.location.reload()}>
              重新加载
            </Button>
            <Button size="sm" variant="outline" onClick={() => this.setState({ error: null })}>
              重试渲染
            </Button>
          </div>
        </div>
      )
    }
    return this.props.children
  }
}

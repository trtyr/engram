/**
 * WikiMarkdown 全站渲染器测试：GFM 扩展（表格/删除线/任务列表/自动链接）
 * 与基础块级元素。回归背景：曾漏装 remark-gfm 导致表格渲染为纯文本。
 */
import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import WikiMarkdown from './WikiMarkdown'

function md(content: string) {
  return render(
    <MemoryRouter>
      <WikiMarkdown content={content} />
    </MemoryRouter>,
  )
}

describe('WikiMarkdown', () => {
  it('GFM 表格渲染为 table/th/td', () => {
    md('| Plan | 状态 |\n|---|---|\n| 后端增强 | In Progress |')
    expect(screen.getByRole('table')).toBeTruthy()
    expect(screen.getByText('Plan').tagName).toBe('TH')
    expect(screen.getByText('后端增强').tagName).toBe('TD')
  })

  it('删除线与任务列表（GFM）', () => {
    md('- [x] 已完成项\n- [ ] 未完成项\n~~废弃方案~~')
    expect(screen.getByText('废弃方案').closest('del')).not.toBeNull()
    const boxes = screen.getAllByRole('checkbox')
    expect(boxes.length).toBe(2)
    expect((boxes[0] as HTMLInputElement).checked).toBe(true)
    expect((boxes[1] as HTMLInputElement).checked).toBe(false)
  })

  it('URL 自动链接（GFM autolink literal）', () => {
    md('仓库：https://github.com/trtyr/engram')
    const link = screen.getByText('https://github.com/trtyr/engram').closest('a')
    expect(link).not.toBeNull()
    expect((link as HTMLAnchorElement).href).toContain('github.com')
  })

  it('代码块内的 [[wikilink]] 保持字面、表格内的行内 code 不被破坏', () => {
    md('| 字段 | 说明 |\n|---|---|\n| slug | 用 `plan-tree` 技能 |\n\n```\n[[not-a-link]]\n```')
    expect(screen.getByRole('table')).toBeTruthy()
    expect(screen.getByText('plan-tree').tagName).toBe('CODE')
  })

  it('基础块级元素（标题/引用/代码块）不受影响', () => {
    md('# 标题一\n\n> 引用内容\n\n```js\nconst x = 1\n```')
    expect(screen.getByRole('heading', { level: 1, name: '标题一' })).toBeTruthy()
    expect(screen.getByText('引用内容').closest('blockquote')).not.toBeNull()
  })
})

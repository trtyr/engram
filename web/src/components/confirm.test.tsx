/**
 * confirm.tsx 单测（P019-M3）：
 * 重叠语义（旧请求取消结算）/ 确认键连击只放行一次 / 未挂载立即 resolve(false)。
 * 注：陈旧闭包竞态在 jsdom 中不可复现（React 提交后 DOM 元素复用、handler 已换新）——
 * settle 绑定 Request 实例的修复以代码评审为准，此处测其可观察契约。
 */
import { describe, expect, it } from 'vitest'
import { render, screen, fireEvent, act } from '@testing-library/react'

import { appConfirm, GlobalConfirm } from '@/components/confirm'

describe('confirm 竞态与契约', () => {
  it('重叠请求：旧请求取消结算，新请求等自己的确认', async () => {
    render(<GlobalConfirm />)
    let p1Settled = false
    let p2Settled: boolean | null = null
    let p1: Promise<boolean> | undefined
    act(() => {
      p1 = appConfirm({ title: '旧弹窗' }).then((v) => {
        p1Settled = true
        return v
      })
    })
    expect(screen.getByText('旧弹窗')).toBeTruthy()

    let p2: Promise<boolean> | undefined
    act(() => {
      p2 = appConfirm({ title: '新弹窗' })
      p2!.then((v) => {
        p2Settled = v
      })
    })
    // 重叠语义：旧请求被取消结算，不挂死调用方
    await expect(p1!).resolves.toBe(false)
    expect(p1Settled).toBe(true)
    // 新弹窗已渲染，尚未结算
    expect(screen.getByText('新弹窗')).toBeTruthy()
    expect(p2Settled).toBeNull()

    // 确认键连击（双击/快速多击）：只放行一次，第二次为无操作
    await act(async () => {
      fireEvent.click(screen.getByText('确认'))
      fireEvent.click(screen.getByText('确认'))
    })
    await expect(p2).resolves.toBe(true)
    expect(p2Settled).toBe(true)
    // UI 收起
    expect(screen.queryByText('新弹窗')).toBeNull()
  })

  it('GlobalConfirm 未挂载时 appConfirm 立即 resolve(false)', async () => {
    // 不渲染 GlobalConfirm：subscriber 为 null
    await expect(appConfirm({ title: '无人值守' })).resolves.toBe(false)
  })
})

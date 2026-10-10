/**
 * Assets fields 保真单测（P019-M4）：buildFields——未触碰键回传原值（类型不变），
 * 新增/改动键用输入字符串，空键丢弃。
 */
import { describe, expect, it } from 'vitest'
import { buildFields } from '@/features/Assets'

describe('buildFields fields 保真', () => {
  it('未触碰键回传原值：数字/布尔/嵌套对象类型不变', () => {
    const orig = { port: 22, enabled: true, meta: { region: 'sh' }, note: 'hello' }
    const fields = buildFields(
      [
        ['port', '22'],
        ['enabled', 'true'],
        ['meta', '[object Object]'],
        ['note', 'hello'],
      ],
      orig,
    )
    expect(fields.port).toBe(22)
    expect(fields.enabled).toBe(true)
    expect(fields.meta).toEqual({ region: 'sh' })
    expect(fields.note).toBe('hello')
  })

  it('改动键用输入字符串；新增键保留；空键丢弃', () => {
    const orig = { port: 22 }
    const fields = buildFields(
      [
        ['port', '2222'],
        ['env', 'prod'],
        ['  ', 'ignored'],
      ],
      orig,
    )
    expect(fields.port).toBe('2222')
    expect(fields.env).toBe('prod')
    expect(fields['  ']).toBeUndefined()
  })

  it('键被删除时不回传', () => {
    const orig = { port: 22, gone: 'x' }
    const fields = buildFields([['port', '22']], orig)
    expect(fields.gone).toBeUndefined()
    expect(fields.port).toBe(22)
  })
})

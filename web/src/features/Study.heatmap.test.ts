/**
 * Study 热力图纯函数单测（P019-M3）：
 * heatmapDatesFrom——journal 失败回退 learned_at、空值过滤（旧实现 '' → dayKey 崩溃）；
 * buildHeatmap 对含空串输入的防御。
 */
import { describe, expect, it } from 'vitest'

import { buildHeatmap, dayKey, heatmapDatesFrom } from '@/features/Study'

const full = (learned: number) => ({
  progress: { learned, total: 10 },
  items: Array.from({ length: learned }, (_, i) => ({
    id: `it-${learned}-${i}`,
    learned_at: '2026-10-01T00:00:00Z',
  })),
})

describe('heatmapDatesFrom', () => {
  it('journal 成功用记录；失败回退 learned_at；空值过滤', () => {
    const js = [{ journal: [{ id: 'j1', track_id: 'tr', created_at: '2026-10-02T00:00:00Z', note: '' }] }, null]
    const fs = [full(1), full(2)] // 有 learned_at 的 fallback
    const dates = heatmapDatesFrom(js, fs as never)
    expect(dates).toContain('2026-10-02T00:00:00Z')
    expect(dates.filter((d) => d === '2026-10-01T00:00:00Z')).toHaveLength(2)
    expect(dates.every((d) => d !== '')).toBe(true)
  })

  it('learned_at 为 null 的节点被过滤，不产生空串', () => {
    const fs = [
      {
        progress: { learned: 2, total: 10 },
        items: [
          { id: 'a', learned_at: null },
          { id: 'b', learned_at: '2026-10-03T00:00:00Z' },
        ],
      },
    ]
    const dates = heatmapDatesFrom([null], fs as never)
    expect(dates).toEqual(['2026-10-03T00:00:00Z'])
  })

  it('两侧全失败时返回空数组（不崩）', () => {
    expect(heatmapDatesFrom([null, null], [null, null])).toEqual([])
  })
})

describe('buildHeatmap 防御', () => {
  it('合法日期正常聚合', () => {
    const { cells, streak } = buildHeatmap([dayKey(Date.now()), dayKey(Date.now())])
    expect(cells).toHaveLength(84)
    expect(streak).toBeGreaterThanOrEqual(1)
  })

  it('过滤后无空串进入 dayKey——旧缺陷路径不再可达', () => {
    // 直接证明空串曾被排除：heatmapDatesFrom 不会产出 ''，buildHeatmap 不会收到 ''
    const dates = heatmapDatesFrom([null], [{ progress: { learned: 1, total: 5 }, items: [{ id: 'x', learned_at: null }] }] as never)
    expect(() => buildHeatmap(dates)).not.toThrow()
  })
})

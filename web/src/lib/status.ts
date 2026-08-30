/**
 * 侧边栏系统状态：轻量轮询 /jobs 聚合出两个全局信号。
 * - failed：failed/dead 任务数（Jobs 导航项红字计数）
 * - distilling：蒸馏族任务进行中数（Memory 导航项脉冲点）
 * 页面不可见时跳过本轮（浏览器节流之外再省一档），失败静默不打扰。
 */
import { useEffect, useState } from 'react'
import { api, type Job } from '@/lib/api'

const DISTILL_KINDS = new Set([
  'extract',
  'extract_atoms',
  'arbitrate',
  'organize',
  'consolidate',
])

const POLL_MS = 10_000

export function useSystemStatus() {
  const [failed, setFailed] = useState(0)
  const [distilling, setDistilling] = useState(0)

  useEffect(() => {
    let alive = true
    const tick = async () => {
      if (document.hidden) return
      try {
        const jobs = await api.get<Job[]>('/jobs?limit=200')
        if (!alive) return
        setFailed(jobs.filter((j) => j.status === 'failed' || j.status === 'dead').length)
        setDistilling(
          jobs.filter(
            (j) =>
              DISTILL_KINDS.has(j.kind) && (j.status === 'pending' || j.status === 'running'),
          ).length,
        )
      } catch {
        /* 静默：徽章失败不构成用户打扰 */
      }
    }
    tick()
    const id = setInterval(tick, POLL_MS)
    return () => {
      alive = false
      clearInterval(id)
    }
  }, [])

  return { failed, distilling }
}

/**
 * 系统状态单例：全站共享一份 10s 轮询（侧栏徽章 / 概览 / 原子条件轮询都订同一份，
 * 网络面板只出现一个轮询流）。
 * - failed：failed/dead 任务数（任务导航项红字计数）
 * - distilling：**正在蒸馏**的会话数（processing）——脉冲只表进行中；
 *   pending 积压是存量不是进行时，归会话页灰字展示，不全局闪。
 * 页面不可见时跳过本轮，失败静默不打扰。
 */
import { useEffect, useState } from 'react'
import { api, type Job, type Session } from '@/lib/api'

const POLL_MS = 10_000

interface Status {
  failed: number
  distilling: number
}

let data: Status = { failed: 0, distilling: 0 }
let subscribers = 0
let timer: ReturnType<typeof setInterval> | null = null
const listeners = new Set<() => void>()

async function tick() {
  if (document.hidden) return
  try {
    const [jobs, sessions] = await Promise.all([
      api.get<Job[]>('/jobs?limit=200'),
      api.get<Session[]>('/memory/sessions?limit=500'),
    ])
    data = {
      failed: jobs.filter((j) => j.status === 'failed' || j.status === 'dead').length,
      distilling: sessions.filter((s) => s.distill_status === 'processing').length,
    }
    listeners.forEach((l) => l())
  } catch {
    /* 静默：徽章失败不构成用户打扰 */
  }
}

/** 订阅单例轮询：首个消费者启动，最后一个退出时停止。 */
function subscribe(notify: () => void): () => void {
  listeners.add(notify)
  if (timer === null) {
    tick()
    timer = setInterval(tick, POLL_MS)
  }
  subscribers++
  return () => {
    listeners.delete(notify)
    subscribers--
    if (subscribers === 0 && timer !== null) {
      clearInterval(timer)
      timer = null
    }
  }
}

export function useSystemStatus() {
  const [, force] = useState(0)
  useEffect(() => {
    const notify = () => force((v) => v + 1)
    return subscribe(notify)
  }, [])
  return data
}
